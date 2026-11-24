//! 剪贴板记录：在置物架查看、再次复制或固定最近复制的文字和图片。
//!
//! - 文字及图片索引保存在本机 `clipboard.json`，图片单独缓存为 PNG。
//! - 密码管理器等标记为「不进剪贴板历史」的内容一律跳过
//!   （`ExcludeClipboardContentFromMonitorProcessing` / `CanIncludeInClipboardHistory = 0`）。
//! - Windows 上用 `AddClipboardFormatListener` 等系统通知，没有轮询；
//!   监听线程只在剪贴板真正变化时醒来。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

mod picture;

/// 未固定条目的上限；固定的不占这个名额。
pub const MAX_RECENT: usize = 30;
/// 单条最长字符数，超过的不记录（大段日志、整份文件内容）。
pub const MAX_CHARS: usize = 20_000;
pub const MAX_PINNED: usize = 20;
const CACHE_LIMIT: u64 = 128 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Picture {
    key: String,
    pub width: u32,
    pub height: u32,
    bytes: u64,
}

impl Picture {
    fn valid(&self) -> bool {
        self.key.len() == 64 && self.key.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            && picture::valid_dimensions(self.width, self.height)
            && (1..=picture::MAX_PNG as u64).contains(&self.bytes)
    }
    pub fn path(&self, dir: &Path) -> PathBuf { dir.join(format!("{}.png", self.key)) }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clip {
    pub text: String,
    pub image: Option<Picture>,
    /// 本地时间 `HH:MM`。
    pub at: String,
    pub pinned: bool,
}

#[derive(Debug, Default)]
pub struct History {
    pub items: Vec<Clip>,
    /// 每次变化 +1，界面据此判断是否需要重绘。
    pub version: u64,
    path: Option<std::path::PathBuf>,
}

impl History {
    /// 新内容放最前；同样的文字只留一条（移到最前、保留固定状态）。
    pub fn push(&mut self, text: &str, at: &str) -> bool {
        if text.trim().is_empty() || text.chars().count() > MAX_CHARS { return false; }
        self.insert(Clip { text: text.into(), image: None, at: at.into(), pinned: false })
    }

    fn push_picture(&mut self, width: u32, height: u32, png: &[u8], at: &str) -> bool {
        use std::fmt::Write;
        if !picture::valid_dimensions(width, height) || png.is_empty() || png.len() > picture::MAX_PNG { return false; }
        let Some(dir) = self.image_dir() else { return false };
        let digest = ring::digest::digest(&ring::digest::SHA256, png);
        let mut key = String::with_capacity(64);
        for byte in digest.as_ref() { let _ = write!(key, "{byte:02x}"); }
        let image = Picture { key, width, height, bytes: png.len() as u64 };
        let pinned_bytes: u64 = self.items.iter().filter(|c| c.pinned).filter_map(|c| c.image.as_ref())
            .filter(|other| **other != image).map(|img| img.bytes).sum();
        if pinned_bytes + image.bytes > CACHE_LIMIT { return false; }
        let path = image.path(&dir);
        if !path.is_file() && std::fs::create_dir_all(dir).and_then(|_| crate::model::atomic_write(&path, png)).is_err() {
            return false;
        }
        self.insert(Clip { text: String::new(), image: Some(image), at: at.into(), pinned: false })
    }

    fn insert(&mut self, mut clip: Clip) -> bool {
        clip.pinned = match self.items.iter().position(|c| c.text == clip.text && c.image == clip.image) {
            Some(0) => return false,
            Some(i) => self.items.remove(i).pinned,
            None => false,
        };
        self.items.insert(0, clip);
        let mut bytes: u64 = self.items.iter().filter(|c| c.pinned).filter_map(|c| c.image.as_ref()).map(|i| i.bytes).sum();
        let mut recent = 0;
        let mut removed = Vec::new();
        self.items.retain(|c| {
            if c.pinned { return true; }
            let size = c.image.as_ref().map_or(0, |i| i.bytes);
            if recent >= MAX_RECENT || bytes + size > CACHE_LIMIT {
                if let Some(image) = &c.image { removed.push(image.clone()); }
                return false;
            }
            recent += 1;
            bytes += size;
            true
        });
        self.changed(&removed);
        true
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.items.len() {
            let removed = self.items.remove(index);
            self.changed(removed.image.as_slice());
        }
    }

    /// 清空未固定的条目，固定的保留。
    pub fn clear_recent(&mut self) {
        let removed: Vec<_> = self.items.iter().filter(|c| !c.pinned).filter_map(|c| c.image.clone()).collect();
        self.items.retain(|c| c.pinned);
        self.changed(&removed);
    }

    pub fn toggle_pin(&mut self, index: usize) -> bool {
        let pinned_count = self.items.iter().filter(|c| c.pinned).count();
        let Some(item) = self.items.get_mut(index) else { return false; };
        if !item.pinned && pinned_count >= MAX_PINNED { return false; }
        item.pinned = !item.pinned;
        self.changed(&[]);
        true
    }

    fn to_json(&self) -> Vec<u8> {
        let rows: Vec<_> = self.items.iter().map(|c| {
            let mut row = serde_json::json!({"pinned": c.pinned,"text": c.text, "at": c.at});
            if let Some(image) = &c.image { row["image"] = serde_json::json!(image); }
            row
        }).collect();
        serde_json::to_vec(&rows).unwrap_or_default()
    }

    pub fn from_json(raw: &[u8]) -> Self {
        let rows: Vec<serde_json::Value> = serde_json::from_slice(raw).unwrap_or_default();
        let items = rows.iter().filter_map(|row| {
            let image = match row.get("image") {
                Some(value) => {
                    let img: Picture = serde_json::from_value(value.clone()).ok()?;
                    if !img.valid() { return None; }
                    Some(img)
                }
                None => None,
            };
            let text = row.get("text").and_then(|v| v.as_str()).unwrap_or_default().to_string();
            if image.is_none() && text.is_empty() { return None; }
            Some(Clip { text, image,
            at: row.get("at").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            pinned: row.get("pinned").and_then(|v| v.as_bool()).unwrap_or(true),
        })}).take(MAX_PINNED + MAX_RECENT).collect();
        Self { items, ..Self::default() }
    }

    fn path() -> std::path::PathBuf { crate::model::config_dir().join("clipboard.json") }

    pub fn load() -> Self {
        Self::load_from(Self::path())
    }

    fn load_from(path: std::path::PathBuf) -> Self {
        let raw = std::fs::read(&path);
        let mut history = raw.as_ref().map(|bytes| Self::from_json(bytes)).unwrap_or_default();
        history.path = Some(path);
        if let Some(dir) = history.image_dir() {
            history.items.retain(|c| c.image.as_ref().is_none_or(|img| img.path(&dir).is_file()));
            // 清理崩溃时已写入 PNG、尚未提交索引的孤立文件。损坏或不可读的索引不清理。
            if raw.as_ref().is_ok_and(|data| serde_json::from_slice::<Vec<serde_json::Value>>(data).is_ok())
                || raw.as_ref().is_err_and(|err| err.kind() == std::io::ErrorKind::NotFound) {
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().is_some_and(|x| x == "png")
                            && !history.items.iter().filter_map(|c| c.image.as_ref()).any(|img| img.path(&dir) == path) {
                            let _ = std::fs::remove_file(path);
                        }
                    }
                }
            }
        }
        history
    }

    pub fn image_dir(&self) -> Option<PathBuf> { Some(self.path.as_ref()?.with_extension("images")) }

    fn changed(&mut self, removed: &[Picture]) {
        self.version += 1;
        if let Some(path) = &self.path {
            if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
            if crate::model::atomic_write(path, &self.to_json()).is_ok() {
                let dir = path.with_extension("images");
                for img in removed { let _ = std::fs::remove_file(img.path(&dir)); }
            }
        }
    }
}

/// 界面与监听线程共用。
#[derive(Clone)]
pub struct Shared {
    pub history: Arc<Mutex<History>>,
    pub enabled: Arc<AtomicBool>,
}

impl Shared {
    pub fn new(enabled: bool) -> Self {
        Self { history: Arc::new(Mutex::new(History::load())), enabled: Arc::new(AtomicBool::new(enabled)) }
    }
    pub fn set_enabled(&self, on: bool) { self.enabled.store(on, Ordering::Relaxed); }
    pub fn is_enabled(&self) -> bool { self.enabled.load(Ordering::Relaxed) }
    pub fn lock(&self) -> std::sync::MutexGuard<'_, History> {
        self.history.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn record_update(&self, text: &str, from_history: bool) -> bool {
        !from_history && self.record(text)
    }
    pub fn record(&self, text: &str) -> bool {
        if !self.is_enabled() { return false; }
        let at = chrono::Local::now().format("%H:%M").to_string();
        self.lock().push(text, &at)
    }
}

impl Default for Shared {
    fn default() -> Self { Self::new(true) }
}

/// 列表里的一行预览：换行与连续空白压成一个空格。
pub fn preview(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------- Windows 监听

/// 剪贴板监听的持有者；销毁时让监听线程退出并注销系统监听，不等待线程结束。
pub struct Watcher {
    #[cfg(windows)]
    stop: Arc<win::Stop>,
}

#[cfg(windows)]
impl Drop for Watcher {
    fn drop(&mut self) { self.stop.request(); }
}

#[cfg(windows)]
pub fn watch(shared: Shared, repaint: impl Fn() + Send + 'static) -> Option<Watcher> {
    let stop = Arc::new(win::Stop::default());
    let thread_stop = stop.clone();
    std::thread::Builder::new().name("daozai-clipboard".into()).spawn(move || {
        // 启动失败（极少见）时剪贴板页只是没有新记录，不影响其他功能。
        let _ = win::run(shared, Box::new(repaint), &thread_stop);
    }).ok()?;
    Some(Watcher { stop })
}

#[cfg(not(windows))]
pub fn watch(_shared: Shared, _repaint: impl Fn() + Send + 'static) -> Option<Watcher> { Some(Watcher {}) }

#[cfg(windows)]
const HISTORY_COPY_FORMAT: &str = "Daozai.ClipboardHistoryCopy";

#[cfg(windows)]
pub fn copy_history(clip: &Clip, dir: Option<&Path>) -> bool {
    use clipboard_win::{Clipboard, raw, register_format};
    let Some(format) = register_format(HISTORY_COPY_FORMAT) else { return false };
    if let Some(image) = &clip.image {
        use windows::Win32::System::Ole::CF_DIBV5;
        let Some(dir) = dir else { return false };
        let path = image.path(dir);
        if !std::fs::metadata(&path).is_ok_and(|m| m.len() == image.bytes) { return false; }
        let Ok(png) = std::fs::read(path) else { return false };
        let Some(rgba) = picture::Format::Png.decode(&png) else { return false };
        let dib = picture::dibv5(&rgba);
        let Some(png_format) = register_format("PNG") else { return false };
        let Ok(_clipboard) = Clipboard::new_attempts(10) else { return false };
        if raw::set(png_format.get(), &png).is_err() { return false; }
        let _ = raw::set_without_clear(CF_DIBV5.0 as u32, &dib);
        return raw::set_without_clear(format.get(), &[1]).is_ok();
    }
    let Ok(_clipboard) = Clipboard::new_attempts(10) else { return false };
    raw::set_string(&clip.text).is_ok() && raw::set_without_clear(format.get(), &[1]).is_ok()
}

#[cfg(windows)]
mod win {
    use super::Shared;
    use std::cell::RefCell;
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{HGLOBAL, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::DataExchange::*;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    use windows::Win32::System::Ole::{CF_UNICODETEXT, CF_DIB, CF_DIBV5};
    use windows::Win32::UI::WindowsAndMessaging::*;

    const WM_CLIPBOARDUPDATE: u32 = 0x031D;

    struct State { shared: Shared, repaint: Box<dyn Fn() + Send> }

    /// 监听线程建好消息窗口后登记线程号；持有者销毁时先置位再读线程号，两边总有一方看到对方。
    #[derive(Default)]
    pub struct Stop { requested: std::sync::atomic::AtomicBool, thread: std::sync::atomic::AtomicU32 }
    impl Stop {
        pub fn request(&self) {
            use std::sync::atomic::Ordering::SeqCst;
            self.requested.store(true, SeqCst);
            let thread = self.thread.load(SeqCst);
            if thread != 0 { unsafe { let _ = PostThreadMessageW(thread, WM_QUIT, WPARAM(0), LPARAM(0)); } }
        }
        fn register(&self) -> bool {
            use std::sync::atomic::Ordering::SeqCst;
            self.thread.store(unsafe { windows::Win32::System::Threading::GetCurrentThreadId() }, SeqCst);
            !self.requested.load(SeqCst)
        }
    }
    thread_local! { static STATE: RefCell<Option<State>> = const { RefCell::new(None) }; }

    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        if msg == WM_CLIPBOARDUPDATE {
            STATE.with(|state| {
                if let Some(state) = state.borrow().as_ref() {
                    if state.shared.is_enabled() {
                        let changed = match read(hwnd) {
                            Some(Data::Text(text)) => state.shared.record_update(&text, false),
                            Some(Data::Image(format, bytes)) => {
                                format.decode(&bytes).and_then(|rgba| {
                                    let png = super::picture::png(&rgba)?;
                                    if !state.shared.is_enabled() { return None; }
                                    let at = chrono::Local::now().format("%H:%M").to_string();
                                    Some(state.shared.lock().push_picture(rgba.width(), rgba.height(), &png, &at))
                                }).unwrap_or(false)
                            }
                            None => false,
                        };
                        if changed { (state.repaint)(); }
                    }
                }
            });
            return LRESULT(0);
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }

    /// 密码管理器、远程桌面等会给敏感内容加上这些格式，表示不要进入剪贴板历史。
    unsafe fn excluded() -> bool {
        let exclude = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
        let ignore = RegisterClipboardFormatW(w!("Clipboard Viewer Ignore"));
        if (exclude != 0 && IsClipboardFormatAvailable(exclude).is_ok()) || (ignore != 0 && IsClipboardFormatAvailable(ignore).is_ok()) {
            return true;
        }
        // 值为 DWORD 0 表示不允许进入历史。
        let history = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
        if history == 0 || IsClipboardFormatAvailable(history).is_err() { return false; }
        let Ok(handle) = GetClipboardData(history) else { return true; };
        let memory = HGLOBAL(handle.0);
        let ptr = GlobalLock(memory) as *const u32;
        if ptr.is_null() { return true; }
        let allowed = GlobalSize(memory) >= 4 && *ptr != 0;
        let _ = GlobalUnlock(memory);
        !allowed
    }

    enum Data { Text(String), Image(super::picture::Format, Vec<u8>) }

    unsafe fn read_bytes(format: u32, limit: usize) -> Option<Vec<u8>> {
        if format == 0 || IsClipboardFormatAvailable(format).is_err() { return None; }
        let memory = HGLOBAL(GetClipboardData(format).ok()?.0);
        let size = GlobalSize(memory);
        if size == 0 || size > limit { return None; }
        let ptr = GlobalLock(memory).cast::<u8>();
        if ptr.is_null() { return None; }
        let data = std::slice::from_raw_parts(ptr, size).to_vec();
        let _ = GlobalUnlock(memory);
        Some(data)
    }

    unsafe fn read(hwnd: HWND) -> Option<Data> {
        // 其他程序可能正占用剪贴板，稍等重试几次。
        let mut opened = false;
        for _ in 0..5 {
            if OpenClipboard(hwnd).is_ok() { opened = true; break; }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if !opened { return None; }
        let data = (|| {
            if excluded() || clipboard_win::register_format(super::HISTORY_COPY_FORMAT)
                .is_some_and(|f| IsClipboardFormatAvailable(f.get()).is_ok()) { return None; }
            // 同一复制操作可能提供多种格式。优先无损 PNG（保留透明度），其次为 Windows 位图。
            let png = RegisterClipboardFormatW(w!("PNG"));
            if let Some(bytes) = read_bytes(png, super::picture::MAX_PNG) {
                return Some(Data::Image(super::picture::Format::Png, bytes));
            }
            for format in [CF_DIBV5, CF_DIB] {
                if let Some(bytes) = read_bytes(format.0 as u32, super::picture::MAX_DIB) {
                    return Some(Data::Image(super::picture::Format::Dib, bytes));
                }
            }
            if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_err() { return None; }
            let handle = GetClipboardData(CF_UNICODETEXT.0 as u32).ok()?;
            let memory = HGLOBAL(handle.0);
            let ptr = GlobalLock(memory) as *const u16;
            if ptr.is_null() { return None; }
            let max = (GlobalSize(memory) / 2).min(super::MAX_CHARS * 2 + 1);
            let mut len = 0;
            while len < max && *ptr.add(len) != 0 { len += 1; }
            let text = (len <= super::MAX_CHARS * 2).then(|| String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len)));
            let _ = GlobalUnlock(memory);
            text.map(Data::Text)
        })();
        let _ = CloseClipboard();
        data
    }

    pub fn run(shared: Shared, repaint: Box<dyn Fn() + Send>, stop: &Stop) -> windows::core::Result<()> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = w!("DaozaiClipboardWatcher");
            let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: instance.into(), lpszClassName: class, ..Default::default() };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), class, PCWSTR::null(), WINDOW_STYLE(0), 0, 0, 0, 0, HWND_MESSAGE, None, instance, None)?;
            // 消息窗口建好后线程已有消息队列，此后投递的 WM_QUIT 不会丢。
            if !stop.register() { let _ = DestroyWindow(hwnd); return Ok(()); }
            STATE.with(|state| *state.borrow_mut() = Some(State { shared, repaint }));
            AddClipboardFormatListener(hwnd)?;
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = RemoveClipboardFormatListener(hwnd);
            let _ = DestroyWindow(hwnd);
            STATE.with(|state| *state.borrow_mut() = None);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn dropping_the_watcher_ends_the_listener_thread(){
        // 先停后启和启动后再停两种顺序都必须让线程退出，不能留下常驻的监听窗口。
        for stop_first in [true,false] {
            let stop=Arc::new(win::Stop::default());let thread_stop=stop.clone();
            let shared=Shared{history:Default::default(),enabled:Arc::new(AtomicBool::new(false))};
            if stop_first {stop.request();}
            let (done,finished)=std::sync::mpsc::channel();
            std::thread::spawn(move||{let _=win::run(shared,Box::new(||{}),&thread_stop);let _=done.send(());});
            if !stop_first {std::thread::sleep(std::time::Duration::from_millis(200));stop.request();}
            finished.recv_timeout(std::time::Duration::from_secs(3)).expect("监听线程应在停止后退出");
        }
    }

    #[test]
    fn history_copy_preserves_order_but_external_copy_promotes() {
        let shared = Shared { history: Default::default(), enabled: Arc::new(AtomicBool::new(true)) };
        shared.lock().items.clear();
        shared.record("较早的一条");
        shared.record("最近的一条");
        let before = shared.lock().items.clone();
        assert!(!shared.record_update("较早的一条", true));
        assert_eq!(shared.lock().items, before);
        assert!(shared.record_update("较早的一条", false));
        assert_eq!(shared.lock().items[0].text, "较早的一条");
    }
    #[test]
    fn history_dedupes_caps_and_keeps_pins() {
        let mut h = History::default();
        assert!(!h.push("   \n", "09:00"), "空白不记录");
        assert!(h.push("a", "09:00") && h.push("b", "09:01"));
        assert!(!h.push("b", "09:02"), "与最新一条相同不重复记录");
        assert!(h.push("a", "09:03"));
        assert_eq!(h.items.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(h.items[0].at, "09:03");
        h.items[1].pinned = true; // b
        for i in 0..(MAX_RECENT + 5) { h.push(&format!("n{i}"), "10:00"); }
        assert_eq!(h.items.iter().filter(|c| !c.pinned).count(), MAX_RECENT);
        assert!(h.items.iter().any(|c| c.text == "b" && c.pinned), "固定的不会被挤掉");
        h.clear_recent();
        assert_eq!(h.items.len(), 1);
        assert!(!h.push(&"x".repeat(MAX_CHARS + 1), "10:00"), "超长内容不记录");
    }

    #[test]
    fn reopening_restores_history_and_respects_manual_cleanup() {
        let path = std::env::temp_dir().join(format!("daozai-clip-{}.json", std::process::id()));
        std::fs::write(&path, br#"[{"text":"old","at":"08:00"}]"#).unwrap();
        let mut history = History::load_from(path.clone());
        assert!(history.items[0].pinned);
        history.push("保留", "09:00");
        history.push("删除", "09:01");
        history.remove(0);
        let mut reopened = History::load_from(path.clone());
        assert_eq!(reopened.items, history.items);
        reopened.clear_recent();
        let final_history = History::load_from(path.clone());
        assert_eq!(final_history.items.len(), 1);
        assert_eq!(final_history.items[0].text, "old");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn pinned_round_trip_and_preview() {
        let mut h = History::default();
        h.push("第一行\n第二行  末尾", "08:00");
        h.push("临时", "08:01");
        h.items[1].pinned = true;
        let back = History::from_json(&h.to_json());
        assert_eq!(back.items, h.items);
        assert_eq!(preview(&back.items[1].text), "第一行 第二行 末尾");
        assert!(History::from_json(b"not json").items.is_empty());
    }

    #[test]
    fn image_cache_survives_restart_and_follows_history_cleanup() {
        let path = std::env::temp_dir().join(format!("daozai-clip-image-{}.json", std::process::id()));
        std::fs::write(&path, br#"[{"text":"legacy","at":"08:00"}]"#).unwrap();
        let mut history = History::load_from(path.clone());
        let pixels = image::RgbaImage::from_pixel(3, 2, image::Rgba([21, 43, 65, 87]));
        let png = picture::png(&pixels).unwrap();
        assert!(history.push_picture(3, 2, &png, "10:00"));
        assert!(!history.push_picture(3, 2, &png, "10:01"));
        let dir = history.image_dir().unwrap();
        let cached = history.items[0].image.as_ref().unwrap().path(&dir);
        assert_eq!(std::fs::read(&cached).unwrap(), png);
        history.toggle_pin(0);
        let mut reopened = History::load_from(path.clone());
        assert_eq!(reopened.items, history.items);
        reopened.clear_recent();
        assert!(cached.exists());
        reopened.remove(0);
        assert!(!cached.exists());
        assert_eq!(History::load_from(path.clone()).items[0].text, "legacy");
        std::fs::remove_dir(dir).unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn image_index_never_resolves_an_external_path() {
        let index = br#"[{"image":{"key":"../outside","width":1,"height":1,"bytes":64}}]"#;
        assert!(History::from_json(index).items.is_empty());
    }
}
