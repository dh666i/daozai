use super::Reading;

// Bluetooth SIG Battery Level Status (0x2BED), BAS 1.1, section 3.2.
fn level_status(bytes: &[u8]) -> Option<Reading> {
    let (&flags, payload) = bytes.split_first()?;
    let power = u16::from_le_bytes(payload.get(..2)?.try_into().ok()?);
    if power & 1 == 0 {
        return None;
    }
    let offset = 2 + usize::from(flags & 1) * 2;
    let has_level = flags & 2 != 0;
    let has_additional_status = flags & 4 != 0;
    if payload.len() < offset + usize::from(has_level) + usize::from(has_additional_status) {
        return None;
    }
    let level = has_level.then(|| payload[offset]).filter(|v| *v <= 100);
    let charging = match (power >> 5) & 3 {
        1 => Some(true),
        2 | 3 => Some(false),
        _ => None,
    };
    Some(Reading { level, charging })
}

// Legacy Battery Power State (0x2A1A) uses different bit assignments.
fn power_state(bytes: &[u8]) -> Option<bool> {
    let &[state] = bytes else {
        return None;
    };
    if state & 3 == 2 {
        return None;
    } // Battery not present.
    match (state >> 4) & 3 {
        1 | 2 => Some(false), // Not chargeable or not charging.
        3 => Some(true),
        _ if (state >> 2) & 3 == 3 => Some(false), // Actively discharging.
        _ => None,
    }
}

#[cfg(windows)]
pub(super) fn read(address: &str, need_level: bool) -> Option<Reading> {
    use super::wait;
    use std::time::{Duration, Instant};
    use windows::core::GUID;
    use windows::Devices::Bluetooth::GenericAttributeProfile::{
        GattCharacteristicProperties, GattCommunicationStatus,
    };
    use windows::Devices::Bluetooth::{
        BluetoothCacheMode, BluetoothConnectionStatus, BluetoothLEDevice,
    };
    use windows::Devices::Enumeration::DeviceAccessStatus;
    use windows::Storage::Streams::DataReader;

    let uuid =
        |short: u16| GUID::from_u128((u128::from(short) << 96) | 0x0000_1000_8000_00805f9b34fb);
    let deadline = Instant::now() + Duration::from_secs(3);
    let address = u64::from_str_radix(address, 16).ok()?;
    let dev = wait(
        BluetoothLEDevice::FromBluetoothAddressAsync(address).ok()?,
        deadline,
    )?;
    let is_allowed = |status| {
        matches!(
            status,
            Ok(DeviceAccessStatus::Allowed) | Ok(DeviceAccessStatus::Unspecified)
        )
    };
    if dev.ConnectionStatus().ok()? != BluetoothConnectionStatus::Connected
        || !is_allowed(
            dev.DeviceAccessInformation()
                .and_then(|a| a.CurrentStatus()),
        )
    {
        return None;
    }
    let services = wait(
        dev.GetGattServicesForUuidWithCacheModeAsync(uuid(0x180f), BluetoothCacheMode::Cached)
            .ok()?,
        deadline,
    )?;
    if services.Status().ok()? != GattCommunicationStatus::Success {
        return None;
    }
    let services = services.Services().ok()?;
    // Multiple instances can represent separate earbuds or a case. Use the Windows device
    // reading for these until each battery can be labelled by its presentation descriptor.
    if services.Size().ok()? != 1 {
        return None;
    }
    let service = services.GetAt(0).ok()?;
    if !is_allowed(
        service
            .DeviceAccessInformation()
            .and_then(|a| a.CurrentStatus()),
    ) {
        return None;
    }
    let result = wait(
        service
            .GetCharacteristicsWithCacheModeAsync(BluetoothCacheMode::Cached)
            .ok()?,
        deadline,
    )?;
    if result.Status().ok()? != GattCommunicationStatus::Success {
        return None;
    }
    let chars = result.Characteristics().ok()?;
    let read_value = |short| -> Option<Vec<u8>> {
        if Instant::now() >= deadline
            || dev.ConnectionStatus().ok()? != BluetoothConnectionStatus::Connected
        {
            return None;
        }
        let c = chars
            .clone()
            .into_iter()
            .find(|c| c.Uuid().ok() == Some(uuid(short)))?;
        if !c
            .CharacteristicProperties()
            .ok()?
            .contains(GattCharacteristicProperties::Read)
        {
            return None;
        }
        let result = wait(
            c.ReadValueWithCacheModeAsync(BluetoothCacheMode::Uncached)
                .ok()?,
            deadline,
        )?;
        if result.Status().ok()? != GattCommunicationStatus::Success {
            return None;
        }
        let reader = DataReader::FromBuffer(&result.Value().ok()?).ok()?;
        let len = reader.UnconsumedBufferLength().ok()? as usize;
        if len == 0 || len > 32 {
            return None;
        }
        let mut bytes = vec![0; len];
        reader.ReadBytes(&mut bytes).ok()?;
        Some(bytes)
    };
    let mut reading = read_value(0x2bed)
        .as_deref()
        .and_then(level_status)
        .unwrap_or_default();
    if need_level && reading.level.is_none() {
        reading.level = read_value(0x2a19).and_then(|b| match b.as_slice() {
            &[level @ 0..=100] => Some(level),
            _ => None,
        });
    }
    if reading.charging.is_none() {
        reading.charging = read_value(0x2a1a).as_deref().and_then(power_state);
    }
    Some(reading)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_status_flags_control_offset_and_unknown_values() {
        let charging = Reading {
            level: Some(44),
            charging: Some(true),
        };
        assert_eq!(level_status(&[2, 0x21, 0, 44]), Some(charging));
        assert_eq!(level_status(&[7, 0x21, 0, 52, 18, 44, 0]), Some(charging));
        assert_eq!(
            level_status(&[0, 0x41, 0]),
            Some(Reading {
                level: None,
                charging: Some(false)
            })
        );
        assert_eq!(level_status(&[2, 1, 0, 255]), Some(Reading::default()));
        assert_eq!(level_status(&[2, 0x21]), None);
        assert_eq!(level_status(&[7, 0x21, 0, 44]), None);
        assert_eq!(level_status(&[2, 0, 0, 80]), None);
    }

    #[test]
    fn legacy_charging_bits_are_distinct_from_modern_battery_status() {
        assert_eq!(power_state(&[0x33]), Some(true));
        assert_eq!(power_state(&[0x23]), Some(false));
        assert_eq!(power_state(&[0x0f]), Some(false));
        assert_eq!(power_state(&[0x03]), None);
        assert_eq!(power_state(&[0x32]), None);
        assert_eq!(power_state(&[]), None);
    }
}
