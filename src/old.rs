use argh::FromArgs;
use btleplug::api::{Central, CharPropFlags, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Manager, Peripheral, PeripheralId};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;
use tokio::time;

#[derive(Serialize, Deserialize)]
struct DeviceInfo {
    id: String,
    name: String,
    rssi: i16,
    services: Vec<ServiceInfo>,
}

impl fmt::Display for DeviceInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{} | {} | {} dBm", self.id, self.name, self.rssi)?;
        for s in &self.services {
            write!(f, "{}", s)?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct ServiceInfo {
    uuid: String,
    characteristics: Vec<CharacteristicInfo>,
}

impl fmt::Display for ServiceInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "  Service: {} ({} characteristics)",
            self.uuid,
            self.characteristics.len()
        )?;
        for c in &self.characteristics {
            write!(f, "{}", c)?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
struct CharacteristicInfo {
    uuid: String,
    properties: String,
    char_type: String,
    value: Option<String>,
}

impl fmt::Display for CharacteristicInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "    └─ Char: {} {}", self.uuid, self.properties)?;
        if self.char_type != "Unknown/Custom" {
            writeln!(f, "       Type: {}", self.char_type)?;
        }
        if let Some(ref value) = self.value {
            writeln!(f, "       Value: {}", value)?;
        }
        Ok(())
    }
}

#[derive(FromArgs)]
/// Simple BLE scanner
struct Args {
    /// scan without connecting/enumerating services (default)
    #[argh(switch, short = 's')]
    _scan: bool,

    /// enumerate services for device with given UUID
    #[argh(option, short = 'e')]
    enumerate: Option<String>,

    /// scan and enumerate all devices
    #[argh(switch, short = 'a')]
    enumerate_all: bool,

    /// read data from characteristic UUID (requires --enumerate)
    #[argh(option, short = 'r')]
    read: Option<String>,

    /// output as JSON (suppresses info messages)
    #[argh(switch, short = 'j')]
    json: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Args = argh::from_env();

    if args.read.is_some() && args.enumerate.is_none() {
        return Err("--read requires --enumerate".into());
    }

    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    let central = adapters
        .into_iter()
        .next()
        .ok_or("No Bluetooth adapter found")?;

    // Need to scan to initialise BLE stack
    central.start_scan(ScanFilter::default()).await?;
    time::sleep(Duration::from_secs(5)).await;
    central.stop_scan().await?;

    if let Some(ref target_uuid) = args.enumerate {
        // Enumerate target device
        let target_id = PeripheralId::from(uuid::Uuid::parse_str(target_uuid)?);
        let peripheral = central.peripheral(&target_id).await?;

        if let Some(ref char_uuid) = args.read {
            let mut device = get_device_info(&peripheral, false).await?;
            let value = read_characteristic(&peripheral, char_uuid).await?;
            device.services.push(value);
            if args.json {
                println!("{}", serde_json::to_string_pretty(&device)?);
            } else {
                println!("{}", device);
            }
        } else {
            let device = get_device_info(&peripheral, true).await?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&device)?);
            } else {
                println!("{}", device);
            }
        }
    } else {
        // Scan
        let peripherals = central.peripherals().await?;
        let mut devices = Vec::new();
        for p in &peripherals {
            devices.push(get_device_info(p, args.enumerate_all).await?);
        }
        if args.json {
            println!("{}", serde_json::to_string_pretty(&devices)?);
        } else {
            for d in &devices {
                println!("{}", d);
            }
        }
    }

    Ok(())
}

fn decode_value(uuid: uuid::Uuid, data: &[u8]) -> String {
    if data.is_empty() {
        return "(empty)".to_string();
    }

    let standard_type = get_characteristic_type(uuid);

    // Try to decode based on known types
    if standard_type.contains("uint8") && data.len() == 1 {
        return format!("{}", data[0]);
    }
    if standard_type.contains("uint16") && data.len() == 2 {
        let val = u16::from_le_bytes([data[0], data[1]]);
        return format!("{}", val);
    }
    if standard_type.contains("uint32") && data.len() == 4 {
        let val = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        return format!("{}", val);
    }
    if standard_type.contains("float") && data.len() == 4 {
        let val = f32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        return format!("{}", val);
    }
    if standard_type.contains("UTF-8") || standard_type.contains("string") {
        if let Ok(s) = std::str::from_utf8(data) {
            return format!("{}", s.trim_end_matches('\0'));
        }
    }

    // Fallback to hex
    let hex: String = data.iter().map(|b| format!("{:02x}", b)).collect();
    format!("0x{} ({} bytes)", hex, data.len())
}

async fn read_characteristic(
    p: &btleplug::platform::Peripheral,
    uuid_str: &str,
) -> Result<ServiceInfo, Box<dyn std::error::Error>> {
    match time::timeout(Duration::from_secs(5), p.connect()).await {
        Ok(Ok(_)) => match time::timeout(Duration::from_secs(5), p.discover_services()).await {
            Ok(Ok(_)) => {
                let uuid = parse_uuid(uuid_str)?;

                for service in p.services() {
                    for char in &service.characteristics {
                        if char.uuid == uuid {
                            let char_type = get_characteristic_type(uuid);

                            if !char.properties.contains(CharPropFlags::READ) {
                                return Err("Characteristic does not support READ".into());
                            }

                            let value =
                                match time::timeout(Duration::from_secs(5), p.read(char)).await {
                                    Ok(Ok(data)) => Some(decode_value(uuid, &data)),
                                    _ => None,
                                };

                            let _ = time::timeout(Duration::from_secs(3), p.disconnect()).await;
                            return Ok(ServiceInfo {
                                uuid: service.uuid.to_string(),
                                characteristics: vec![CharacteristicInfo {
                                    uuid: uuid.to_string(),
                                    properties: format_properties(char.properties),
                                    char_type: char_type.to_string(),
                                    value,
                                }],
                            });
                        }
                    }
                }
                return Err("Characteristic Not Found".into());
            }
            _ => {
                let _ = time::timeout(Duration::from_secs(3), p.disconnect()).await;
            }
        },
        _ => {}
    }
    Err("Failed to read characteristic".into())
}

async fn get_device_info(
    p: &Peripheral,
    enumerate: bool,
) -> Result<DeviceInfo, Box<dyn std::error::Error>> {
    let properties = p.properties().await?.unwrap_or_default();
    let id = p.id();
    let name = properties
        .local_name
        .clone()
        .unwrap_or_else(|| "Unknown".to_string());
    let rssi = properties.rssi.unwrap_or(0);

    Ok(DeviceInfo {
        id: id.to_string(),
        name: name.clone(),
        rssi,
        services: if enumerate {
            enumerate_services(p).await?
        } else {
            Vec::new()
        },
    })
}

async fn enumerate_services(
    p: &Peripheral,
) -> Result<Vec<ServiceInfo>, Box<dyn std::error::Error>> {
    let mut services = Vec::new();

    match time::timeout(Duration::from_secs(5), p.connect()).await {
        Ok(Ok(_)) => {
            match time::timeout(Duration::from_secs(5), p.discover_services()).await {
                Ok(Ok(_)) => {
                    for service in p.services() {
                        let mut chars = Vec::new();
                        for char in &service.characteristics {
                            let char_type = get_characteristic_type(char.uuid);
                            chars.push(CharacteristicInfo {
                                uuid: char.uuid.to_string(),
                                properties: format_properties(char.properties),
                                char_type: char_type.to_string(),
                                value: None,
                            });
                        }
                        services.push(ServiceInfo {
                            uuid: service.uuid.to_string(),
                            characteristics: chars,
                        });
                    }
                }
                _ => {}
            }
            let _ = time::timeout(Duration::from_secs(3), p.disconnect()).await;
        }
        _ => {}
    }

    Ok(services)
}

fn parse_uuid(s: &str) -> Result<uuid::Uuid, uuid::Error> {
    if s.len() == 4 {
        // 16-bit UUID like "2a19" -> "00002a19-0000-1000-8000-00805f9b34fb"
        let full = format!("0000{}-0000-1000-8000-00805f9b34fb", s.to_lowercase());
        uuid::Uuid::parse_str(&full)
    } else {
        uuid::Uuid::parse_str(s)
    }
}

fn get_characteristic_type(uuid: uuid::Uuid) -> &'static str {
    let uuid_str = uuid.to_string().to_lowercase();
    // Extract the 16-bit portion from 128-bit UUIDs
    let short = if uuid_str.ends_with("-0000-1000-8000-00805f9b34fb") {
        &uuid_str[0..8] // "00002a19"
    } else {
        &uuid_str
    };

    // Standard Bluetooth SIG UUIDs
    match short {
        // Device Information
        "00002a29" => "Manufacturer Name (UTF-8 string)",
        "00002a24" => "Model Number (UTF-8 string)",
        "00002a25" => "Serial Number (UTF-8 string)",
        "00002a27" => "Hardware Revision (UTF-8 string)",
        "00002a26" => "Firmware Revision (UTF-8 string)",
        "00002a28" => "Software Revision (UTF-8 string)",
        "00002a23" => "System ID (uint40+uint24)",

        // Battery
        "00002a19" => "Battery Level (uint8 %)",

        // Generic Access
        "00002a00" => "Device Name (UTF-8 string)",
        "00002a01" => "Appearance (uint16)",
        "00002a04" => "Peripheral Preferred Connection Params (uint16×4)",
        "00002a05" => "Service Changed (uint16+uint16)",

        // Common
        "00002a1c" => "Temperature Measurement (flags+float)",
        "00002a1d" => "Temperature Type (uint8 enum)",
        "00002a1e" => "Intermediate Temperature (flags+float)",
        "00002a21" => "Measurement Interval (uint16)",

        _ => "Unknown/Custom",
    }
}

fn format_properties(props: CharPropFlags) -> String {
    let mut p = Vec::new();
    if props.contains(CharPropFlags::READ) {
        p.push("Read");
    }
    if props.contains(CharPropFlags::WRITE) {
        p.push("Write");
    }
    if props.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE) {
        p.push("WriteNoResponse");
    }
    if props.contains(CharPropFlags::NOTIFY) {
        p.push("Notify");
    }
    if props.contains(CharPropFlags::INDICATE) {
        p.push("Indicate");
    }
    format!("[{}]", p.join(","))
}
