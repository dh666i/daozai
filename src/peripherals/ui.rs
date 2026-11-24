use super::{Device, Panel, Transport};
use crate::paint::{self, Card, H};
use crate::theme::{self, W};
use eframe::egui::{self, Color32, Id};
use std::f32::consts::{FRAC_PI_2, TAU};
use std::time::Duration;

pub(crate) const COMPACT_WIDTH: f32 = 24.0;
const PAGE_SIZE: usize = 2;
const FILL_SECONDS: f64 = 0.45;

fn status(device: &Device) -> &'static str {
    if device.charging == Some(true) {
        return "充电中";
    }
    if device.is_full() {
        return "电量已满";
    }
    if device.is_low() {
        return if device.charging.is_none() {
            "低电量 · 充电状态未知"
        } else {
            "低电量 · 未充电"
        };
    }
    match (device.level, device.charging) {
        (None, Some(false)) => "电量未知 · 未充电",
        (None, None) => "电量与充电状态未知",
        (_, Some(false)) => "未充电",
        _ => "充电状态未知",
    }
}

fn tint(device: &Device) -> Color32 {
    if device.charging == Some(true) || device.is_full() {
        theme::GREEN
    } else if device.is_low() {
        theme::YELLOW
    } else if device.level.is_some() {
        theme::CYAN
    } else {
        theme::DIM
    }
}

fn transport_name(device: &Device) -> &'static str {
    match device.transport {
        Transport::Wireless24 => "2.4 GHz",
        Transport::Bluetooth => "蓝牙",
        Transport::Usb => "USB",
    }
}

fn percentage(device: &Device) -> String {
    device.level.map_or_else(|| "—".into(), |level| format!("{level}%"))
}

#[derive(Clone, Copy)]
struct Fill {
    from: f32,
    target: f32,
    start: f64,
}

impl Fill {
    fn value(self, time: f64) -> f32 {
        let t = ((time - self.start) / FILL_SECONDS).clamp(0.0, 1.0) as f32;
        self.from + (self.target - self.from) * t * t * (3.0 - 2.0 * t)
    }
}

fn animated_level(ctx: &egui::Context, device: &Device, time: f64, motion: bool) -> Option<f32> {
    let id = Id::new(("peripheral-fill", &device.id));
    ctx.data_mut(|data| {
        let Some(level) = device.level else {
            data.remove::<Fill>(id);
            return None;
        };
        let target = f32::from(level) / 100.0;
        let fill = data.get_temp_mut_or(
            id,
            Fill {
                from: target,
                target,
                start: time,
            },
        );
        if !motion {
            *fill = Fill {
                from: target,
                target,
                start: time,
            };
        } else if fill.target != target {
            *fill = Fill {
                from: fill.value(time),
                target,
                start: time,
            };
        }
        Some(fill.value(time))
    })
}

fn frame_key(ctx: &egui::Context) -> Id {
    Id::new(("peripheral-animation", ctx.viewport_id()))
}

pub(crate) fn needs_frame(ctx: &egui::Context) -> bool {
    let key = frame_key(ctx);
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|data| data.get_temp::<u64>(key) == Some(pass))
}

fn draw_gauge(c: &Card, device: &Device, cx: f32, cy: f32, radius: f32) {
    let diameter = radius * 2.0;
    if c.painter.opacity() <= 0.0
        || !c
            .painter
            .clip_rect()
            .intersects(c.rect(cx - radius - 2.0, cy - radius - 2.0, diameter + 4.0, diameter + 4.0))
    {
        return;
    }
    let ctx = c.ui.ctx();
    let time = ctx.input(|i| i.time);
    let motion = ctx.style().animation_time > 0.0 && crate::bar_layout::animations_enabled();
    let level = animated_level(ctx, device, time, motion);
    let charging = device.charging == Some(true);
    let low = device.is_low();
    let transitioning = level
        .zip(device.level)
        .is_some_and(|(shown, actual)| (shown - f32::from(actual) / 100.0).abs() > f32::EPSILON);
    if c.live && motion && (charging || low || transitioning) {
        let pass = ctx.cumulative_pass_nr();
        let key = frame_key(ctx);
        ctx.data_mut(|data| data.insert_temp(key, pass));
        ctx.request_repaint_after(Duration::from_secs_f64(1.0 / 60.0));
    }

    let color = if low && motion {
        let light = (1.0 - (TAU * (time % 2.4) as f32 / 2.4).cos()) * 0.18;
        paint::mix(tint(device), theme::WHITE, light)
    } else {
        tint(device)
    };
    let width = (radius * 0.15).clamp(1.5, 3.5);
    c.ring(cx, cy, radius, width, theme::BTN_HOVER);
    if let Some(fill) = level {
        if fill >= 1.0 {
            c.ring(cx, cy, radius, width, color);
        } else if fill > 0.0 {
            c.arc(cx, cy, radius, -FRAC_PI_2, -FRAC_PI_2 + fill * TAU, width, color);
        } else if low {
            c.circle(cx, cy - radius, width / 2.0, color);
        }
    }
    let size = radius * 1.36;
    crate::extras::device_icon(c, device.kind, cx - size / 2.0, cy - size / 2.0, size);
    if charging {
        // Animate the charging symbol independently of the reported quantity.
        let progress = ((time % 1.6) / 1.6) as f32;
        let light = if motion { (1.0 - (progress * TAU).cos()) * 0.5 } else { 1.0 };
        if motion {
            if let Some(fill) = level.filter(|value| *value >= 0.03) {
                // Keep the moving highlight within the measured portion of the ring.
                let span = TAU * fill;
                let length = span.min(0.55);
                let start = -FRAC_PI_2 + (span - length) * progress;
                c.arc(
                    cx,
                    cy,
                    radius,
                    start,
                    start + length,
                    width,
                    paint::mix(color, theme::WHITE, light * 0.8),
                );
            }
        }
        let cx = cx + radius * 0.68;
        let cy = cy + radius * 0.68;
        let badge = radius * 0.43;
        c.circle(cx, cy, badge + 0.6, theme::CARD);
        c.circle(cx, cy, badge, paint::mix(theme::SOFT_GREEN, theme::CHARGE, light));
        let s = badge * 0.78;
        c.poly(
            &[
                (cx + s * 0.23, cy - s),
                (cx - s * 0.63, cy + s * 0.15),
                (cx + s * 0.11, cy + s * 0.15),
            ],
            theme::CARD,
        );
        c.poly(
            &[
                (cx - s * 0.09, cy - s * 0.12),
                (cx + s * 0.63, cy - s * 0.12),
                (cx - s * 0.23, cy + s),
            ],
            theme::CARD,
        );
    }
}

pub(crate) fn draw_compact(c: &Card, cx: f32, cy: f32, device: &Device) {
    draw_gauge(c, device, cx, cy, 9.5);
}

fn draw_device(c: &Card, device: &Device, y: f32, large: bool, unavailable: bool, pinned: &mut String) -> bool {
    let (cx, radius, x, title_size, number_size, line_gap) = if large {
        (72.0, 24.0, 109.0, 15.0, 21.0, 10.0)
    } else {
        (59.0, 13.5, 83.0, 12.0, 14.0, 7.0)
    };
    draw_gauge(c, device, cx, y, radius);
    let value = percentage(device);
    let number_w = c.text_w(&value, number_size, W::Semibold);
    let value_right = if large { 374.0 } else { 296.0 };
    let title_width = value_right - number_w - x - 10.0;
    let title = c.fit(&device.name, title_size, W::Semibold, title_width);
    c.centered_text(x, y - line_gap, &title, title_size, W::Semibold, theme::WHITE, H::Left);
    c.centered_text(value_right, y - line_gap, &value, number_size, W::Semibold, tint(device), H::Right);
    let detail = format!("{} · {}", transport_name(device), if unavailable { "暂时无法读取" } else { status(device) });
    c.centered_text(
        x,
        y + line_gap,
        &detail,
        if large { 11.5 } else { 10.0 },
        W::Regular,
        theme::GRAY,
        H::Left,
    );

    let selected = pinned == &device.id;
    let button_y = if large { 103.0 } else { y - 10.0 };
    let (response, button) = paint::capsule(
        c,
        306.0,
        button_y,
        68.0,
        20.0,
        &format!("island-peripheral-{}", device.id),
        Color32::from_gray(if selected { 39 } else { 22 }),
        Color32::from_gray(47),
    );
    button.text(
        340.0, button_y + 14.0, "岛上电量", 10.0, W::Semibold,
        if selected { theme::WHITE } else { theme::GRAY }, H::Center,
    );
    let response = response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if selected {
            "关闭这台外设的岛上电量"
        } else {
            "收起时显示这台外设的电量"
        });
    let changed = c.live && response.clicked();
    if changed {
        if selected {
            pinned.clear();
        } else {
            pinned.clone_from(&device.id);
        }
    }
    if title != device.name && c.live {
        c.ui.interact(
            c.rect(x, y - line_gap - 9.0, title_width, 18.0),
            Id::new(("peripheral-name", &device.id)),
            egui::Sense::hover(),
        )
        .on_hover_text(&device.name);
    }
    changed
}

impl Panel {
    pub(crate) fn draw(&mut self, c: &Card, pinned: &mut String) -> bool {
        let page_count = self.devices.len().div_ceil(PAGE_SIZE);
        self.page = self.page.min(page_count.saturating_sub(1));
        if self.devices.is_empty() {
            let (message, detail) = if let Some(error) = self.error.as_deref() {
                ("暂时无法读取外设", error)
            } else if self.scanning {
                ("正在读取外设电量", "")
            } else {
                ("未发现支持的外设", "请连接或唤醒鼠标、键盘或耳机")
            };
            c.centered_text(209.5, 69.0, message, 14.0, W::Semibold, theme::WHITE, H::Center);
            let fitted = c.fit(detail, 11.0, W::Regular, 308.0);
            c.centered_text(209.5, 92.0, &fitted, 11.0, W::Regular, theme::GRAY, H::Center);
            if detail != fitted && c.live {
                c.ui.interact(
                    c.rect(45.0, 82.0, 330.0, 20.0),
                    Id::new("peripheral-error-detail"),
                    egui::Sense::hover(),
                )
                .on_hover_text(detail);
            }
            return false;
        }
        let large = self.devices.len() == 1;
        let footer = page_count > 1 || self.error.is_some();
        let top = if large {
            74.0
        } else if footer {
            49.0
        } else {
            54.0
        };
        let step = if footer { 34.0 } else { 43.0 };
        let mut pin_changed = false;
        for (row, device) in self.devices.iter().skip(self.page * PAGE_SIZE).take(PAGE_SIZE).enumerate() {
            if row > 0 {
                c.seg((45.0, top + step / 2.0), (374.0, top + step / 2.0), 1.0, theme::TRACK);
            }
            pin_changed |= draw_device(c, device, top + row as f32 * step, large, self.is_unavailable(&device.id), pinned);
        }
        if let Some(error) = self.error.as_deref() {
            c.centered_text(45.0, 113.0, "部分外设读取失败", 10.0, W::Regular, theme::DIM, H::Left);
            if c.live {
                c.ui.interact(
                    c.rect(42.0, 103.0, 95.0, 19.0),
                    Id::new("peripheral-partial-error"),
                    egui::Sense::hover(),
                )
                .on_hover_text(error);
            }
        }
        if page_count > 1 {
            c.centered_text(
                333.0,
                113.0,
                &format!("{} / {page_count}", self.page + 1),
                10.0,
                W::Regular,
                theme::GRAY,
                H::Center,
            );
            for (x, dir, next, hint) in [
                (301.0, -1.0, self.page.saturating_sub(1), "上一组外设"),
                (365.0, 1.0, (self.page + 1).min(page_count - 1), "下一组外设"),
            ] {
                let enabled = next != self.page;
                let button = if enabled { c.clone() } else { c.faded(0.35).ghost() };
                let response = button
                    .hit(x - 12.0, 103.0, 24.0, 20.0, ("peripheral-page", hint))
                    .on_hover_text(hint);
                let (hover, press) = button.hover_t(&response);
                paint::chevron(&button, x, 113.0, dir, hover, press);
                if c.live && enabled && response.clicked() {
                    self.page = next;
                }
            }
        }
        pin_changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bt::DeviceKind;

    fn device(name: &str, kind: DeviceKind, level: Option<u8>, charging: Option<bool>) -> Device {
        Device {
            id: name.into(),
            name: name.into(),
            kind,
            transport: Transport::Bluetooth,
            level,
            charging,
            threshold: None,
        }
    }

    #[test]
    fn unknown_readings_clear_interpolation_and_remain_distinct_from_zero() {
        let ctx = egui::Context::default();
        let mut d = device("耳机", DeviceKind::Earbuds, Some(0), None);
        assert_eq!(percentage(&d), "0%");
        assert!(status(&d).contains("低电量"));
        assert_eq!(animated_level(&ctx, &d, 0.0, true), Some(0.0));
        d.level = Some(80);
        assert_eq!(animated_level(&ctx, &d, 1.0, true), Some(0.0));
        let values: Vec<_> = (1..25)
            .map(|n| animated_level(&ctx, &d, 1.0 + f64::from(n) / 60.0, true).unwrap())
            .collect();
        assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(animated_level(&ctx, &d, 2.0, true), Some(0.8));
        d.level = None;
        assert_eq!(animated_level(&ctx, &d, 3.0, true), None);
        assert_eq!(percentage(&d), "—");
        assert!(status(&d).contains("未知"));
        d.level = Some(100);
        assert_eq!(animated_level(&ctx, &d, 4.0, true), Some(1.0));
        assert_eq!(status(&d), "电量已满");
        d.level = Some(5);
        d.charging = Some(true);
        assert_eq!(animated_level(&ctx, &d, 5.0, false), Some(0.05));
        assert_eq!(status(&d), "充电中");
    }

    #[test]
    fn collapsed_gauges_fit_both_docks_and_keep_device_icons_while_charging() {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let mouse = device("鼠标", DeviceKind::Mouse, Some(20), Some(true));
        let keyboard = device("键盘", DeviceKind::Keyboard, Some(100), Some(true));
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(130.0, 40.0));
        let top = egui::Rect::from_center_size(egui::pos2(30.0, 20.0), egui::vec2(COMPACT_WIDTH, 30.0));
        let side = egui::Rect::from_center_size(egui::pos2(92.0, 20.0), egui::vec2(30.0, 32.0));
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(screen),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                    let c = Card::new(ui, screen.min, screen);
                    draw_compact(&c, top.center().x, top.center().y, &mouse);
                    draw_compact(&c, side.center().x, side.center().y, &keyboard);
                    assert_eq!(needs_frame(ctx), crate::bar_layout::animations_enabled());
                });
            },
        );
        let mut icons = Vec::new();
        for clipped in output.shapes {
            let bounds = clipped.shape.visual_bounding_rect();
            match clipped.shape {
                egui::Shape::Text(text) => panic!("unexpected collapsed label: {}", text.galley.text()),
                egui::Shape::Mesh(mesh) if mesh.texture_id != egui::TextureId::default() => {
                    icons.push((mesh.texture_id, bounds));
                }
                egui::Shape::Circle(_) | egui::Shape::Path(_) => {
                    assert!(top.contains_rect(bounds) || side.contains_rect(bounds), "{bounds:?}");
                }
                _ => {}
            }
        }
        assert_eq!(icons.len(), 2);
        assert_ne!(icons[0].0, icons[1].0, "each charging device must keep its own icon");
        assert!(top.contains_rect(icons[0].1) && side.contains_rect(icons[1].1));
    }

    #[test]
    fn charging_ring_changes_at_display_cadence() {
        if !crate::bar_layout::animations_enabled() {
            return;
        }
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let d = device("鼠标", DeviceKind::Mouse, Some(22), Some(true));
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(64.0, 64.0));
        let mut raster = crate::preview_raster::Raster::new(128, 128);
        let mut previous = Vec::new();
        let mut rgba = Vec::new();
        let mut changes = 0;
        for frame in 0..60 {
            let output = ctx.run(
                egui::RawInput {
                    viewports: [(
                        egui::ViewportId::ROOT,
                        egui::ViewportInfo {
                            native_pixels_per_point: Some(2.0),
                            ..Default::default()
                        },
                    )]
                    .into_iter()
                    .collect(),
                    screen_rect: Some(screen),
                    time: Some(0.2 + f64::from(frame) / 60.0),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                        draw_compact(&Card::new(ui, screen.min, screen), 32.0, 32.0, &d);
                    });
                },
            );
            raster.update_textures(&output.textures_delta);
            raster.buf.fill([0.0; 4]);
            raster.draw(&ctx.tessellate(output.shapes, 2.0), 2.0);
            raster.to_rgba8(&mut rgba);
            if frame > 0 && previous != rgba {
                changes += 1;
            }
            std::mem::swap(&mut previous, &mut rgba);
        }
        assert_eq!(changes, 59, "charging ring changed on {changes}/59 consecutive frames");
    }

    #[test]
    fn island_power_toggle_selects_one_device_across_pages_and_keeps_full_readings() {
        let ctx = egui::Context::default();
        theme::load_fonts(&ctx);
        egui_extras::install_image_loaders(&ctx);
        let mut panel = Panel::default();
        panel.devices = vec![
            device("鼠标", DeviceKind::Mouse, Some(22), Some(false)),
            device("键盘", DeviceKind::Keyboard, None, None),
            device("耳机", DeviceKind::Headphones, Some(87), Some(true)),
        ];
        let mut pinned = String::new();
        let frame = |panel: &mut Panel, pinned: &mut String, events: Vec<egui::Event>| {
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(419.0, 140.0));
            let mut animating = false;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                        panel.draw(&Card::new(ui, screen.min, screen), pinned);
                        animating = needs_frame(ctx);
                    });
                },
            );
            let text: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                    _ => None,
                })
                .collect();
            (text, animating)
        };
        let (first, animating) = frame(&mut panel, &mut pinned, vec![]);
        assert!(first.iter().any(|t| t == "22%") && first.iter().any(|t| t == "—"));
        assert_eq!(first.iter().filter(|t| t.as_str() == "岛上电量").count(), 2);
        assert!(!animating);
        let click = |panel: &mut Panel, pinned: &mut String, pos| {
            frame(panel, pinned, vec![egui::Event::PointerMoved(pos)]);
            for pressed in [true, false] {
                frame(
                    panel,
                    pinned,
                    vec![egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
            }
        };
        click(&mut panel, &mut pinned, egui::pos2(361.0, 49.0));
        assert_eq!(pinned, "鼠标");
        click(&mut panel, &mut pinned, egui::pos2(365.0, 113.0));
        assert_eq!(panel.page, 1);
        click(&mut panel, &mut pinned, egui::pos2(361.0, 49.0));
        assert_eq!(pinned, "耳机");
        click(&mut panel, &mut pinned, egui::pos2(361.0, 49.0));
        assert!(pinned.is_empty());
        let (second, animating) = frame(&mut panel, &mut pinned, vec![]);
        assert!(second.iter().any(|t| t == "蓝牙 · 充电中"));
        assert_eq!(animating, crate::bar_layout::animations_enabled());
        ctx.style_mut(|s| s.animation_time = 0.0);
        assert!(!frame(&mut panel, &mut pinned, vec![]).1);
        panel.devices.truncate(1);
        frame(&mut panel, &mut pinned, vec![]);
        assert_eq!(panel.page, 0);
        click(&mut panel, &mut pinned, egui::pos2(340.0, 113.0));
        assert_eq!(pinned, "鼠标");
    }
}
