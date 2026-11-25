use super::*;

#[test]
fn recorder_audio_preferences_persist_through_the_island_controls() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = H::new();
    assert!(h.app.settings.record_system_audio);
    assert!(!h.app.settings.record_microphone);
    h.app.open_recorder();
    h.hover(H::local(200.0, 40.0), 1.0);
    h.click_text("系统声音");
    h.click_text("麦克风");
    let saved = crate::model::Settings::load();
    assert!(!saved.record_system_audio);
    assert!(saved.record_microphone);
    h.click(H::local(394.0, 18.0));
    assert!(h.app.settings_open);
    assert_eq!(h.app.settings_tab, crate::extras::SETTINGS_TAB_RECORDER);
}

#[test]
fn peripheral_preferences_upgrade_and_roundtrip(){
    let _guard=ENV_LOCK.lock().unwrap_or_else(|e|e.into_inner());let _h=H::new();
    let dir=crate::model::config_dir();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("settings.txt"),"visible_pages=1011111110\npage_order=9,8,7,6,5,4,3,2,1,0\nbluetooth_notifications=0\n").unwrap();
    let mut settings=crate::model::Settings::load();
    assert!(settings.peripheral_pin.is_empty() && settings.peripheral_low_alert);
    assert!(settings.peripheral_auto);
    assert_eq!((settings.peripheral_warn_at,settings.peripheral_auto_at),(20,20));
    assert_eq!(settings.page_order,[9,8,7,6,5,4,3,2,1,0,10]);
    assert!(settings.visible_pages[crate::app::PERIPHERALS_PAGE]);
    assert!(!settings.visible_pages[crate::app::RECORDER_PAGE] && !settings.bluetooth_notifications);
    settings.peripheral_pin="rog:1ace:07:test".into();
    settings.peripheral_low_alert=false;
    settings.peripheral_warn_at=37;
    settings.peripheral_auto=false;
    settings.peripheral_auto_at=10;
    settings.save();
    let saved=crate::model::Settings::load();
    assert_eq!(saved.peripheral_pin,settings.peripheral_pin);
    assert!(!saved.peripheral_low_alert && !saved.peripheral_auto);
    assert_eq!((saved.peripheral_warn_at,saved.peripheral_auto_at),(37,10));

    let path=dir.join("settings.txt");
    std::fs::write(&path,"peripheral_low_alert=0\n").unwrap();
    let upgraded=crate::model::Settings::load();
    assert!(!upgraded.peripheral_low_alert && upgraded.peripheral_auto);
    assert_eq!((upgraded.peripheral_warn_at,upgraded.peripheral_auto_at),(20,20));
    for (input,expected) in [("0",1),("1",1),("100",100),("999",100),("abc",20)] {
        std::fs::write(&path,format!("peripheral_warn_at={input}\nperipheral_auto_at={input}\n")).unwrap();
        let loaded=crate::model::Settings::load();
        assert_eq!((loaded.peripheral_warn_at,loaded.peripheral_auto_at),(expected,expected));
    }
}

#[test]
fn peripheral_threshold_controls_are_independent_and_persist() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = H::new();
    h.app.sync_bluetooth_power(&crate::bt::Snapshot {
        devices: vec![crate::bt::Device {
            id: "test".into(), name: "鼠标".into(), kind: crate::bt::DeviceKind::Mouse, battery: Some(80),
        }],
        ..Default::default()
    });
    h.app.settings.peripheral_pin = "bt-test".into();
    h.app.peripherals.sync_selection(&mut h.app.settings);
    assert!(h.app.settings.peripheral_identity.is_some());
    h.app.open_settings();
    h.app.settings_tab = crate::extras::SETTINGS_TAB_PERIPHERALS;
    h.ctx.set_embed_viewports(true);
    fn draw(h: &mut H, events: Vec<egui::Event>) {
        h.t += 1.0 / 60.0;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(600.0, 450.0))),
            time: Some(h.t), events, ..Default::default()
        };
        h.shapes = h.ctx.run(raw, |ctx| h.app.settings_ui(ctx, false)).shapes;
    }
    fn click(h: &mut H, pos: Pos2) {
        draw(h, vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            draw(h, vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton {
                pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default()
            }]);
        }
    }
    for _ in 0..3 { draw(&mut h, vec![]); }
    let mut decreases = Vec::new();
    let mut switches = Vec::new();
    let mut automatic = None;
    for shape in &h.shapes {
        let rect = match &shape.shape {
            egui::Shape::Text(t) if t.galley.text() == "−" => t.visual_bounding_rect(),
            egui::Shape::Text(t) if t.galley.text() == "切回自动上岛" => t.visual_bounding_rect(),
            egui::Shape::Rect(r) if r.rect.size() == egui::vec2(34.0, 20.0) => r.rect,
            _ => continue,
        };
        assert!(shape.clip_rect.contains(rect.center()));
        match &shape.shape {
            egui::Shape::Text(t) if t.galley.text() == "−" => decreases.push(rect.center()),
            egui::Shape::Text(_) => automatic = Some(rect.center()),
            _ => switches.push(rect.center()),
        }
    }
    assert_eq!(decreases.len(), 2);
    assert_eq!(switches.len(), 2);
    decreases.sort_by(|a,b| a.y.total_cmp(&b.y));
    switches.sort_by(|a,b| a.y.total_cmp(&b.y));
    let automatic = automatic.expect("手动常驻时应有切回自动上岛的入口");
    click(&mut h, switches[1]);
    click(&mut h, automatic);
    click(&mut h, decreases[1]);
    assert_eq!(h.app.settings.peripheral_auto_at, 20);
    assert!(!h.app.settings.peripheral_auto && h.app.settings.peripheral_low_alert);
    assert!(!h.app.settings.peripheral_pin.is_empty());
    click(&mut h, switches[1]);
    assert!(h.app.settings.peripheral_auto, "应能重新开启自动上岛");
    click(&mut h, switches[0]);
    assert!(!h.app.settings.peripheral_low_alert, "应能单独关闭提醒");
    click(&mut h, decreases[0]);
    for _ in 0..10 { click(&mut h, decreases[1]); }
    assert!(!h.app.settings.peripheral_low_alert && h.app.settings.peripheral_auto);
    click(&mut h, switches[0]);
    click(&mut h, automatic);
    let saved = crate::model::Settings::load();
    assert!(saved.peripheral_pin.is_empty() && saved.peripheral_identity.is_none());
    assert!(saved.peripheral_low_alert && saved.peripheral_auto);
    assert_eq!((saved.peripheral_warn_at,saved.peripheral_auto_at),(20,10));
}

/// 旧版「新邮件提醒」关着的用户：升级后邮箱模块关闭，内部开关恢复为开，不会突然开始收信。
#[test]
fn merged_mail_switch_migrates_old_off_to_module_off(){
    let _g=ENV_LOCK.lock().unwrap_or_else(|e|e.into_inner());let mut h=H::new();h.frame(vec![]);
    h.app.settings.visible_pages[crate::app::MAIL_PAGE]=true;h.app.settings.mail_enabled=false;
    assert!(!h.app.mail_active());
    h.app.migrate_merged_switches();
    assert!(!h.app.settings.visible_pages[crate::app::MAIL_PAGE],"旧的新邮件提醒关闭，迁移为邮箱模块关闭");
    assert!(h.app.settings.mail_enabled,"内部开关恢复为开，以后只由模块开关控制");
    assert!(!h.app.mail_active(),"迁移前后都不收信");
    assert!(crate::model::Settings::load().mail_enabled,"迁移结果要保存");
    h.app.set_page_visible(crate::app::MAIL_PAGE,true);
    assert!(h.app.mail_active(),"重新打开模块即恢复收信，不再需要第二个开关");
}

#[test]
fn merged_quota_switch_migrates_old_off_to_module_off(){
    let _g=ENV_LOCK.lock().unwrap_or_else(|e|e.into_inner());let mut h=H::new();h.frame(vec![]);
    h.app.settings.visible_pages[crate::app::QUOTA_PAGE]=true;h.app.quota.config.enabled=false;
    h.app.migrate_merged_switches();
    assert!(!h.app.settings.visible_pages[crate::app::QUOTA_PAGE]);
    assert!(h.app.quota.config.enabled);
    assert!(!h.app.quota_active());
}

/// 已经开着的用户不受影响。
#[test]
fn merged_switches_keep_enabled_users_unchanged(){
    let _g=ENV_LOCK.lock().unwrap_or_else(|e|e.into_inner());let mut h=H::new();h.frame(vec![]);
    let pages=h.app.settings.visible_pages;
    h.app.settings.mail_enabled=true;h.app.quota.config.enabled=true;
    h.app.migrate_merged_switches();
    assert_eq!(h.app.settings.visible_pages,pages);
    assert!(h.app.settings.mail_enabled && h.app.quota.config.enabled);
}

/// 只剩邮箱一个页面时模块关不掉：保持原来的“页面在、不收信”，不改变行为。
#[test]
fn merged_mail_switch_keeps_last_page_behavior(){
    let _g=ENV_LOCK.lock().unwrap_or_else(|e|e.into_inner());let mut h=H::new();h.frame(vec![]);
    h.app.settings.visible_pages=[false;crate::app::PAGE_COUNT];h.app.settings.visible_pages[crate::app::MAIL_PAGE]=true;
    h.app.settings.mail_enabled=false;
    h.app.migrate_merged_switches();
    assert!(h.app.settings.visible_pages[crate::app::MAIL_PAGE]);
    assert!(!h.app.settings.mail_enabled,"关不掉模块时保留旧的内部开关");
    assert!(!h.app.mail_active());
}

/// 旧的分页序号（岛上入口、更新提示等按序号跳转）仍能打开合并后的页面，每个序号只属于一个侧栏项。
#[test]
fn settings_old_tab_numbers_open_merged_pages(){
    use crate::extras::settings_tab_group_for_tests as group;
    assert_eq!(group(2),"专注与健康");assert_eq!(group(3),"专注与健康");
    assert_eq!(group(4),"待办与置物");assert_eq!(group(5),"待办与置物");
    assert_eq!(group(6),"消息提醒");assert_eq!(group(7),"邮箱");assert_eq!(group(8),"日程");
    assert_eq!(group(0),"通用");assert_eq!(group(13),"外观");assert_eq!(group(14),"位置");
    assert_eq!(group(10),"页面管理");assert_eq!(group(9),"关于");
    assert_eq!(group(crate::app::PAGE_SETTINGS_TABS[crate::app::RECORDER_PAGE]),"录屏");
    assert_eq!(group(crate::app::PAGE_SETTINGS_TABS[crate::app::PERIPHERALS_PAGE]),"外设电量");
}

/// 每个分页都能正常绘制（含功能关闭时的提示），不会因为挪动布局而崩溃。
#[test]
fn settings_every_tab_renders_with_modules_on_and_off(){
    let _g=ENV_LOCK.lock().unwrap_or_else(|e|e.into_inner());
    for on in [true,false] {
        let mut h=H::new();h.app.open_settings();h.ctx.set_embed_viewports(true);
        if !on {for page in 0..crate::app::PAGE_COUNT {if page!=crate::app::MUSIC_PAGE {h.app.settings.visible_pages[page]=false;}}}
        for tab in 0..=crate::extras::SETTINGS_TAB_PERIPHERALS {
            h.app.settings_tab=tab;
            for _ in 0..2 {let _=h.ctx.run(egui::RawInput::default(),|ctx|h.app.settings_ui(ctx,false));}
            assert_eq!(h.app.settings_tab,tab,"绘制分页 {tab} 不应自行跳转");
        }
    }
}

/// 设置窗口拉到最小（600×450）时，侧栏末尾的录屏和关于仍然可见。
#[test]
fn settings_sidebar_keeps_about_visible_at_minimum_height(){
    let _g=ENV_LOCK.lock().unwrap_or_else(|e|e.into_inner());
    let mut h=H::new();h.app.open_settings();h.ctx.set_embed_viewports(false);
    let output=std::sync::Arc::new(Mutex::new(None));let capture=output.clone();
    egui::Context::set_immediate_viewport_renderer(move|ctx,mut viewport|{
        let id=viewport.ids.this;
        let raw=egui::RawInput{viewport_id:id,screen_rect:Some(Rect::from_min_size(Pos2::ZERO,egui::vec2(600.0,450.0))),
            viewports:std::iter::once((id,egui::ViewportInfo::default())).collect(),..Default::default()};
        *capture.lock().unwrap()=Some(ctx.run(raw,|ctx|(viewport.viewport_ui_cb)(ctx)));
    });
    for _ in 0..3 {let _=h.ctx.run(Default::default(),|ctx|h.app.settings_ui(ctx,false));}
    let frame=output.lock().unwrap().take().unwrap();
    for label in ["通用","天气","录屏","外设电量","关于"] {
        let rect=frame.shapes.iter().find_map(|s|match &s.shape{egui::Shape::Text(t) if t.galley.text()==label && t.pos.x<180.0=>Some(t.visual_bounding_rect()),_=>None})
            .unwrap_or_else(||panic!("侧栏缺少 {label}"));
        assert!(rect.bottom()<=450.0,"{label} 超出窗口底部：{rect:?}");
    }
}
