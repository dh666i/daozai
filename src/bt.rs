//! 经典蓝牙与低功耗蓝牙设备的连接状态和电量。

use std::collections::HashMap;
use std::time::{Duration, Instant};

#[cfg(any(windows, test))]
mod ble;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub battery: Option<u8>,
    pub kind: DeviceKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DeviceKind {
    Bone,
    Earbuds,
    Headphones,
    Speaker,
    Keyboard,
    Mouse,
    Gamepad,
    Phone,
    Other,
}

fn device_kind(name: &str, classic: bool, class: Option<u32>) -> DeviceKind {
    use DeviceKind::*;
    let name = name.to_lowercase();
    if ["shokz", "aftershokz", "openmove", "openrun", "骨传导"]
        .iter()
        .any(|s| name.contains(s))
    {
        return Bone;
    }
    if ["earbuds", "airpods", "buds", "earphones", "入耳"]
        .iter()
        .any(|s| name.contains(s))
    {
        return Earbuds;
    }
    if ["keyboard", "键盘"].iter().any(|s| name.contains(s)) {
        return Keyboard;
    }
    if ["mouse", "鼠标"].iter().any(|s| name.contains(s)) {
        return Mouse;
    }
    if ["gamepad", "controller", "手柄"]
        .iter()
        .any(|s| name.contains(s))
    {
        return Gamepad;
    }
    if ["speaker", "音箱", "音响"].iter().any(|s| name.contains(s)) {
        return Speaker;
    }
    if let Some(class) = class {
        if classic {
            match (class >> 8) & 0x1f {
                2 => return Phone,
                4 => {
                    return if matches!((class >> 2) & 0x3f, 5 | 7 | 10) {
                        Speaker
                    } else {
                        Headphones
                    }
                }
                5 => {
                    return match (class >> 6) & 3 {
                        1 | 3 => Keyboard,
                        2 => Mouse,
                        _ if matches!((class >> 2) & 15, 1 | 2) => Gamepad,
                        _ => Other,
                    }
                }
                _ => {}
            }
        } else {
            match (class >> 6, class & 0x3f) {
                (15, 1) => return Keyboard,
                (15, 2) => return Mouse,
                (15, 3 | 4) => return Gamepad,
                (37, 1) => return Earbuds,
                (37, _) => return Headphones,
                (33, 1 | 3 | 4 | 5) => return Speaker,
                (1, _) => return Phone,
                _ => {}
            }
        }
    }
    if ["headphone", "headset", "耳机"]
        .iter()
        .any(|s| name.contains(s))
    {
        Headphones
    } else {
        Other
    }
}

impl Device {
    /// 通知条正文：`名称 · 电量 80%`，设备没上报电量时只写名称。
    pub fn summary(&self) -> String {
        match self.battery {
            Some(b) => format!("{} · 电量 {b}%", self.name),
            None => self.name.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Power {
    pub charging: Option<bool>,
    pub full: bool,
    pub observed_at: Instant,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub devices: Vec<Device>,
    pub power: HashMap<String, Power>,
}

#[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
struct Reading {
    level: Option<u8>,
    charging: Option<bool>,
}

impl Reading {
    fn or(self, fallback: Self) -> Self {
        Self {
            level: self.level.or(fallback.level),
            charging: self.charging.or(fallback.charging),
        }
    }
}

#[derive(Default)]
struct Cache {
    snapshot: Snapshot,
    last_attempt: HashMap<String, Instant>,
}

impl Cache {
    fn update(
        &mut self,
        mut devices: Vec<Device>,
        now: Instant,
        read: impl FnOnce(&[String]) -> HashMap<String, Reading>,
    ) -> Snapshot {
        self.last_attempt
            .retain(|id, _| devices.iter().any(|d| &d.id == id));
        self.snapshot
            .power
            .retain(|id, _| self.last_attempt.contains_key(id));
        let due: Vec<_> = devices
            .iter()
            .filter(|device| {
                self.last_attempt
                    .get(&device.id)
                    .is_none_or(|t| now.duration_since(*t) >= Duration::from_secs(60))
            })
            .map(|device| device.id.clone())
            .collect();
        let mut fresh = if due.is_empty() {
            HashMap::new()
        } else {
            read(&due)
        };
        for id in due {
            self.last_attempt.insert(id, now);
        }
        for device in &mut devices {
            if let Some(reading) = fresh
                .remove(&device.id)
                .filter(|r| *r != Reading::default())
            {
                device.battery = reading.level;
                self.snapshot.power.insert(
                    device.id.clone(),
                    Power {
                        charging: reading.charging,
                        full: reading.level == Some(100) && reading.charging == Some(false),
                        observed_at: now,
                    },
                );
            } else {
                device.battery = self
                    .snapshot
                    .devices
                    .iter()
                    .find(|d| d.id == device.id)
                    .and_then(|d| d.battery);
            }
        }
        self.snapshot.devices = devices;
        self.snapshot.clone()
    }
}

/// 释放服务时唤醒后台线程，停止后续扫描。
pub struct Service {
    pub rx: std::sync::mpsc::Receiver<Snapshot>,
    stop: std::sync::mpsc::Sender<()>,
}
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.stop.send(());
    }
}

pub fn spawn() -> Service {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let (stop, stopped) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("daozai-bluetooth".into())
        .spawn(move || {
            #[cfg(windows)]
            let mut cache = Cache::default();
            #[cfg(windows)]
            let apartment = unsafe {
                windows::Win32::System::WinRT::RoInitialize(
                    windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
                )
            };
            loop {
                if stopped.try_recv() != Err(std::sync::mpsc::TryRecvError::Empty) {
                    break;
                }
                #[cfg(windows)]
                if let Ok(found) = query(&mut cache) {
                    if matches!(
                        tx.try_send(found),
                        Err(std::sync::mpsc::TrySendError::Disconnected(_))
                    ) {
                        break;
                    }
                }
                #[cfg(not(windows))]
                let _ = &tx;
                if stopped.recv_timeout(Duration::from_secs(2))
                    != Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                {
                    break;
                }
            }
            #[cfg(windows)]
            if apartment.is_ok() {
                unsafe { windows::Win32::System::WinRT::RoUninitialize() };
            }
        })
        .ok();
    Service { rx, stop }
}

#[cfg(windows)]
fn wait<T: windows::core::RuntimeType + 'static>(
    op: windows::Foundation::IAsyncOperation<T>,
    deadline: Instant,
) -> Option<T> {
    use windows::Foundation::{AsyncOperationCompletedHandler, AsyncStatus};
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    op.SetCompleted(&AsyncOperationCompletedHandler::new(move |_, status| {
        let _ = tx.try_send(status);
        Ok(())
    }))
    .ok()?;
    match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(AsyncStatus::Completed) => op.GetResults().ok(),
        Err(_) => {
            let _ = op.Cancel();
            None
        }
        _ => None,
    }
}

fn windows_power(level: Option<u8>, charging: Option<u8>, combined: Option<u8>) -> Reading {
    // System.Devices.ChargingState and System.Devices.BatteryPlusCharging.
    let combined = combined.filter(|v| *v <= 200);
    Reading {
        level: level
            .filter(|v| *v <= 100)
            .or_else(|| combined.map(|v| if v > 100 { v - 100 } else { v })),
        charging: match charging {
            Some(0) => Some(false),
            Some(1) => Some(true),
            _ => combined.map(|v| v > 100),
        },
    }
}

fn address(value: &str) -> Option<String> {
    let hex: String = value.chars().filter(|c| !matches!(c, ':' | '-')).collect();
    (hex.len() == 12 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| hex.to_ascii_uppercase())
}

#[cfg(windows)]
fn query(cache: &mut Cache) -> Result<Snapshot, ()> {
    use windows::core::{Interface, HSTRING};
    use windows::Devices::Bluetooth::{
        BluetoothConnectionStatus, BluetoothDevice, BluetoothLEDevice,
    };
    use windows::Devices::Enumeration::{DeviceInformation, DeviceInformationKind};
    use windows::Foundation::{Collections::IIterable, IPropertyValue};

    const ADDRESS: &str = "System.Devices.Aep.DeviceAddress";
    const MAJOR: &str = "System.Devices.Aep.Bluetooth.Cod.Major";
    const MINOR: &str = "System.Devices.Aep.Bluetooth.Cod.Minor";
    const APPEARANCE: &str = "System.Devices.Aep.Bluetooth.Le.Appearance";
    const LIFE: &str = "System.Devices.BatteryLife";
    const CHARGING: &str = "System.Devices.ChargingState";
    const COMBINED: &str = "System.Devices.BatteryPlusCharging";
    let props = IIterable::<HSTRING>::try_from(
        [ADDRESS, MAJOR, MINOR, APPEARANCE, LIFE, CHARGING, COMBINED]
            .map(HSTRING::from)
            .to_vec(),
    )
    .map_err(|_| ())?;
    let selectors = [
        (
            true,
            BluetoothDevice::GetDeviceSelectorFromConnectionStatus(
                BluetoothConnectionStatus::Connected,
            ),
        ),
        (
            false,
            BluetoothLEDevice::GetDeviceSelectorFromConnectionStatus(
                BluetoothConnectionStatus::Connected,
            ),
        ),
    ];
    let mut devices: Vec<Device> = Vec::new();
    let mut paired_ble = std::collections::HashSet::new();
    let mut reported: HashMap<String, Reading> = HashMap::new();
    for (classic, selector) in selectors {
        let selector = selector.map_err(|_| ())?;
        let aqs = HSTRING::from(format!("({selector}) AND System.Devices.Aep.IsConnected:=System.StructuredQueryType.Boolean#True"));
        let op = DeviceInformation::FindAllAsyncWithKindAqsFilterAndAdditionalProperties(
            &aqs,
            &props,
            DeviceInformationKind::AssociationEndpoint,
        )
        .map_err(|_| ())?;
        let found = wait(op, Instant::now() + Duration::from_secs(4)).ok_or(())?;
        for info in found {
            let (Ok(id), Ok(name), Ok(values)) = (info.Id(), info.Name(), info.Properties()) else {
                continue;
            };
            let prop = |key: &str| {
                values
                    .Lookup(&HSTRING::from(key))
                    .ok()?
                    .cast::<IPropertyValue>()
                    .ok()
            };
            let key = prop(ADDRESS)
                .and_then(|p| p.GetString().ok())
                .and_then(|s| address(&s.to_string()))
                .or_else(|| id.to_string().rsplit('-').next().and_then(address));
            let Some(key) = key else { continue };
            let name = name.to_string();
            if name.trim().is_empty() {
                continue;
            }
            let class = if classic {
                prop(MAJOR).and_then(|p| p.GetUInt16().ok()).map(|major| {
                    let minor = prop(MINOR).and_then(|p| p.GetUInt16().ok()).unwrap_or(0);
                    ((major as u32) << 8) | ((minor as u32) << 2)
                })
            } else {
                if info.Pairing().and_then(|p| p.IsPaired()).unwrap_or(false) {
                    paired_ble.insert(key.clone());
                }
                prop(APPEARANCE)
                    .and_then(|p| p.GetUInt16().ok())
                    .map(u32::from)
            };
            let byte = |key| prop(key).and_then(|p| p.GetUInt8().ok());
            let fields = windows_power(byte(LIFE), byte(CHARGING), byte(COMBINED));
            let entry = reported.entry(key.clone()).or_default();
            *entry = entry.or(fields);
            let kind = device_kind(&name, classic, class);
            if let Some(existing) = devices.iter_mut().find(|d| d.id == key) {
                if existing.kind == DeviceKind::Other {
                    existing.kind = kind;
                }
            } else {
                devices.push(Device {
                    id: key,
                    name,
                    kind,
                    battery: None,
                });
            }
        }
    }
    devices.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(cache.update(devices, Instant::now(), |due| {
        let nodes = read_batteries(due);
        for key in due {
            let fallback = reported
                .get(key)
                .copied()
                .unwrap_or_default()
                .or(nodes.get(key).copied().unwrap_or_default());
            let gatt = paired_ble
                .get(key)
                .filter(|_| fallback.level.is_none() || fallback.charging.is_none())
                .and_then(|id| ble::read(id, fallback.level.is_none()))
                .unwrap_or_default();
            reported.insert(key.clone(), gatt.or(fallback));
        }
        reported.retain(|id, _| due.contains(id));
        reported
    }))
}

#[cfg(windows)]
fn read_batteries(devices: &[String]) -> HashMap<String, Reading> {
    use windows::core::{GUID, PCWSTR};
    use windows::Win32::Devices::DeviceAndDriverInstallation::*;
    use windows::Win32::Devices::Properties::DEVPROPKEY;
    use windows::Win32::Foundation::HWND;

    const BATTERY: DEVPROPKEY = DEVPROPKEY {
        fmtid: GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5),
        pid: 2,
    };
    const DEVICE: GUID = GUID::from_u128(0x49cd1f76_5626_4b17_a4e8_18b4aa1a2213);
    let mut batteries: HashMap<String, Reading> = HashMap::new();
    unsafe {
        let Ok(set) = SetupDiGetClassDevsW(
            None,
            PCWSTR::null(),
            HWND::default(),
            DIGCF_PRESENT | DIGCF_ALLCLASSES,
        ) else {
            return batteries;
        };
        for i in 0..u32::MAX {
            let mut data = SP_DEVINFO_DATA {
                cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            if SetupDiEnumDeviceInfo(set, i, &mut data).is_err() {
                break;
            }
            let Some(instance) = instance_id(set, &data) else {
                continue;
            };
            let upper = instance.to_ascii_uppercase();
            let Some(key) = devices.iter().find(|id| upper.contains(id.as_str())) else {
                continue;
            };
            let byte = |pid| byte_prop(set, &data, &DEVPROPKEY { fmtid: DEVICE, pid });
            let reading = windows_power(
                byte_prop(set, &data, &BATTERY)
                    .filter(|v| *v <= 100)
                    .or_else(|| byte(10)),
                byte(11),
                byte(22),
            );
            let entry = batteries.entry(key.clone()).or_default();
            *entry = entry.or(reading);
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
    }
    batteries
}

#[cfg(windows)]
unsafe fn instance_id(
    set: windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO,
    data: &windows::Win32::Devices::DeviceAndDriverInstallation::SP_DEVINFO_DATA,
) -> Option<String> {
    use windows::Win32::Devices::DeviceAndDriverInstallation::SetupDiGetDeviceInstanceIdW;
    let mut buf = [0u16; 512];
    let mut need = 0u32;
    SetupDiGetDeviceInstanceIdW(set, data, Some(&mut buf), Some(&mut need)).ok()?;
    Some(String::from_utf16_lossy(
        &buf[..(need as usize).saturating_sub(1)],
    ))
}

#[cfg(windows)]
unsafe fn byte_prop(
    set: windows::Win32::Devices::DeviceAndDriverInstallation::HDEVINFO,
    data: &windows::Win32::Devices::DeviceAndDriverInstallation::SP_DEVINFO_DATA,
    key: &windows::Win32::Devices::Properties::DEVPROPKEY,
) -> Option<u8> {
    use windows::Win32::Devices::DeviceAndDriverInstallation::SetupDiGetDevicePropertyW;
    use windows::Win32::Devices::Properties::DEVPROPTYPE;
    // DEVPROP_TYPE_BYTE
    const BYTE: DEVPROPTYPE = DEVPROPTYPE(0x3);
    let mut ty = DEVPROPTYPE::default();
    let mut buf = [0u8; 1];
    let mut size = 0u32;
    SetupDiGetDevicePropertyW(set, data, key, &mut ty, Some(&mut buf), Some(&mut size), 0).ok()?;
    (ty == BYTE && size == 1).then_some(buf[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str) -> Device {
        Device {
            id: id.into(),
            name: id.into(),
            battery: None,
            kind: DeviceKind::Mouse,
        }
    }

    #[test]
    fn idle_bluetooth_does_not_rescan_all_hardware_every_two_seconds() {
        let start = Instant::now();
        let mut cache = Cache::default();
        let mut scans = 0;
        for second in (0..360).step_by(2) {
            let found = cache.update(
                vec![device("AA")],
                start + Duration::from_secs(second),
                |_| {
                    scans += 1;
                    HashMap::from([(
                        "AA".into(),
                        Reading {
                            level: Some(80),
                            charging: None,
                        },
                    )])
                },
            );
            assert_eq!(found.devices[0].battery, Some(80));
            assert_eq!(found.power["AA"].charging, None);
        }
        assert_eq!(scans, 6, "六分钟内同一设备的电量扫描应最多每分钟一次");
    }

    #[test]
    fn bluetooth_connection_change_refreshes_immediately_and_failed_scans_back_off() {
        let now = std::time::Instant::now();
        let mut cache = Cache::default();
        cache.update(vec![device("AA")], now, |_| {
            HashMap::from([(
                "AA".into(),
                Reading {
                    level: Some(80),
                    charging: Some(true),
                },
            )])
        });
        let found = cache.update(
            vec![device("AA"), device("BB")],
            now + Duration::from_secs(2),
            |due| {
                assert_eq!(due, ["BB"]);
                HashMap::from([(
                    "BB".into(),
                    Reading {
                        level: Some(60),
                        charging: None,
                    },
                )])
            },
        );
        assert_eq!(found.devices[1].battery, Some(60));
        let found = cache.update(
            vec![device("AA"), device("BB")],
            now + Duration::from_secs(62),
            |_| HashMap::new(),
        );
        assert_eq!(found.devices[0].battery, Some(80));
        assert_eq!(found.power["AA"].observed_at, now);
        cache.update(
            vec![device("AA"), device("BB")],
            now + Duration::from_secs(64),
            |_| panic!("失败后不能立即重复扫描"),
        );
        assert!(cache
            .update(vec![], now + Duration::from_secs(66), |_| panic!(
                "无设备不能扫描整机"
            ))
            .power
            .is_empty());
        let found = cache.update(vec![device("AA")], now + Duration::from_secs(68), |_| {
            HashMap::new()
        });
        assert_eq!(found.devices[0].battery, None);
        assert!(found.power.is_empty());
    }

    #[test]
    fn identifies_connected_audio_and_input_devices() {
        assert_eq!(
            device_kind("OpenMove by Shokz", true, Some(0x240418)),
            DeviceKind::Bone
        );
        assert_eq!(
            device_kind("QingLong 87 Ultra 8K", false, Some(961)),
            DeviceKind::Keyboard
        );
        assert_eq!(
            device_kind("MX Anywhere", false, Some(962)),
            DeviceKind::Mouse
        );
        assert_eq!(
            device_kind("耳机", true, Some(0x0404)),
            DeviceKind::Headphones
        );
        assert_eq!(device_kind("AirPods", true, None), DeviceKind::Earbuds);
        assert_eq!(
            device_kind("LE Desk Audio", false, Some(0x0943)),
            DeviceKind::Headphones
        );
        assert_eq!(
            device_kind("BT Accessory", false, Some(0x0941)),
            DeviceKind::Earbuds
        );
        assert_eq!(
            device_kind("BT Accessory", true, Some(0x0508)),
            DeviceKind::Gamepad
        );
        assert_eq!(device_kind("未知设备", false, None), DeviceKind::Other);
    }

    #[test]
    fn windows_reports_explicit_charging_and_keeps_unknowns() {
        assert_eq!(
            windows_power(None, None, Some(169)),
            Reading {
                level: Some(69),
                charging: Some(true)
            }
        );
        assert_eq!(
            windows_power(None, None, Some(100)),
            Reading {
                level: Some(100),
                charging: Some(false)
            }
        );
        assert_eq!(windows_power(None, None, Some(201)), Reading::default());
        assert_eq!(
            windows_power(Some(100), Some(2), None),
            Reading {
                level: Some(100),
                charging: None
            }
        );
        assert_eq!(
            windows_power(None, Some(1), None),
            Reading {
                level: None,
                charging: Some(true)
            }
        );
        assert_eq!(
            windows_power(Some(0), Some(0), None),
            Reading {
                level: Some(0),
                charging: Some(false)
            }
        );
        assert_eq!(
            address("a1:b2:c3:d4:e5:f6").as_deref(),
            Some("A1B2C3D4E5F6")
        );
        assert_eq!(address(""), None);
        assert_eq!(address("Keyboard"), None);
    }

    #[test]
    fn summary_omits_missing_battery() {
        let d = Device {
            id: "1".into(),
            name: "Headset".into(),
            battery: None,
            kind: DeviceKind::Headphones,
        };
        assert_eq!(d.summary(), "Headset");
        let d = Device {
            battery: Some(80),
            ..d
        };
        assert_eq!(d.summary(), "Headset · 电量 80%");
    }
}

#[cfg(test)]
mod lifecycle_tests {
    #[test]
    fn dropping_service_signals_stop_even_without_successful_scans() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let (stop, stopped) = std::sync::mpsc::channel();
        drop(super::Service { rx, stop });
        assert_eq!(
            stopped.recv_timeout(std::time::Duration::from_millis(100)),
            Ok(())
        );
    }
}
