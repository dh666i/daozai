//! Wireless peripheral power observations used by the desktop island.
use crate::bt::{self, DeviceKind};
use crate::model::Settings;
use std::collections::HashSet;
use std::time::Duration;

#[cfg(windows)]
mod rog;
pub(crate) mod ui;

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum Transport { Bluetooth, Wireless24, Usb }

#[derive(Clone, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Device {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub transport: Transport,
    // Settings only remember the selected device's identity between sessions.
    #[serde(skip)]
    pub level: Option<u8>,
    #[serde(skip)]
    pub charging: Option<bool>,
    #[serde(skip)]
    pub threshold: Option<u8>,
}

impl Device {
    fn same_identity(&self, other: &Device) -> bool {
        self.id == other.id && self.name == other.name && self.kind == other.kind
            && self.transport == other.transport
    }

    fn without_reading(&self) -> Self {
        Self { level: None, charging: None, threshold: None, ..self.clone() }
    }

    // v1.2.0 only stored an ID. Omni encodes its slot in that ID even when asleep.
    fn from_legacy_pin(id: &str) -> Self {
        let mut fields = id.split(':');
        let (name, kind, transport) = match (fields.next(), fields.next(), fields.next()) {
            (Some("rog"), Some("1ace"), Some("07")) => ("ROG Omni 鼠标", DeviceKind::Mouse, Transport::Wireless24),
            (Some("rog"), Some("1ace"), Some("01")) => ("ROG Omni 键盘", DeviceKind::Keyboard, Transport::Wireless24),
            (Some("rog"), Some("1c0c"), Some("07")) => ("ROG Keris II Origin", DeviceKind::Mouse, Transport::Usb),
            (Some("rog"), Some("1b04"), Some("01")) => ("ROG Falchion RX Low Profile", DeviceKind::Keyboard, Transport::Usb),
            _ => ("所选外设", DeviceKind::Other, Transport::Bluetooth),
        };
        Self { id: id.into(), name: name.into(), kind, transport, level: None, charging: None, threshold: None }
    }

    fn is_full(&self) -> bool { self.level == Some(100) && self.charging != Some(true) }
    fn low_reading(&self, threshold: u8, active: bool) -> Option<u8> {
        self.level.filter(|&level| self.charging != Some(true) && level < 100
            && (level <= threshold || (active && level < threshold.saturating_add(3))))
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Thresholds {
    warning: Option<u8>,
    resident: Option<u8>,
}

#[derive(Default)]
pub(crate) struct Panel {
    devices: Vec<Device>,
    unavailable_id: Option<String>,
    bluetooth: Vec<Device>,
    bt_seen: Option<std::time::Instant>,
    #[cfg(windows)]
    receiver: Option<Receiver>,
    wireless: Vec<Device>,
    thresholds: Thresholds,
    readings_changed: bool,
    warned: HashSet<String>,
    auto_low: HashSet<String>,
    auto_selected: Option<String>,
    scanning: bool,
    error: Option<String>,
    page: usize,
}

#[cfg(windows)]
struct Receiver {
    data: std::sync::mpsc::Receiver<Result<Vec<Device>, String>>,
    _stop: std::sync::mpsc::Sender<()>,
}

#[cfg(windows)]
impl Receiver {
    fn start() -> std::io::Result<Self> {
        let (send, data) = std::sync::mpsc::sync_channel(1);
        let (stop, cancelled) = std::sync::mpsc::channel();
        std::thread::Builder::new().name("daozai-peripheral-power".into()).spawn(move || loop {
            if send.send(rog::read_devices()).is_err() { break; }
            if cancelled.recv_timeout(Duration::from_secs(12)) != Err(std::sync::mpsc::RecvTimeoutError::Timeout) { break; }
        })?;
        Ok(Self { data, _stop: stop })
    }
}

impl Panel {
    pub(crate) fn update_bt(&mut self, found: &bt::Snapshot) {
        self.bt_seen = Some(std::time::Instant::now());
        let devices = found.devices.iter().map(|d| {
            let p = found.power.get(&d.id).filter(|p| p.observed_at.elapsed() < Duration::from_secs(150));
            Device {
                id: format!("bt-{}", d.id), name: d.name.clone(), kind: d.kind, transport: Transport::Bluetooth,
                level: p.and_then(|p| d.battery.or(p.full.then_some(100))),
                charging: p.and_then(|p| p.charging), threshold: None,
            }
        }).collect::<Vec<_>>();
        if self.bluetooth != devices { self.bluetooth = devices; self.merge(); }
    }

    fn merge(&mut self) {
        self.readings_changed = true;
        self.unavailable_id = None;
        self.devices = self.wireless.iter().chain(&self.bluetooth).cloned().collect();
        self.devices.sort_by(|a,b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    }

    #[cfg(windows)]
    pub(crate) fn tick(&mut self, enabled: bool) {
        if !enabled {
            if self.receiver.is_some() || !self.devices.is_empty() || self.error.is_some() { *self = Self::default(); }
            return;
        }
        if self.bt_seen.is_some_and(|t| t.elapsed() >= Duration::from_secs(150)) {
            self.bt_seen = None;
            self.bluetooth.clear();
            self.merge();
        }
        if self.receiver.is_none() && self.error.is_none() {
            match Receiver::start() {
                Ok(rx) => { self.receiver = Some(rx); self.scanning = true; }
                Err(e) => self.error = Some(format!("外设读取服务启动失败：{e}")),
            }
        }
        if let Some(reply) = self.receiver.as_ref().and_then(|rx| rx.data.try_recv().ok()) {
            self.scanning = false;
            match reply {
                Ok(devices) => { self.wireless = devices; self.error = None; }
                Err(e) => { self.wireless.clear(); self.error = Some(e); }
            }
            self.merge();
        }
    }

    fn update_low_power(&mut self, settings: &Settings) -> Vec<String> {
        let thresholds = Thresholds {
            warning: settings.peripheral_low_alert.then_some(settings.peripheral_warn_at),
            resident: settings.peripheral_auto.then_some(settings.peripheral_auto_at),
        };
        if !self.readings_changed && thresholds == self.thresholds { return Vec::new(); }
        self.readings_changed = false;
        let previous = std::mem::replace(&mut self.thresholds, thresholds);
        if thresholds.warning != previous.warning {
            // A new threshold takes effect even for devices that slept through the edit.
            // Preserve acknowledged readings that are still below the new threshold.
            self.warned.retain(|id| thresholds.warning.is_some_and(|at| self.devices.iter()
                .any(|device| device.id == *id && device.low_reading(at, false).is_some())));
        }
        // Only warnings survive sleep: a missing observation must vacate an automatic badge.
        self.auto_low.retain(|id| self.devices.iter().any(|device| device.id == *id));
        let mut result = Vec::new();
        for device in &self.devices {
            if let Some(at) = thresholds.warning {
                let warned = self.warned.contains(&device.id);
                if let Some(level) = device.low_reading(at, warned) {
                    if !warned {
                        self.warned.insert(device.id.clone());
                        result.push(format!("{} · 剩余 {}%", device.name, level));
                    }
                } else if device.level.is_some() || device.charging == Some(true) {
                    self.warned.remove(&device.id);
                }
            }
            let resident = self.auto_low.contains(&device.id);
            if thresholds.resident.is_some_and(|at| device.low_reading(at, resident && thresholds.resident == previous.resident).is_some()) {
                if !resident { self.auto_low.insert(device.id.clone()); }
            } else {
                self.auto_low.remove(&device.id);
            }
        }
        let current = self.auto_selected.as_deref();
        let next = self.devices.iter().filter(|device| self.auto_low.contains(&device.id))
            .min_by_key(|device| (device.level, Some(device.id.as_str()) != current));
        if next.map(|device| device.id.as_str()) != current {
            self.auto_selected = next.map(|device| device.id.clone());
        }
        result
    }

    fn low_indicator(&self, device: &Device) -> bool {
        let threshold = self.thresholds.warning.max(self.thresholds.resident)
            .or(device.threshold).unwrap_or(20);
        let active = self.auto_low.contains(&device.id) || self.warned.contains(&device.id);
        threshold != 0 && device.low_reading(threshold, active).is_some()
    }

    fn pinned(&self, id: &str) -> Option<&Device> { self.devices.iter().find(|d| d.id == id) }

    fn resident(&self, settings: &Settings) -> Option<&Device> {
        if !settings.peripheral_pin.is_empty() { return self.pinned(&settings.peripheral_pin); }
        if !settings.peripheral_auto { return None; }
        self.pinned(self.auto_selected.as_deref()?)
    }

    fn is_unavailable(&self, id: &str) -> bool { self.unavailable_id.as_deref() == Some(id) }

    /// Reconcile the selected row after a scan or an explicit change. Returns true
    /// only when the identity kept in settings changed, never for power updates.
    pub(crate) fn sync_selection(&mut self, settings: &mut Settings) -> bool {
        let selected = &settings.peripheral_pin;
        if self.is_unavailable(selected) { return false; }
        if let Some(previous) = self.unavailable_id.take() {
            self.devices.retain(|d| d.id != previous);
        }
        if selected.is_empty() {
            return settings.peripheral_identity.take().is_some();
        }
        let remembered = settings.peripheral_identity.as_ref();
        if let Some(observed) = self.pinned(selected) {
            if remembered.is_some_and(|d| d.same_identity(observed)) { return false; }
            settings.peripheral_identity = Some(observed.without_reading());
            return true;
        }
        // A missing receiver slot or Bluetooth observation says nothing about power.
        // Keep just the selected row, so its toggle remains available while offline.
        let placeholder = remembered.filter(|d| d.id == *selected)
            .map(Device::without_reading).unwrap_or_else(|| Device::from_legacy_pin(selected));
        self.devices.push(placeholder);
        self.unavailable_id = Some(selected.clone());
        if remembered.is_some_and(|d| d.id != *selected) {
            settings.peripheral_identity = None;
            return true;
        }
        false
    }
}

impl crate::app::App {
    pub fn sync_bluetooth_power(&mut self, snapshot: &bt::Snapshot) {
        if self.module_on(crate::app::PERIPHERALS_PAGE) { self.peripherals.update_bt(snapshot); }
    }
    #[cfg(windows)]
    pub fn poll_peripherals(&mut self) {
        let enabled = self.module_on(crate::app::PERIPHERALS_PAGE);
        self.peripherals.tick(enabled);
        if enabled && self.peripherals.sync_selection(&mut self.settings) {
            self.settings.save();
        }
        if enabled {
            for message in self.peripherals.update_low_power(&self.settings) {
                self.alert("外设电量低", message, crate::app::PERIPHERALS_PAGE, true);
            }
        }
    }
    pub(crate) fn pinned_peripheral(&self) -> Option<&Device> {
        self.peripherals.resident(&self.settings)
            .filter(|_| self.module_on(crate::app::PERIPHERALS_PAGE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_device_survives_missing_scans_and_restart_without_obsolete_power() {
        let mouse = Device {
            id: "rog:1ace:07:0001".into(), name: "月刃 2 RGB".into(), kind: DeviceKind::Mouse,
            transport: Transport::Wireless24, level: Some(10), charging: Some(true), threshold: Some(20),
        };
        let mut panel = Panel::default();
        let mut settings = Settings::default();
        settings.peripheral_pin = mouse.id.clone(); // selection saved by v1.2.0
        panel.sync_selection(&mut settings);
        let startup = panel.pinned(&mouse.id).unwrap();
        assert_eq!((startup.kind, startup.level, startup.charging), (mouse.kind, None, None));

        panel.wireless.push(mouse.clone());
        panel.merge();
        assert!(panel.sync_selection(&mut settings));
        assert_eq!(panel.pinned(&mouse.id), Some(&mouse));
        assert!(!panel.is_unavailable(&mouse.id));
        assert!(!panel.sync_selection(&mut settings));
        let saved = serde_json::to_value(&settings.peripheral_identity).unwrap();
        for key in ["level", "charging", "threshold"] { assert!(saved.get(key).is_none()); }
        panel.wireless.clear();
        for _ in 0..3 {
            panel.merge();
            assert!(!panel.sync_selection(&mut settings));
            assert!(panel.is_unavailable(&mouse.id));
            assert_eq!(panel.devices, [mouse.without_reading()]);
            assert!(panel.update_low_power(&settings).is_empty());
        }
        settings.peripheral_identity = serde_json::from_value(saved).unwrap();
        let mut restarted = Panel::default();
        restarted.sync_selection(&mut settings);
        assert_eq!(restarted.devices, [mouse.without_reading()]);
        let awake = Device { level: Some(46), charging: Some(false), ..mouse };
        restarted.wireless.push(awake.clone());
        restarted.merge();
        assert!(!restarted.sync_selection(&mut settings)); // readings alone never rewrite settings
        assert_eq!(restarted.devices, [awake]);
    }

    #[test]
    fn switch_or_disable_while_a_selected_bluetooth_device_is_unavailable() {
        let phones = Device {
            id: "bt-1".into(), name: "耳机".into(), kind: DeviceKind::Headphones,
            transport: Transport::Bluetooth, level: Some(80), charging: Some(false), threshold: None,
        };
        let keyboard = Device { id: "bt-2".into(), name: "键盘".into(), kind: DeviceKind::Keyboard, ..phones.clone() };
        let mut settings = Settings::default();
        let mut panel = Panel::default();
        settings.peripheral_pin = phones.id.clone();
        panel.bluetooth = vec![phones.clone(), keyboard.clone()];
        panel.merge();
        panel.sync_selection(&mut settings);
        panel.bluetooth = vec![keyboard.clone()];
        panel.merge();
        panel.sync_selection(&mut settings);
        assert!(panel.is_unavailable(&phones.id));

        settings.peripheral_pin = keyboard.id.clone();
        assert!(panel.sync_selection(&mut settings));
        assert_eq!(panel.devices, [keyboard.clone()]);
        assert!(panel.pinned(&phones.id).is_none());
        panel.bluetooth.clear();
        panel.merge();
        panel.sync_selection(&mut settings);
        assert_eq!(panel.devices, [keyboard.without_reading()]);
        settings.peripheral_pin.clear();
        assert!(panel.sync_selection(&mut settings));
        assert!(panel.devices.is_empty() && settings.peripheral_identity.is_none());
        panel.merge();
        assert!(!panel.sync_selection(&mut settings));
        assert!(panel.devices.is_empty());
    }

    fn reading(id: &str, level: Option<u8>) -> Device {
        Device {
            id: id.into(), name: id.into(), kind: DeviceKind::Mouse, transport: Transport::Wireless24,
            level, charging: None, threshold: Some(0), // user rules also cover devices with firmware alerts off
        }
    }

    fn scan(panel: &mut Panel, settings: &Settings, readings: Vec<Device>) -> (usize, Option<String>) {
        panel.wireless = readings;
        panel.merge();
        let count = panel.update_low_power(settings).len();
        (count, panel.resident(settings).map(|device| device.id.clone()))
    }

    #[test]
    fn warning_at_twenty_and_resident_at_ten_with_independent_recovery() {
        let settings = Settings { peripheral_auto_at: 10, ..Settings::default() };
        let mut panel = Panel::default();
        for (level, warnings, resident) in [
            (21, 0, false), (20, 1, false), (21, 0, false), (19, 0, false),
            (10, 0, true), (12, 0, true), (10, 0, true), (13, 0, false),
            (22, 0, false), (20, 0, false), (23, 0, false), (20, 1, false),
        ] {
            assert_eq!(scan(&mut panel, &settings, vec![reading("mouse", Some(level))]),
                (warnings, resident.then(|| "mouse".into())), "{level}%");
        }
        assert!(panel.low_indicator(&panel.devices[0]));
        assert!(panel.update_low_power(&settings).is_empty()); // repeat render
        assert!(settings.peripheral_pin.is_empty());
    }

    #[test]
    fn auto_releases_on_sleep_unknown_charging_and_full_while_warnings_survive_sleep() {
        let mut panel = Panel::default();
        let mut settings = Settings::default();
        let low = reading("mouse", Some(20));
        let shown = Some("mouse".to_owned());
        assert_eq!(scan(&mut panel, &settings, vec![low.clone()]), (1, shown.clone()));
        assert_eq!(scan(&mut panel, &settings, vec![]), (0, None));
        assert_eq!(scan(&mut panel, &settings, vec![low.clone()]), (0, shown.clone()));
        assert_eq!(scan(&mut panel, &settings, vec![reading("mouse", None)]), (0, None));
        assert_eq!(scan(&mut panel, &settings, vec![low.clone()]), (0, shown.clone()));
        let charging = Device { charging: Some(true), ..low.clone() };
        assert_eq!(scan(&mut panel, &settings, vec![charging]), (0, None));
        assert_eq!(scan(&mut panel, &settings, vec![low]), (1, shown.clone()));
        settings.peripheral_warn_at = 100;
        settings.peripheral_auto_at = 100;
        assert_eq!(scan(&mut panel, &settings, vec![reading("mouse", Some(100))]), (0, None));
        assert!(!panel.low_indicator(&panel.devices[0]));
        assert_eq!(scan(&mut panel, &settings, vec![reading("mouse", Some(99))]), (1, shown));
    }

    #[test]
    fn lowest_online_device_wins_ties_stay_stable_and_manual_selection_survives_sleep() {
        let mut panel = Panel::default();
        let mut settings = Settings { peripheral_auto_at: 10, ..Settings::default() };
        let keyboard = reading("keyboard", Some(10));
        let mut devices = vec![reading("mouse", Some(10)), keyboard.clone(), reading("headset", Some(5))];
        assert_eq!(scan(&mut panel, &settings, devices.clone()), (3, Some("headset".into())));
        devices[0].level = Some(3);
        assert_eq!(scan(&mut panel, &settings, devices.clone()), (0, Some("mouse".into())));
        devices[2].level = Some(3); // sorted headset comes first
        assert_eq!(scan(&mut panel, &settings, devices.clone()), (0, Some("mouse".into())));
        settings.peripheral_pin = "keyboard".into();
        panel.sync_selection(&mut settings);
        assert_eq!(panel.resident(&settings), Some(&keyboard));
        devices.remove(1);
        assert_eq!(scan(&mut panel, &settings, devices.clone()), (0, None));
        panel.sync_selection(&mut settings);
        assert_eq!(panel.resident(&settings), Some(&keyboard.without_reading()));
        settings.peripheral_pin.clear();
        panel.sync_selection(&mut settings);
        assert_eq!(panel.resident(&settings).unwrap().id, "mouse");
        devices[1].level = Some(12); // still in the earlier low episode
        devices[0].charging = Some(true);
        assert_eq!(scan(&mut panel, &settings, devices), (0, Some("headset".into())));
    }

    #[test]
    fn edits_take_effect_on_current_readings_and_toggles_work_independently() {
        let mut panel = Panel::default();
        let mut settings = Settings { peripheral_low_alert: false, ..Settings::default() };
        assert_eq!(scan(&mut panel, &settings, vec![reading("mouse", Some(19))]), (0, Some("mouse".into())));
        settings.peripheral_auto_at = 18; // apply the new limit immediately even inside the old recovery band
        assert!(panel.update_low_power(&settings).is_empty());
        assert!(panel.resident(&settings).is_none());
        assert!(!panel.low_indicator(&panel.devices[0]));
        settings.peripheral_low_alert = true;
        assert_eq!(panel.update_low_power(&settings).len(), 1);
        assert!(panel.low_indicator(&panel.devices[0]));
        settings.peripheral_warn_at = 18;
        assert!(panel.update_low_power(&settings).is_empty());
        assert!(!panel.low_indicator(&panel.devices[0]));
        settings.peripheral_auto_at = 19;
        assert!(panel.update_low_power(&settings).is_empty());
        assert!(panel.resident(&settings).is_some());
        settings.peripheral_auto = false;
        settings.peripheral_warn_at = 19;
        assert_eq!(panel.update_low_power(&settings).len(), 1);
        assert!(panel.resident(&settings).is_none());
    }

    #[test]
    fn a_warning_threshold_edited_during_sleep_uses_the_new_entry_boundary() {
        let mut settings = Settings { peripheral_auto: false, ..Settings::default() };
        let mut panel = Panel::default();
        assert_eq!(scan(&mut panel, &settings, vec![reading("mouse", Some(19))]), (1, None));
        assert_eq!(scan(&mut panel, &settings, vec![]), (0, None));
        settings.peripheral_warn_at = 1;
        assert!(panel.update_low_power(&settings).is_empty());
        assert_eq!(scan(&mut panel, &settings, vec![reading("mouse", Some(2))]), (0, None));
        assert!(!panel.low_indicator(&panel.devices[0]));
        assert_eq!(scan(&mut panel, &settings, vec![reading("mouse", Some(1))]), (1, None));
        assert_eq!(scan(&mut panel, &settings, vec![reading("mouse", Some(0))]), (0, None));
    }
}
