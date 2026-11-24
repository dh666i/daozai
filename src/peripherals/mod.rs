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
    fn is_low(&self) -> bool {
        let threshold = self.threshold.unwrap_or(20);
        threshold != 0 && self.charging != Some(true) && self.level.is_some_and(|n| n <= threshold)
    }
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
    warned: HashSet<String>,
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

    fn warnings(&mut self) -> Vec<String> {
        let mut result = Vec::new();
        for d in &self.devices {
            if d.is_low() {
                if self.warned.insert(d.id.clone()) {
                    result.push(format!("{} · 剩余 {}%", d.name, d.level.unwrap_or_default()));
                }
            } else if d.charging == Some(true) || d.level.is_some_and(|n| n > d.threshold.unwrap_or(20)) {
                self.warned.remove(&d.id);
            }
        }
        result
    }

    fn pinned(&self, id: &str) -> Option<&Device> { self.devices.iter().find(|d| d.id == id) }

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
        if enabled && self.settings.peripheral_low_alert {
            for message in self.peripherals.warnings() {
                self.alert("外设电量低", message, crate::app::PERIPHERALS_PAGE, true);
            }
        }
    }
    pub(crate) fn pinned_peripheral(&self) -> Option<&Device> {
        self.peripherals.pinned(&self.settings.peripheral_pin)
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
            assert!(panel.warnings().is_empty());
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

    #[test]
    fn one_warning_per_low_episode_even_after_sleep() {
        let mouse = Device { id:"rog-mouse".into(), name:"鼠标".into(), kind:DeviceKind::Mouse,
            transport:Transport::Wireless24, level:Some(20), charging:Some(false),threshold:Some(20) };
        let mut panel = Panel::default();
        panel.devices.push(mouse.clone());
        assert_eq!(panel.warnings().len(), 1);
        assert!(panel.warnings().is_empty());
        panel.devices.clear();
        assert!(panel.warnings().is_empty());
        panel.devices.push(mouse);
        assert!(panel.warnings().is_empty());
        panel.devices[0].charging = Some(true);
        assert!(panel.warnings().is_empty());
        panel.devices[0].charging = Some(false);
        assert_eq!(panel.warnings().len(), 1);
        panel.devices[0].threshold = Some(0);
        assert!(!panel.devices[0].is_low());
    }
}
