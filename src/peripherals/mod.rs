//! Wireless peripheral power observations used by the desktop island.
use crate::bt::{self, DeviceKind};
use std::collections::HashSet;
use std::time::Duration;

#[cfg(windows)]
mod rog;
pub(crate) mod ui;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Transport { Bluetooth, Wireless24, Usb }

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Device {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub transport: Transport,
    pub level: Option<u8>,
    pub charging: Option<bool>,
    pub threshold: Option<u8>,
}

impl Device {
    fn is_full(&self) -> bool { self.level == Some(100) && self.charging != Some(true) }
    fn is_low(&self) -> bool {
        let threshold = self.threshold.unwrap_or(20);
        threshold != 0 && self.charging != Some(true) && self.level.is_some_and(|n| n <= threshold)
    }
}

#[derive(Default)]
pub(crate) struct Panel {
    devices: Vec<Device>,
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
}

impl crate::app::App {
    pub fn sync_bluetooth_power(&mut self, snapshot: &bt::Snapshot) {
        if self.module_on(crate::app::PERIPHERALS_PAGE) { self.peripherals.update_bt(snapshot); }
    }
    #[cfg(windows)]
    pub fn poll_peripherals(&mut self) {
        let enabled = self.module_on(crate::app::PERIPHERALS_PAGE);
        self.peripherals.tick(enabled);
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
