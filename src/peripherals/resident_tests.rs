use super::*;
use crate::bt::{Device, DeviceKind, Power, Snapshot};
use crate::model::Dock;

fn select_mouse(h: &mut H) {
    let device = Device {
        id: "mouse".into(), name: "测试鼠标".into(), battery: Some(73), kind: DeviceKind::Mouse,
    };
    let power = Power { charging: Some(true), full: false, observed_at: std::time::Instant::now() };
    h.app.sync_bluetooth_power(&Snapshot {
        devices: vec![device], power: [("mouse".to_owned(), power)].into_iter().collect(),
    });
    h.app.settings.peripheral_pin = "bt-mouse".into();
}

fn badge(h: &H) -> Rect {
    let rect = h.ctx.read_response(egui::Id::new(("card", true, "open-peripheral-badge")))
        .expect("收起后应能点击岛上电量图标").rect;
    assert!(rect.width() >= 20.0 && rect.height() >= 20.0, "图标被裁切 {rect:?}");
    assert!(!h.shapes.iter().any(|s| matches!(&s.shape,
        egui::Shape::Text(text) if text.galley.text() == "73%")));
    rect
}

#[test]
fn lyrics_and_focus_leave_space_for_persistent_power_and_click_opens_details() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = H::new();
    select_mouse(&mut h);
    h.app.settings.island_lyrics = true;
    h.app.settings.pet = true;
    h.media = crate::smtc::NowPlaying {
        title: "Track".into(), artist: "Artist".into(), duration: 120.0, position: 7.0,
        playing: true, ..Default::default()
    };
    h.app.set_lyrics("Track", "Artist", 120.0, crate::lyrics::parse_lrc("[00:05]正在播放的歌词"));
    h.app.pomo.running = true;
    for _ in 0..120 { h.frame(vec![]); }
    let power = badge(&h);
    for text in ["正在播放的歌词", &format!("{:02}:{:02}", h.app.pomo.remaining.ceil() as u32 / 60, h.app.pomo.remaining.ceil() as u32 % 60)] {
        let bounds = h.shapes.iter().find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == text => Some(t.visual_bounding_rect()), _ => None,
        }).unwrap();
        assert!(bounds.right() + 3.0 < power.left() && h.hit.contains_rect(power));
    }
    h.media.playing = false;
    for _ in 0..90 { h.frame(vec![]); }
    let power = badge(&h);
    h.click(power.center());
    h.hover(H::local(200.0, 82.0), 0.8);
    assert!(h.app.open && h.app.page() == crate::app::PERIPHERALS_PAGE);
    assert!(h.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == "73%")));
}

#[test]
fn automatic_minimal_keeps_only_the_selected_gauge_and_restores_the_strip_when_off() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = H::new();
    h.app.settings.browser_minimal = true;
    h.browser_at_top = Some(true);
    select_mouse(&mut h);
    for _ in 0..150 { h.frame(vec![]); }
    let power = badge(&h);
    assert!(h.hit.contains_rect(power) && h.hit.width() < 60.0);
    assert!(!h.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.text() == "09:41")));
    h.app.settings.peripheral_pin.clear();
    for _ in 0..90 { h.frame(vec![]); }
    assert!(h.hit.height() < 10.0);
}

#[test]
fn left_and_right_docks_keep_the_icon_after_a_notice_and_in_manual_minimal() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    for dock in [Dock::Left, Dock::Right] {
        let mut h = H::new();
        h.app.settings.dock = dock;
        select_mouse(&mut h);
        for _ in 0..90 { side_frame(&mut h, None); }
        badge(&h);
        h.app.alert("大写锁定", "已开启", 5, false);
        for _ in 0..420 { side_frame(&mut h, None); }
        assert!(h.app.notice.is_none());
        badge(&h);
        h.app.settings.minimal = true;
        for _ in 0..90 { side_frame(&mut h, None); }
        let power = badge(&h);
        let out = side_frame(&mut h, None);
        assert!(!out.click_through && out.hit.contains(power.center()));
    }
}
