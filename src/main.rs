use argh::FromArgs;
use btleplug::api::{Central, CharPropFlags, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Manager, PeripheralId};
use std::time::Duration;
use tokio::time;

#[derive(FromArgs)]
/// Simple BLE scanner
struct Args {
    /// scan without connecting/enumerating services (default)
    #[argh(switch, short = 's')]
    scan: bool,

    /// enumerate services for device with given UUID
    #[argh(option, short = 'e')]
    enumerate: Option<String>,

    /// scan and enumerate all devices
    #[argh(switch, short = 'a')]
    enumerate_all: bool,

    /// read data from characteristic UUID (requires --enumerate)
    #[argh(option, short = 'r')]
    read: Option<String>,
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

    if let Some(ref target_uuid) = args.enumerate {
        println!("Connecting to {}...", target_uuid);

        central.start_scan(ScanFilter::default()).await?;
        time::sleep(Duration::from_secs(5)).await;
        central.stop_scan().await?;

        let target_id = PeripheralId::from(uuid::Uuid::parse_str(target_uuid)?);
        let peripheral = central.peripheral(&target_id).await?;

        let properties = peripheral.properties().await?.unwrap_or_default();
        let name = properties
            .local_name
            .unwrap_or_else(|| "Unknown".to_string());
        let rssi = properties.rssi.unwrap_or(0);

        println!("{} | {} | {} dBm", target_uuid, name, rssi);

        if let Some(ref char_uuid) = args.read {
            read_characteristic(&peripheral, char_uuid).await?;
        } else {
            enumerate_services(&peripheral).await;
        }

        return Ok(());
    }

    println!("Starting BLE scan...");
    central.start_scan(ScanFilter::default()).await?;
    time::sleep(Duration::from_secs(5)).await;
    central.stop_scan().await?;

    let peripherals = central.peripherals().await?;
    println!("Found {} device(s)\n", peripherals.len());

    let scan_only = args.scan || !args.enumerate_all;

    for p in &peripherals {
        let properties = p.properties().await?.unwrap_or_default();
        let id = p.id();
        let name = properties
            .local_name
            .clone()
            .unwrap_or_else(|| "Unknown".to_string());
        let rssi = properties.rssi.unwrap_or(0);

        println!("{} | {} | {} dBm", id, name, rssi);

        if scan_only {
            continue;
        }

        enumerate_services(p).await;
        println!();
    }

    Ok(())
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

fn decode_value(uuid: uuid::Uuid, data: &[u8]) -> String {
    if data.is_empty() {
        return "(empty)".to_string();
    }

    let standard_type = get_characteristic_type(uuid);

    // Try to decode based on known types
    if standard_type.contains("uint8") && data.len() == 1 {
        return format!("{} (uint8)", data[0]);
    }
    if standard_type.contains("uint16") && data.len() == 2 {
        let val = u16::from_le_bytes([data[0], data[1]]);
        return format!("{} (uint16 LE)", val);
    }
    if standard_type.contains("uint32") && data.len() == 4 {
        let val = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        return format!("{} (uint32 LE)", val);
    }
    if standard_type.contains("float") && data.len() == 4 {
        let val = f32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        return format!("{} (float32 LE)", val);
    }
    if standard_type.contains("UTF-8") || standard_type.contains("string") {
        if let Ok(s) = std::str::from_utf8(data) {
            return format!("\"{}\" (UTF-8)", s.trim_end_matches('\0'));
        }
    }

    // Fallback to hex
    let hex: String = data.iter().map(|b| format!("{:02x}", b)).collect();
    format!("0x{} ({} bytes)", hex, data.len())
}

async fn read_characteristic(
    p: &btleplug::platform::Peripheral,
    char_uuid_str: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    match time::timeout(Duration::from_secs(5), p.connect()).await {
        Ok(Ok(_)) => {
            match time::timeout(Duration::from_secs(5), p.discover_services()).await {
                Ok(Ok(_)) => {
                    let char_uuid = parse_uuid(char_uuid_str)?;

                    for service in p.services() {
                        for char in &service.characteristics {
                            if char.uuid == char_uuid {
                                let char_type = get_characteristic_type(char_uuid);
                                println!(
                                    "Found characteristic {} in service {}",
                                    char_uuid, service.uuid
                                );
                                println!("Type: {}", char_type);

                                if !char.properties.contains(CharPropFlags::READ) {
                                    println!("Warning: characteristic does not support READ");
                                }

                                match time::timeout(Duration::from_secs(5), p.read(char)).await {
                                    Ok(Ok(data)) => {
                                        println!("Value: {}", decode_value(char_uuid, &data));
                                    }
                                    Ok(Err(e)) => println!("Read failed: {}", e),
                                    Err(_) => println!("Read timed out"),
                                }

                                let _ = time::timeout(Duration::from_secs(3), p.disconnect()).await;
                                return Ok(());
                            }
                        }
                    }
                    println!("Characteristic {} not found", char_uuid);
                }
                Ok(Err(e)) => println!("Service discovery failed: {}", e),
                Err(_) => println!("Service discovery timed out"),
            }
            let _ = time::timeout(Duration::from_secs(3), p.disconnect()).await;
        }
        Ok(Err(e)) => println!("Connection failed: {}", e),
        Err(_) => println!("Connection timed out"),
    }
    Ok(())
}

async fn enumerate_services(p: &btleplug::platform::Peripheral) {
    match time::timeout(Duration::from_secs(5), p.connect()).await {
        Ok(Ok(_)) => {
            match time::timeout(Duration::from_secs(5), p.discover_services()).await {
                Ok(Ok(_)) => {
                    let services = p.services();
                    if services.is_empty() {
                        println!("  (no services discovered)");
                    } else {
                        for service in services {
                            println!(
                                "  Service: {} ({} characteristics)",
                                service.uuid,
                                service.characteristics.len()
                            );
                            for char in &service.characteristics {
                                let props = format_properties(char.properties);
                                let char_type = get_characteristic_type(char.uuid);
                                println!("    └─ Char: {} {}", char.uuid, props);
                                if char_type != "Unknown/Custom" {
                                    println!("        Type: {}", char_type);
                                }
                            }
                        }
                    }
                }
                Ok(Err(e)) => println!("  (service discovery failed: {})", e),
                Err(_) => println!("  (service discovery timed out)"),
            }
            let _ = time::timeout(Duration::from_secs(3), p.disconnect()).await;
        }
        Ok(Err(e)) => println!("  (connection failed: {})", e),
        Err(_) => println!("  (connection timed out)"),
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
