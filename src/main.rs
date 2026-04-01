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
    _scan: bool,

    /// enumerate services for device with given UUID (no scan needed)
    #[argh(option, short = 'e')]
    enumerate: Option<String>,

    /// scan and enumerate all devices
    #[argh(switch, short = 'a')]
    enumerate_all: bool,

    /// read data from characteristic UUID (requires --enumerate)
    #[argh(option, short = 'r')]
    read: Option<String>,

    /// subscribe to notifications from characteristic UUID (requires --enumerate)
    #[argh(option, short = 'n')]
    notify: Option<String>,

    /// listen duration for notifications in seconds (default: 10)
    #[argh(option, default = "10")]
    duration: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Args = argh::from_env();

    // Validate args
    if (args.read.is_some() || args.notify.is_some()) && args.enumerate.is_none() {
        return Err("--read and --notify require --enumerate".into());
    }

    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    let central = adapters
        .into_iter()
        .next()
        .ok_or("No Bluetooth adapter found")?;

    // If enumerating specific device, skip scan and connect directly
    if let Some(ref target_uuid) = args.enumerate {
        // Quick scan to discover the specific device
        central.start_scan(ScanFilter::default()).await?;
        time::sleep(Duration::from_secs(2)).await;
        central.stop_scan().await?;

        println!("Connecting to {}...", target_uuid);

        let target_id = PeripheralId::from(uuid::Uuid::parse_str(target_uuid)?);
        let peripheral = central.peripheral(&target_id).await?;

        let properties = peripheral.properties().await?.unwrap_or_default();
        let name = properties
            .local_name
            .unwrap_or_else(|| "Unknown".to_string());
        let rssi = properties.rssi.unwrap_or(0);

        println!("{} | {} | {} dBm", target_uuid, name, rssi);

        if let Some(ref char_uuid) = args.notify {
            subscribe_notifications(&peripheral, char_uuid, args.duration).await?;
        } else if let Some(ref char_uuid) = args.read {
            read_characteristic(&peripheral, char_uuid).await?;
        } else {
            enumerate_services(&peripheral).await;
        }

        return Ok(());
    }

    // Otherwise scan for devices
    println!("Starting BLE scan...");
    central.start_scan(ScanFilter::default()).await?;
    time::sleep(Duration::from_secs(5)).await;
    central.stop_scan().await?;

    let peripherals = central.peripherals().await?;
    println!("Found {} device(s)\n", peripherals.len());

    for p in &peripherals {
        let properties = p.properties().await?.unwrap_or_default();
        let id = p.id();
        let name = properties
            .local_name
            .clone()
            .unwrap_or_else(|| "Unknown".to_string());
        let rssi = properties.rssi.unwrap_or(0);

        println!("{} | {} | {} dBm", id, name, rssi);

        if args.enumerate_all {
            enumerate_services(p).await;
            println!();
        }
    }

    Ok(())
}

async fn subscribe_notifications(
    p: &btleplug::platform::Peripheral,
    char_uuid_str: &str,
    duration_secs: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    match time::timeout(Duration::from_secs(5), p.connect()).await {
        Ok(Ok(_)) => {
            match time::timeout(Duration::from_secs(5), p.discover_services()).await {
                Ok(Ok(_)) => {
                    let char_uuid = uuid::Uuid::parse_str(char_uuid_str)?;

                    for service in p.services() {
                        for char in &service.characteristics {
                            if char.uuid == char_uuid {
                                println!(
                                    "Found characteristic {} in service {}",
                                    char_uuid, service.uuid
                                );
                                println!("Properties: {:?}", char.properties);

                                if !char.properties.contains(CharPropFlags::NOTIFY) {
                                    println!("Warning: characteristic does not support NOTIFY");
                                }

                                // Subscribe to notifications
                                p.subscribe(char).await?;
                                println!("Subscribed for {} seconds...", duration_secs);

                                // btleplug 0.12 doesn't provide a portable way to receive notification data
                                // so just sleep
                                time::sleep(Duration::from_secs(duration_secs)).await;

                                p.unsubscribe(char).await?;
                                println!("Unsubscribed");
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
                                println!("    └─ Char: {} {:?}", char.uuid, char.properties);
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

async fn read_characteristic(
    p: &btleplug::platform::Peripheral,
    char_uuid_str: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Connect first
    match time::timeout(Duration::from_secs(5), p.connect()).await {
        Ok(Ok(_)) => {
            // Discover services
            match time::timeout(Duration::from_secs(5), p.discover_services()).await {
                Ok(Ok(_)) => {
                    let char_uuid = uuid::Uuid::parse_str(char_uuid_str)?;

                    // Find the characteristic
                    for service in p.services() {
                        for char in service.characteristics {
                            if char.uuid == char_uuid {
                                println!(
                                    "Found characteristic {} in service {}",
                                    char_uuid, service.uuid
                                );

                                // Read the value
                                match time::timeout(Duration::from_secs(5), p.read(&char)).await {
                                    Ok(Ok(data)) => {
                                        println!("Read {} bytes: {:?}", data.len(), data);
                                        // Try to print as string if valid UTF-8
                                        if let Ok(s) = std::str::from_utf8(&data) {
                                            println!("String: {}", s);
                                        }
                                        // Try to print as hex
                                        let hex: String =
                                            data.iter().map(|b| format!("{:02x}", b)).collect();
                                        println!("Hex: 0x{}", hex);
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
