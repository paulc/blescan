use anyhow::{anyhow, Context};
use btleplug::api::{
    bleuuid::BleUuid, Central, CentralEvent, CharPropFlags, Manager as _, Peripheral as _,
    ScanFilter,
};
use btleplug::platform::{Manager, Peripheral};
use futures::StreamExt;
use hex;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Duration;
use tokio::time::timeout;
use uuid::Uuid;

use argh::FromArgs;

// Include UUID map
include!(concat!(env!("OUT_DIR"), "/uuid_map.rs"));

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeviceInfo {
    id: String,
    name: String,
    rssi: i16,
    services: Vec<ServiceInfo>,
}

impl std::fmt::Display for DeviceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{} | {} | {} dBm", self.id, self.name, self.rssi)?;
        for s in &self.services {
            write!(f, "{}", s)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServiceInfo {
    uuid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(skip_deserializing)]
    service_type: Option<&'static str>,
    characteristics: Vec<CharacteristicInfo>,
}

impl std::fmt::Display for ServiceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.characteristics.is_empty() {
            writeln!(f, "    Service: {}", self.uuid,)?;
        } else {
            writeln!(
                f,
                "    Service: {} {}({} characteristics)",
                self.uuid,
                if let Some(t) = self.service_type {
                    format!(" [{}] ", t)
                } else {
                    "".to_string()
                },
                self.characteristics.len()
            )?;
            for c in &self.characteristics {
                write!(f, "{}", c)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CharacteristicInfo {
    uuid: String,
    properties: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(skip_deserializing)]
    char_type: Option<&'static str>,
    #[serde(serialize_with = "serialize_hex", deserialize_with = "deserialize_hex")]
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<Vec<u8>>,
}

impl std::fmt::Display for CharacteristicInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "    └─ Char: {} {}", self.uuid, self.properties)?;
        if let Some(t) = self.char_type {
            writeln!(f, "       Type: {}", t)?;
        }
        if let Some(ref value) = self.value {
            writeln!(f, "       Value: 0x{}", hex::encode(value))?;
        }
        Ok(())
    }
}

fn serialize_hex<S>(bytes: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match bytes {
        Some(v) => {
            let hex = format!("0x{}", hex::encode(v));
            serializer.serialize_str(&hex)
        }
        None => serializer.serialize_none(),
    }
}

fn deserialize_hex<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    match opt {
        Some(s) => {
            let s = s.strip_prefix("0x").unwrap_or(&s); // Strip 0x
            hex::decode(s).map(Some).map_err(serde::de::Error::custom)
        }
        None => Ok(None),
    }
}

const ENUMERATE_TIMEOUT: u64 = 5;
const CONNECT_TIMEOUT: u64 = 5;
const DISCONNECT_TIMEOUT: u64 = 1;

#[derive(FromArgs)]
/// Simple BLE scanner
struct Args {
    /// scan timeout
    #[argh(option)]
    timeout: Option<u64>,

    /// enumerate services
    #[argh(switch)]
    enumerate: bool,

    /// read service data
    #[argh(switch)]
    read: bool,

    /// read service data continuously (poll interval in s)
    #[argh(option)]
    poll: Option<f64>,

    /// NDJSON output
    #[argh(switch)]
    json: bool,

    /// compact JSON output
    #[argh(switch)]
    compact: bool,

    /// filter device name [multiple allowed]
    #[argh(option)]
    name: Vec<String>,

    /// filter service uuid (needs --enumerate) [multiple allowed]
    #[argh(option)]
    service: Vec<String>,

    /// filter characteristic uuid (needs --service) [multiple allowed]
    #[argh(option)]
    characteristic: Vec<String>,

    /// write characteristic <hex> [multiple allowed - must have the same number of values as characteristics]
    #[argh(option)]
    write: Vec<String>,

    /// minimum RSSI
    #[argh(option)]
    rssi: Option<i16>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Get args
    let args: Args = argh::from_env();

    if (args.read || !args.service.is_empty()) && !args.enumerate {
        anyhow::bail!("--service/--read require --enumerate");
    }

    if !args.characteristic.is_empty() && args.service.is_empty() {
        anyhow::bail!("--characteristic requires --service");
    }

    if !args.write.is_empty() && (args.characteristic.len() != args.write.len()) {
        anyhow::bail!("number of --write values must equal number of --characteristic values");
    }

    if args.compact && !args.json {
        anyhow::bail!("--compact requires --json");
    }

    // Convert service filter to HashSet<Uuid>
    let service_filter = args
        .service
        .iter()
        .map(|s| parse_uuid(s))
        .collect::<Result<HashSet<Uuid>, _>>()
        .context("Error Parsing Service UUID")?;

    // Convert characteristic filter to HashSet<Uuid>
    let characteristic_filter = args
        .characteristic
        .iter()
        .map(|s| parse_uuid(s))
        .collect::<Result<HashSet<Uuid>, _>>()
        .context("Error Parsing Characteristic UUID")?;

    // Convert write values into HashMap<Uuid,Vec<u8>>
    let write_map = characteristic_filter
        .iter()
        .zip(args.write.iter())
        .map(|(uuid, value)| (uuid, hex_to_vec(&value).expect("Error decoding hex value")))
        .collect::<HashMap<_, _>>();

    // Initialise Bluetooth
    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    let central = adapters
        .into_iter()
        .next()
        .ok_or(anyhow!("No Bluetooth adapters found"))?;

    // ScanFilter only checks for services in the Advertisement
    // payload which tend to be very limited (31 bytes max) rather
    // then the full list of GATT services (which need connection)
    central.start_scan(ScanFilter::default()).await?;
    let mut seen = HashSet::new();

    let scan = async {
        let mut events = central.events().await?;
        while let Some(event) = events.next().await {
            // Check if the event is a device discovery
            match event {
                CentralEvent::DeviceDiscovered(id) => {
                    if seen.contains(&id) {
                        continue;
                    }
                    seen.insert(id.clone());
                    match central.peripheral(&id).await {
                        Ok(peripheral) => {
                            // Get basic info first (fast)
                            let device = get_device_info(&peripheral).await?;
                            // Filter by RSSI
                            if let Some(rssi) = args.rssi {
                                if device.rssi < rssi {
                                    continue;
                                }
                            }
                            // Filter by name
                            if !args.name.is_empty() && !args.name.contains(&device.name) {
                                continue;
                            }
                            if let Some(poll_interval) = args.poll {
                                // Poll device
                                let service_filter = service_filter.clone();
                                let characteristic_filter = characteristic_filter.clone();
                                tokio::spawn(async move {
                                    poll_service(
                                        &peripheral,
                                        &device,
                                        &service_filter,
                                        &characteristic_filter,
                                        poll_interval,
                                        args.json,
                                        args.compact,
                                    )
                                    .await?;
                                    Ok::<(), anyhow::Error>(())
                                });
                            } else if args.enumerate {
                                // Spawn enumeration in background so we don't block events
                                let service_filter = service_filter.clone();
                                let characteristic_filter = characteristic_filter.clone();
                                tokio::spawn(async move {
                                    match enumerate_services(
                                        &peripheral,
                                        args.read,
                                        &service_filter,
                                        &characteristic_filter,
                                    )
                                    .await
                                    {
                                        Ok(Some(services)) => {
                                            // Update DeviceInfo with discovered services
                                            let device = DeviceInfo { services, ..device };
                                            print_response(&device, args.json, args.compact)?;
                                        }
                                        Ok(None) => {
                                            // No filter matches
                                        }
                                        Err(e) => eprintln!("Enumeration error: {:?}", e),
                                    }
                                    Ok::<(), anyhow::Error>(())
                                });
                            } else {
                                print_response(&device, args.json, args.compact)?;
                            }
                        }
                        Err(e) => {
                            eprintln!("Error retrieving peripheral: {:?}", e);
                        }
                    }
                }
                _ => {
                    // eprintln!(">> EVENT: {:?}", event);
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    };

    if let Some(t) = args.timeout {
        if !args.json {
            println!("Listening for BLE advertisements: Timeout {t} secs");
        }
        match timeout(Duration::from_secs(t), scan).await {
            Ok(result) => result.map_err(|e| anyhow!("Scan Error: {e}"))?,
            Err(_) => println!("\n[!] Timeout reached. Stopping scan."),
        }
    } else {
        if !args.json {
            println!("Listening for BLE advertisements: Ctrl+C to stop");
        }
        scan.await.map_err(|e| anyhow!("Scan Error: {e}"))?
    }

    Ok(())
}

fn print_response(device: &DeviceInfo, json: bool, compact: bool) -> anyhow::Result<()> {
    if json {
        println!(
            "{}",
            if compact {
                serde_json::to_string(&device)?
            } else {
                serde_json::to_string_pretty(&device)?
            }
        );
    } else {
        print!("[+] Discovered: {}", device);
    }
    Ok(())
}

async fn get_device_info(p: &Peripheral) -> anyhow::Result<DeviceInfo> {
    let properties = p.properties().await?.unwrap_or_default();
    let id = p.id();
    let name = properties
        .local_name
        .clone()
        .unwrap_or_else(|| "Unknown".to_string());
    let rssi = properties.rssi.unwrap_or(0);
    // Read basic service data from the advertisment
    // but this may miss some services (only advertised
    // intermittently)
    let services = properties
        .services
        .iter()
        .map(|uuid| ServiceInfo {
            uuid: uuid.to_short_string(),
            service_type: SERVICE_MAP.get(uuid).map(|v| &**v),
            characteristics: Vec::new(),
        })
        .collect::<Vec<_>>();
    Ok(DeviceInfo {
        id: id.to_string(),
        name: name,
        rssi,
        services,
    })
}

async fn poll_service(
    p: &Peripheral,
    device: &DeviceInfo,
    service_filter: &HashSet<Uuid>,
    characteristic_filter: &HashSet<Uuid>,
    interval: f64,
    json: bool,
    compact: bool,
) -> anyhow::Result<()> {
    match timeout(Duration::from_secs(CONNECT_TIMEOUT), p.connect()).await {
        Ok(Ok(_)) => {
            match timeout(
                Duration::from_secs(ENUMERATE_TIMEOUT),
                p.discover_services(),
            )
            .await
            {
                Ok(Ok(_)) => {
                    let services = if service_filter.is_empty() {
                        p.services()
                    } else {
                        p.services()
                            .into_iter()
                            .filter(|s| service_filter.contains(&s.uuid))
                            .collect::<BTreeSet<_>>()
                    };
                    if !services.is_empty() {
                        let mut ticker = tokio::time::interval(Duration::from_millis(
                            (interval * 1000.0) as u64,
                        ));
                        loop {
                            ticker.tick().await; // First tick returns immediately
                            let mut service_info = Vec::new();
                            for service in &services {
                                let mut chars = Vec::new();
                                for char in &service.characteristics {
                                    if characteristic_filter.is_empty()
                                        || characteristic_filter.contains(&char.uuid)
                                    {
                                        chars.push(CharacteristicInfo {
                                            uuid: char.uuid.to_short_string(),
                                            properties: format_properties(char.properties),
                                            char_type: CHARACTERISTIC_MAP
                                                .get(&char.uuid)
                                                .map(|v| &**v),
                                            value: if char.properties.contains(CharPropFlags::READ)
                                            {
                                                p.read(char).await.ok()
                                            } else {
                                                None
                                            },
                                        });
                                    }
                                }
                                service_info.push(ServiceInfo {
                                    uuid: service.uuid.to_short_string(),
                                    service_type: SERVICE_MAP.get(&service.uuid).map(|v| &**v),
                                    characteristics: chars,
                                });
                            }
                            let device = DeviceInfo {
                                id: device.id.clone(),
                                name: device.name.clone(),
                                rssi: device.rssi, // RSSI doesnt seem to be updated
                                services: service_info.clone(),
                            };
                            print_response(&device, json, compact)?;
                        }
                    }
                }
                _ => {
                    // eprintln!("Service discovery failed/timeout for {}", p.id()),
                }
            }
        }
        _ => {
            // eprintln!("Connect timeout/error for {}", p.id()),
        }
    }
    Ok(())
}

// Distinguish between no services (empty Some<Vec>) and no filter matches (None)
async fn enumerate_services(
    p: &Peripheral,
    read: bool,
    service_filter: &HashSet<Uuid>,
    characteristic_filter: &HashSet<Uuid>,
) -> anyhow::Result<Option<Vec<ServiceInfo>>> {
    let mut service_info = Vec::new();
    match timeout(Duration::from_secs(CONNECT_TIMEOUT), p.connect()).await {
        Ok(Ok(_)) => {
            match timeout(
                Duration::from_secs(ENUMERATE_TIMEOUT),
                p.discover_services(),
            )
            .await
            {
                Ok(Ok(_)) => {
                    let services = if service_filter.is_empty() {
                        p.services()
                    } else {
                        p.services()
                            .into_iter()
                            .filter(|s| service_filter.contains(&s.uuid))
                            .collect::<BTreeSet<_>>()
                    };
                    for service in services {
                        let mut chars = Vec::new();
                        for char in &service.characteristics {
                            if characteristic_filter.is_empty()
                                || characteristic_filter.contains(&char.uuid)
                            {
                                chars.push(CharacteristicInfo {
                                    uuid: char.uuid.to_short_string(),
                                    properties: format_properties(char.properties),
                                    char_type: CHARACTERISTIC_MAP.get(&char.uuid).map(|v| &**v),
                                    value: if read && char.properties.contains(CharPropFlags::READ)
                                    {
                                        p.read(char).await.ok()
                                    } else {
                                        None
                                    },
                                });
                            }
                        }
                        service_info.push(ServiceInfo {
                            uuid: service.uuid.to_short_string(),
                            service_type: SERVICE_MAP.get(&service.uuid).map(|v| &**v),
                            characteristics: chars,
                        });
                    }
                }
                _ => eprintln!("Service discovery failed/timeout for {}", p.id()),
            }
            if let Err(_e) = timeout(Duration::from_secs(DISCONNECT_TIMEOUT), p.disconnect()).await
            {
                // eprintln!("Disconnect timeout/error for {}: {:?}", p.id(), e);
            }
        }
        _ => {
            // eprintln!("Connect timeout/error for {}", p.id())
        }
    }
    if service_info.is_empty() && !service_filter.is_empty() {
        Ok(None)
        /*
            // XXX Dont return service if none of the characteristic filters match
            } else if service_info.iter().all(|s| s.characteristics.is_empty())
                && !characteristic_filter.is_empty()
            {
                Ok(None)
        */
    } else {
        Ok(Some(service_info))
    }
}

fn format_properties(props: CharPropFlags) -> String {
    let mut p = Vec::new();
    if props.contains(CharPropFlags::BROADCAST) {
        p.push("Broadcast");
    }
    if props.contains(CharPropFlags::READ) {
        p.push("Read");
    }
    if props.contains(CharPropFlags::WRITE) {
        p.push("Write");
    }
    if props.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE) {
        p.push("WriteNoResp");
    }
    if props.contains(CharPropFlags::NOTIFY) {
        p.push("Notify");
    }
    if props.contains(CharPropFlags::INDICATE) {
        p.push("Indicate");
    }
    if props.contains(CharPropFlags::AUTHENTICATED_SIGNED_WRITES) {
        p.push("AuthSignedWrite");
    }
    format!("[{}]", p.join(","))
}

fn parse_uuid(s: &str) -> Result<uuid::Uuid, uuid::Error> {
    if s.len() == 4 {
        // 8-bit UUID
        let full = format!("0000{}-0000-1000-8000-00805f9b34fb", s.to_lowercase());
        uuid::Uuid::parse_str(&full)
    } else if s.len() == 6 && s.starts_with("0x") {
        // 8-bit UUID (0x prexfix)
        let s = &s[2..];
        let full = format!("0000{}-0000-1000-8000-00805f9b34fb", s.to_lowercase());
        uuid::Uuid::parse_str(&full)
    } else if s.len() == 8 {
        // 16-bit UUID
        let full = format!("{}-0000-1000-8000-00805f9b34fb", s.to_lowercase());
        uuid::Uuid::parse_str(&full)
    } else if s.len() == 10 && s.starts_with("0x") {
        // 16-bit UUID (0x prefix)
        let s = &s[2..];
        let full = format!("{}-0000-1000-8000-00805f9b34fb", s.to_lowercase());
        uuid::Uuid::parse_str(&full)
    } else {
        uuid::Uuid::parse_str(s)
    }
}

fn hex_to_vec(s: &str) -> Result<Vec<u8>, hex::FromHexError> {
    // Strip "0x" if necessary
    let cleaned = s.strip_prefix("0x").unwrap_or(s);

    // The hex crate automatically ignores whitespace
    hex::decode(cleaned)
}
