//! Battery telemetry for ROG Omni and its supported wired devices.
//! Packet layouts: https://github.com/phifogg/squire-gear/blob/main/protocol.json
use super::{Device, Transport};
use crate::bt::DeviceKind;
use hidapi::{DeviceInfo, HidApi};
use std::io::Read;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Mouse,
    Keyboard,
}

impl Slot {
    fn kind(self) -> DeviceKind {
        match self {
            Self::Mouse => DeviceKind::Mouse,
            Self::Keyboard => DeviceKind::Keyboard,
        }
    }
    fn query(self) -> u8 {
        match self {
            Self::Mouse => 0x07,
            Self::Keyboard => 0x01,
        }
    }
}

#[derive(Clone, Copy)]
struct Endpoint {
    slot: Slot,
    wireless: bool,
}

impl Endpoint {
    fn select(vid: u16, pid: u16, interface: i32, page: u16, usage: u16) -> Option<Self> {
        if vid != 0x0b05 || usage != 1 {
            return None;
        }
        let (slot, wireless) = match (pid, interface, page) {
            (0x1ace, 2, 0xff01) => (Slot::Mouse, true),
            (0x1ace, 2, 0xff00) => (Slot::Keyboard, true),
            (0x1c0c, 0, 0xff01) => (Slot::Mouse, false),
            (0x1b04, 1, 0xff00) => (Slot::Keyboard, false),
            _ => return None,
        };
        Some(Self { slot, wireless })
    }

    fn report_id(self) -> u8 {
        match (self.wireless, self.slot) {
            (false, _) => 0,
            (true, Slot::Mouse) => 3,
            (true, Slot::Keyboard) => 2,
        }
    }

    fn exchange(self, api: &HidApi, info: &DeviceInfo) -> Result<Option<Power>, String> {
        let handle = info
            .open_device(api)
            .map_err(|e| format!("无法打开 ROG 设备：{e}"))?;
        let mut request = [0u8; 65];
        request[..3].copy_from_slice(&[self.report_id(), 0x12, self.slot.query()]);
        let request = &request[..if self.wireless { 64 } else { 65 }];
        let written = handle
            .write(request)
            .map_err(|e| format!("ROG 电量查询失败：{e}"))?;
        if written != request.len() {
            return Err("ROG 查询报文发送不完整".into());
        }

        // The receiver may emit FF AA immediately before a valid reading.
        let deadline = Instant::now() + Duration::from_millis(320);
        let mut unavailable = false;
        let mut response = [0; 65];
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let n = handle
                .read_timeout(&mut response, remaining.as_millis() as i32)
                .map_err(|e| format!("读取 ROG 电量失败：{e}"))?;
            if n == 0 {
                break;
            }
            match self.decode(&response[..n]) {
                Reply::Battery(power) => return Ok(Some(power)),
                Reply::Unavailable => unavailable = true,
                Reply::Other => {}
            }
        }
        if unavailable {
            Ok(None)
        } else {
            Err("ROG 设备未返回有效电量数据".into())
        }
    }

    fn decode(self, report: &[u8]) -> Reply {
        if report.len() != 64 {
            return Reply::Other;
        }
        // hidapi removes the zero prefix from unnumbered Windows input reports.
        let data = if self.wireless {
            let Some(rest) = report.strip_prefix(&[self.report_id()]) else {
                return Reply::Other;
            };
            rest
        } else {
            report
        };
        if data.starts_with(&[0xff, 0xaa]) {
            return Reply::Unavailable;
        }
        if !data.starts_with(&[0x12, self.slot.query(), 0, 0]) {
            return Reply::Other;
        }
        let (level, flag, threshold) = match self.slot {
            Slot::Mouse => (data[4], data[9], data[6]),
            Slot::Keyboard => (
                if data[10] <= 100 { data[10] } else { data[5] },
                data[8],
                data[9],
            ),
        };
        if level > 100 {
            return Reply::Other;
        }
        Reply::Battery(Power {
            level,
            charging: match flag {
                0 => Some(false),
                1 => Some(true),
                _ => None,
            },
            threshold: (threshold <= 100).then_some(threshold),
        })
    }

    fn name(self, info: &DeviceInfo, catalogue: Option<&roxmltree::Document<'_>>) -> String {
        if self.wireless {
            if let (Some(doc), Some(serial)) = (catalogue, info.serial_number()) {
                if let Some(name) = armoury_name(doc, serial, self.slot) {
                    return name;
                }
            }
        }
        match (self.wireless, self.slot) {
            (true, Slot::Mouse) => "ROG Omni 鼠标",
            (true, Slot::Keyboard) => "ROG Omni 键盘",
            (false, Slot::Mouse) => "ROG Keris II Origin",
            (false, Slot::Keyboard) => "ROG Falchion RX Low Profile",
        }
        .into()
    }

    fn id(self, info: &DeviceInfo) -> String {
        let path = info.path().to_string_lossy();
        let serial = info
            .serial_number()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&path);
        let digest = ring::digest::digest(
            &ring::digest::SHA256,
            serial.to_ascii_lowercase().as_bytes(),
        );
        let suffix = u64::from_be_bytes(digest.as_ref()[..8].try_into().unwrap());
        format!(
            "rog:{:04x}:{:02x}:{suffix:016x}",
            info.product_id(),
            self.slot.query()
        )
    }
}

#[derive(Debug, PartialEq)]
struct Power {
    level: u8,
    charging: Option<bool>,
    threshold: Option<u8>,
}

#[derive(Debug, PartialEq)]
enum Reply {
    Battery(Power),
    Unavailable,
    Other,
}

fn read_catalogue() -> Option<String> {
    let root = std::env::var_os("ProgramData")?;
    let file = std::fs::File::open(
        std::path::Path::new(&root).join("ASUS/RogAura30/GetDeviceStatusNew.xml"),
    )
    .ok()?;
    let mut xml = String::new();
    file.take(512 * 1024).read_to_string(&mut xml).ok()?;
    Some(xml)
}

fn armoury_name(doc: &roxmltree::Document<'_>, serial: &str, slot: Slot) -> Option<String> {
    if serial.trim().is_empty() {
        return None;
    }
    for node in doc
        .root_element()
        .children()
        .filter(|n| n.has_tag_name("Name"))
    {
        let kind = node
            .attribute("DeviceType")
            .unwrap_or("")
            .to_ascii_lowercase();
        if !kind.contains(match slot {
            Slot::Mouse => "mouse",
            Slot::Keyboard => "keyboard",
        }) {
            continue;
        }
        let field = |tag: &str| {
            node.children()
                .find(|n| n.has_tag_name(tag))
                .and_then(|n| n.text())
        };
        let receiver = field("DeviceID")
            .and_then(|id| id.rsplit_once('_'))
            .map(|(_, s)| s.trim());
        if !receiver.is_some_and(|s| s.eq_ignore_ascii_case(serial)) {
            continue;
        }
        let names = field("DisplayNameMultiLang")
            .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok());
        let name = names
            .as_ref()
            .and_then(|v| v.get("zh-cn"))
            .and_then(|v| v.as_str())
            .or_else(|| node.attribute("UWPDisplayName"));
        if let Some(name) = name.map(str::trim).filter(|s| !s.is_empty()) {
            return Some(name.chars().take(80).collect());
        }
    }
    None
}

pub(super) fn read_devices() -> Result<Vec<Device>, String> {
    let api = HidApi::new().map_err(|e| format!("无法枚举 ROG 设备：{e}"))?;
    let mut xml = None;
    let mut devices = Vec::new();
    let mut slot_error = None;
    for info in api.device_list() {
        let Some(endpoint) = Endpoint::select(
            info.vendor_id(),
            info.product_id(),
            info.interface_number(),
            info.usage_page(),
            info.usage(),
        ) else {
            continue;
        };
        let power = match endpoint.exchange(&api, info) {
            Ok(power) => power,
            Err(e) => {
                slot_error = Some(e);
                None
            }
        };
        // An empty Omni slot exposes the same HID collection as a paired device.
        // A wired PID, on the other hand, identifies a physically connected device.
        if endpoint.wireless && power.is_none() {
            continue;
        }
        let catalogue = if endpoint.wireless {
            xml.get_or_insert_with(read_catalogue)
                .as_deref()
                .and_then(|s| roxmltree::Document::parse(s).ok())
        } else {
            None
        };
        devices.push(Device {
            id: endpoint.id(info),
            name: endpoint.name(info, catalogue.as_ref()),
            kind: endpoint.slot.kind(),
            transport: if endpoint.wireless {
                Transport::Wireless24
            } else {
                Transport::Usb
            },
            level: power.as_ref().map(|p| p.level),
            charging: power.as_ref().and_then(|p| p.charging),
            threshold: power.and_then(|p| p.threshold),
        });
    }
    match slot_error {
        Some(e) if devices.is_empty() => Err(e),
        _ => Ok(devices),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(header: &[u8]) -> [u8; 64] {
        let mut data = [0; 64];
        data[..header.len()].copy_from_slice(header);
        data
    }

    #[test]
    fn recorded_wireless_and_wired_power_reports() {
        let cases: &[(Slot, bool, &[u8], u8, bool)] = &[
            (
                Slot::Mouse,
                true,
                &[3, 18, 7, 0, 0, 77, 2, 20, 0x6d, 0x0f, 0],
                77,
                false,
            ),
            (
                Slot::Mouse,
                true,
                &[3, 18, 7, 0, 0, 77, 2, 20, 0x1f, 0x10, 1],
                77,
                true,
            ),
            (
                Slot::Mouse,
                false,
                &[18, 7, 0, 0, 76, 2, 20, 0x47, 0x10, 1],
                76,
                true,
            ),
            (
                Slot::Keyboard,
                true,
                &[2, 18, 1, 0, 0, 0, 78, 2, 1, 0, 20, 78],
                78,
                false,
            ),
            (
                Slot::Keyboard,
                true,
                &[2, 18, 1, 0, 0, 0, 78, 2, 1, 1, 20, 78],
                78,
                true,
            ),
            (
                Slot::Keyboard,
                false,
                &[18, 1, 0, 0, 0, 78, 2, 1, 1, 20, 78],
                78,
                true,
            ),
        ];
        for &(slot, wireless, bytes, level, charging) in cases {
            assert_eq!(
                Endpoint { slot, wireless }.decode(&report(bytes)),
                Reply::Battery(Power {
                    level,
                    charging: Some(charging),
                    threshold: Some(20)
                })
            );
        }
    }

    #[test]
    fn unrelated_reports_sleep_and_partial_data_never_become_zero_percent() {
        let mouse = Endpoint {
            slot: Slot::Mouse,
            wireless: true,
        };
        for bytes in [
            &[2, 18, 7, 0, 0, 55][..],
            &[3, 18, 1, 0, 0, 55],
            &[3, 18, 7, 1, 0, 55],
            &[3, 18, 7, 0, 0, 255],
        ] {
            assert_eq!(mouse.decode(&report(bytes)), Reply::Other);
        }
        assert_eq!(mouse.decode(&[3, 18, 7, 0, 0, 55]), Reply::Other);
        assert_eq!(mouse.decode(&report(&[3, 0xff, 0xaa])), Reply::Unavailable);
        assert_eq!(mouse.decode(&report(&[2, 0xff, 0xaa])), Reply::Other);
        assert_eq!(
            mouse.decode(&report(&[3, 18, 7, 0, 0, 0, 2, 0xff, 0, 0, 0xff])),
            Reply::Battery(Power {
                level: 0,
                charging: None,
                threshold: None
            })
        );
        assert_eq!(
            mouse.decode(&report(&[3, 18, 7, 0, 0, 100, 2, 0, 0, 0, 0])),
            Reply::Battery(Power {
                level: 100,
                charging: Some(false),
                threshold: Some(0)
            })
        );
    }

    #[test]
    fn ignores_unrelated_hid_collections_and_other_receivers_names() {
        assert!(Endpoint::select(0x0b05, 0x1ace, 2, 0xff01, 1).is_some());
        for (pid, interface, page) in [(0x1ace, 3, 0xff01), (0x1ace, 2, 1), (0xbeef, 2, 0xff01)] {
            assert!(Endpoint::select(0x0b05, pid, interface, page, 1).is_none());
        }
        let doc = roxmltree::Document::parse(
            r#"<root><Name DeviceType="Mouse" UWPDisplayName="Model">
            <DeviceID>0B051C0E_mouse_serial01</DeviceID>
            <DisplayNameMultiLang>{"zh-cn":"月刃2 RGB"}</DisplayNameMultiLang></Name></root>"#,
        )
        .unwrap();
        assert_eq!(
            armoury_name(&doc, "SERIAL01", Slot::Mouse).as_deref(),
            Some("月刃2 RGB")
        );
        assert!(armoury_name(&doc, "serial02", Slot::Mouse).is_none());
        assert!(armoury_name(&doc, "serial01", Slot::Keyboard).is_none());
    }

    #[test]
    #[ignore = "requires an awake supported ROG mouse or keyboard"]
    fn connected_rog_battery() {
        let devices = read_devices().unwrap();
        assert!(!devices.is_empty(), "no supported active device responded");
        for d in devices {
            println!(
                "{} {:?}, level={:?}, charging={:?}, threshold={:?}",
                d.name, d.kind, d.level, d.charging, d.threshold
            );
            assert!(d.level.is_some_and(|p| p <= 100));
        }
    }
}
