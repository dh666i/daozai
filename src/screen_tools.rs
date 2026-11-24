use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

use crate::{
    app::Cmd,
    paint::{capsule, Card, H},
    theme::{self, W},
};
use eframe::egui::Color32;

#[derive(Clone, Copy, PartialEq)]
enum Phase { Starting, Recording, Pausing, Paused, Resuming, Stopping }

enum Event { Ready, Paused, Resumed, End(Result<(), String>) }

struct Recording {
    input: ChildStdin,
    events: Receiver<Event>,
    path: PathBuf,
    phase: Phase,
    started: Option<Instant>,
    elapsed: Duration,
}

#[derive(Default)]
pub(crate) struct ScreenTools {
    recording: Option<Recording>,
    snip: Option<Receiver<Result<String, String>>>,
    recent: Option<PathBuf>,
    error: Option<String>,
}

fn recording_dir() -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        use windows::Win32::{System::Com::CoTaskMemFree, UI::Shell::{SHGetKnownFolderPath, FOLDERID_Videos, KF_FLAG_DEFAULT}};
        let ptr = unsafe { SHGetKnownFolderPath(&FOLDERID_Videos, KF_FLAG_DEFAULT, None) }
            .map_err(|e| format!("找不到视频文件夹：{e}"))?;
        let path = unsafe { ptr.to_string() };
        unsafe { CoTaskMemFree(Some(ptr.0.cast())); }
        Ok(PathBuf::from(path.map_err(|e| e.to_string())?).join("岛仔录屏"))
    }
    #[cfg(not(windows))]
    Err("录屏需要 Windows".into())
}

impl ScreenTools {
    fn start(&mut self, settings: &crate::model::Settings) -> Result<(), String> {
        if self.recording.is_some() { return Err("正在录制".into()); }
        let dir = recording_dir()?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("无法创建视频文件夹：{e}"))?;
        let path = dir.join(format!("岛仔-{}.mp4", chrono::Local::now().format("%Y%m%d-%H%M%S-%3f")));
        let exe = std::env::current_exe().map_err(|e| e.to_string())?.with_file_name("recorder").join("DaozaiRecorder.exe");
        let mut command = Command::new(exe);
        command.arg("--output").arg(&path);
        if settings.record_system_audio { command.arg("--system-audio"); }
        if settings.record_microphone { command.arg("--microphone"); }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn().map_err(|e| format!("录屏组件启动失败：{e}"))?;
        let input = child.stdin.take().expect("piped stdin");
        let output = child.stdout.take().expect("piped stdout");
        let errors = child.stderr.take().expect("piped stderr");
        let (send, events) = mpsc::channel();
        std::thread::spawn(move || {
            let err = std::thread::spawn(move || BufReader::new(errors).lines().map_while(Result::ok).last());
            let mut finished = false;
            for line in BufReader::new(output).lines().map_while(Result::ok) {
                let event = match line.trim() {
                    "READY" => Event::Ready,
                    "PAUSED" => Event::Paused,
                    "RESUMED" => Event::Resumed,
                    "DONE" => { finished = true; continue; }
                    _ => continue,
                };
                let _ = send.send(event);
            }
            let exited = child.wait().is_ok_and(|s| s.success());
            let detail = err.join().ok().flatten();
            let result = if exited && finished { Ok(()) } else {
                Err(detail.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "录屏组件意外退出".into()))
            };
            let _ = send.send(Event::End(result));
        });
        self.recording = Some(Recording { input, events, path, phase: Phase::Starting, started: None, elapsed: Duration::ZERO });
        Ok(())
    }

    fn control(&mut self, command: &str, next: Phase) -> Result<(), String> {
        if let Some(rec) = &mut self.recording {
            writeln!(rec.input, "{command}").and_then(|_| rec.input.flush()).map_err(|e| e.to_string())?;
            rec.phase = next;
        }
        Ok(())
    }

    pub(crate) fn poll(&mut self) -> Option<String> {
        if let Some(error) = self.error.take() { return Some(error); }
        if let Some(ch) = &self.snip {
            match ch.try_recv() {
                Ok(result) => { self.snip = None; return Some(result.unwrap_or_else(|s| s)); }
                Err(mpsc::TryRecvError::Disconnected) => { self.snip = None; return Some("截图工具已退出".into()); }
                Err(mpsc::TryRecvError::Empty) => (),
            }
        }
        let rec = self.recording.as_mut()?;
        while let Ok(event) = rec.events.try_recv() {
            match event {
                Event::Ready | Event::Resumed => { rec.phase = Phase::Recording; rec.started = Some(Instant::now()); }
                Event::Paused => {
                    if let Some(t) = rec.started.take() { rec.elapsed += t.elapsed(); }
                    rec.phase = Phase::Paused;
                }
                Event::End(result) => {
                    let path = rec.path.clone();
                    self.recording = None;
                    if let Err(e) = result { return Some(format!("录屏失败：{e}")); }
                    if !path.metadata().is_ok_and(|m| m.len() > 0) { return Some("没有生成录屏文件".into()); }
                    self.recent = Some(path);
                    return Some("已保存到「视频 / 岛仔录屏」".into());
                }
            }
        }
        None
    }

    pub(crate) fn active(&self) -> bool { self.recording.is_some() }

    pub(crate) fn timer_label(&self) -> Option<String> {
        let rec = self.recording.as_ref()?;
        let seconds = (rec.elapsed + rec.started.map(|i| i.elapsed()).unwrap_or_default()).as_secs();
        Some(format!("{:02}:{:02}", seconds / 60, seconds % 60))
    }

    pub(crate) fn open_directory(&mut self, cmds: &mut Vec<Cmd>) {
        self.error = recording_dir().and_then(|dir| {
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            cmds.push(Cmd::Open(dir));
            Ok(())
        }).err();
    }

    pub(crate) fn draw_recorder(&mut self, c: &Card, cmds: &mut Vec<Cmd>, settings: &mut crate::model::Settings) {
        let phase = self.recording.as_ref().map(|r| r.phase);
        let status = match phase {
            None => "主显示器",
            Some(Phase::Starting) => "正在准备",
            Some(Phase::Recording) => "录制中",
            Some(Phase::Paused) => "已暂停",
            Some(Phase::Pausing) => "正在暂停",
            Some(Phase::Resuming) => "正在继续",
            Some(Phase::Stopping) => "正在保存",
        };
        let status_color = match phase {
            Some(Phase::Recording) => theme::RED_TOP,
            Some(Phase::Paused) => theme::YELLOW,
            _ => theme::GRAY,
        };
        c.circle(47.0, 44.0, 3.0, status_color);
        c.centered_text(57.0, 44.0, status, 11.0, W::Semibold, status_color, H::Left);
        let time = self.timer_label().unwrap_or_else(|| "00:00".into());
        c.centered_text(43.0, 72.0, &time, 26.0, W::Semibold, theme::WHITE, H::Left);

        let start_stop = if phase.is_some() { "停止并保存" } else { "开始录制" };
        let can_stop = matches!(phase, Some(Phase::Recording | Phase::Paused));
        let can_record = phase.is_none() || can_stop;
        let action_card = if can_record { c.clone() } else { c.ghost().faded(0.48) };
        let (action, inner) = capsule(&action_card, 211.0, 37.0, 105.0, 31.0, "recorder-start-stop",
            Color32::from_rgb(161, 37, 58), Color32::from_rgb(188, 42, 65));
        if phase.is_none() {
            inner.circle(227.0, 52.5, 4.5, theme::WHITE);
        } else {
            inner.fill(223.0, 48.5, 8.0, 8.0, 1.5, theme::WHITE);
        }
        inner.centered_text(269.0, 52.5, start_stop, 12.0, W::Semibold, theme::WHITE, H::Center);
        if can_record && action.clicked() {
            self.error = if can_stop { self.control("stop", Phase::Stopping) } else { self.start(settings) }.err();
        }
        if phase.is_some() {
            let paused = phase == Some(Phase::Paused);
            let transport = if can_stop { c.clone() } else { c.ghost().faded(0.48) };
            let (response, inner) = capsule(&transport, 324.0, 37.0, 53.0, 31.0, "recorder-pause", theme::BTN, theme::BTN_HOVER);
            crate::paint::play_pause(&inner, 350.5, 52.5, 12.0, if paused { 0.0 } else { 1.0 }, theme::WHITE);
            if response.on_hover_text(if paused { "继续录制" } else { "暂停录制" }).clicked() && can_stop {
                self.error = if paused { self.control("resume", Phase::Resuming) } else { self.control("pause", Phase::Pausing) }.err();
            }
        }

        let audio = if phase.is_none() { c.clone() } else { c.ghost().faded(0.70) };
        let system_changed = audio_switch(&audio, 211.0, "系统声音", &mut settings.record_system_audio);
        let mic_changed = audio_switch(&audio, 302.0, "麦克风", &mut settings.record_microphone);
        if system_changed || mic_changed { settings.save(); }
        if button(c, 43.0, 101.0, 76.0, "录屏目录", theme::BTN) {
            self.open_directory(cmds);
        }
        if let Some(path) = self.recent.as_ref() {
            if button(c, 126.0, 101.0, 86.0, "播放上次录屏", theme::BTN) { cmds.push(Cmd::Open(path.clone())); }
        }
    }

    pub(crate) fn draw_snips(&mut self, c: &Card) {
        #[cfg(windows)]
        {
            use crate::screen_snip::{self, Action};
            for (x, label, action) in [(43.0, "框选截图", Action::CopyImage), (158.0, "截图提字", Action::Ocr), (273.0, "识别二维码", Action::Qr)] {
                if button(c, x, 69.0, 104.0, label, theme::BTN) && self.snip.is_none() {
                    let (sender, receiver) = mpsc::channel();
                    self.snip = Some(receiver);
                    std::thread::spawn(move || { let _ = sender.send(screen_snip::run(action)); });
                }
            }
        }
        if self.snip.is_some() {
            c.text(210.0, 118.0, "正在框选或识别…", 11.0, W::Regular, theme::GRAY, H::Center);
        }
    }
}

fn audio_switch(c: &Card, x: f32, label: &str, enabled: &mut bool) -> bool {
    let width = c.text_w(label, 11.0, W::Regular) + 32.0;
    let r = c.hit(x, 75.0, width, 22.0, ("recorder-audio", label));
    let changed = c.live && r.clicked();
    if changed { *enabled = !*enabled; }
    let track = if *enabled { theme::GREEN } else { theme::BTN_HOVER };
    c.fill(x, 80.0, 23.0, 13.0, 6.5, track);
    c.circle(x + if *enabled { 16.5 } else { 6.5 }, 86.5, 4.5, theme::WHITE);
    c.centered_text(x + 28.0, 86.5, label, 11.0, W::Regular, theme::GRAY, H::Left);
    changed
}

fn button(c: &Card, x: f32, y: f32, width: f32, label: &str, color: Color32) -> bool {
    let (response, inner) = capsule(c, x, y, width, 23.0, label, color, color.gamma_multiply(1.25));
    inner.text(x + width / 2.0, y + 16.0, label, 12.0, W::Semibold, theme::WHITE, H::Center);
    response.clicked()
}
