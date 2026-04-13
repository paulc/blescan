use anyhow::anyhow;
use btleplug::api::Manager as _;
use btleplug::platform::Manager;

use argh::FromArgs;

mod char_data;
mod device_info;
mod enumerate;
mod poll;
mod scan;
mod util;
mod write;

use util::parse_uuid;

// Include UUID map
include!(concat!(env!("OUT_DIR"), "/uuid_map.rs"));

pub const ENUMERATE_TIMEOUT: u64 = 5;
pub const CONNECT_TIMEOUT: u64 = 5;
pub const WRITE_TIMEOUT: u64 = 5;
pub const DISCONNECT_TIMEOUT: u64 = 1;

#[derive(FromArgs, Debug)]
/// Simple BLE scanner
struct Args {
    #[argh(subcommand)]
    command: Commands,
}

#[derive(FromArgs, Debug)]
#[argh(subcommand)]
enum Commands {
    Scan(ScanArgs),
    Enumerate(EnumerateArgs),
    Poll(PollArgs),
    Write(WriteArgs),
}

#[derive(FromArgs, Debug)]
/// Scan BLE Devices
#[argh(subcommand, name = "scan")]
struct ScanArgs {
    /// filter device name [multiple allowed]
    #[argh(option)]
    name: Vec<String>,

    /// minimum RSSI
    #[argh(option)]
    rssi: Option<i16>,

    /// scan timeout
    #[argh(option)]
    timeout: Option<u64>,

    /// NDJSON output
    #[argh(switch)]
    json: bool,
}

#[derive(FromArgs, Debug)]
/// Enumerate BLE Devices
#[argh(subcommand, name = "enumerate")]
struct EnumerateArgs {
    /// read characteristic data
    #[argh(switch)]
    read: bool,

    /// filter device name [multiple allowed]
    #[argh(option)]
    name: Vec<String>,

    /// filter device uuid [multiple allowed]
    #[argh(option)]
    device: Vec<String>,

    /// filter service uuid [multiple allowed]
    #[argh(option)]
    service: Vec<String>,

    /// filter characteristic uuid [multiple allowed]
    #[argh(option)]
    characteristic: Vec<String>,

    /// minimum RSSI
    #[argh(option)]
    rssi: Option<i16>,

    /// scan timeout
    #[argh(option)]
    timeout: Option<u64>,

    /// NDJSON output
    #[argh(switch)]
    json: bool,

    /// max number of device matches (note: may be exceeded if multiple
    /// matching tasks running in parallel)
    #[argh(option)]
    max: Option<u32>,
}

#[derive(FromArgs, Debug)]
/// Read service data continuously
#[argh(subcommand, name = "poll")]
struct PollArgs {
    /// read service data continuously (poll interval in s)
    #[argh(option, default = "f64::from(5)")]
    interval: f64,

    /// filter device name [multiple allowed]
    #[argh(option)]
    name: Vec<String>,

    /// filter device uuid [multiple allowed]
    #[argh(option)]
    device: Vec<String>,

    /// filter service uuid [multiple allowed]
    #[argh(option)]
    service: Vec<String>,

    /// filter characteristic uuid [multiple allowed]
    #[argh(option)]
    characteristic: Vec<String>,

    /// minimum RSSI
    #[argh(option)]
    rssi: Option<i16>,

    /// scan timeout
    #[argh(option)]
    timeout: Option<u64>,

    /// NDJSON output
    #[argh(switch)]
    json: bool,
}

#[derive(FromArgs, Debug)]
/// Enumerate BLE Devices
#[argh(subcommand, name = "write")]
struct WriteArgs {
    /// device name
    #[argh(option)]
    name: Option<String>,

    /// device uuid
    #[argh(option)]
    device: Option<String>,

    /// service uuid
    #[argh(option)]
    service: String,

    /// characteristic uuid
    #[argh(option)]
    characteristic: String,

    /// data (hex)
    #[argh(option)]
    data: String,

    /// minimum RSSI
    #[argh(option)]
    rssi: Option<i16>,

    /// scan timeout
    #[argh(option)]
    timeout: Option<u64>,

    /// NDJSON output
    #[argh(switch)]
    json: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Get args
    let args: Args = argh::from_env();

    // Initialise Bluetooth
    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    let central = adapters
        .into_iter()
        .next()
        .ok_or(anyhow!("No Bluetooth adapters found"))?;

    // Run command
    match args.command {
        Commands::Scan(args) => scan::run(central, args).await?,
        Commands::Enumerate(args) => enumerate::run(central, args).await?,
        Commands::Poll(args) => poll::run(central, args).await?,
        Commands::Write(args) => write::run(central, args).await?,
    }

    Ok(())
}
