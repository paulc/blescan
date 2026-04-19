use argh::FromArgs;

#[derive(FromArgs, Debug)]
/// Simple BLE scanner
pub struct Args {
    #[argh(subcommand)]
    pub command: Commands,
}

#[derive(FromArgs, Debug)]
#[argh(subcommand)]
pub enum Commands {
    Scan(ScanArgs),
    Enumerate(EnumerateArgs),
    Poll(PollArgs),
    Write(WriteArgs),
    Notify(NotifyArgs),
}

#[derive(FromArgs, Debug)]
/// Scan BLE Devices
#[argh(subcommand, name = "scan")]
pub struct ScanArgs {
    /// filter device name [multiple allowed]
    #[argh(option)]
    pub name: Vec<String>,

    /// filter device uuid [multiple allowed]
    #[argh(option)]
    pub device: Vec<String>,

    /// minimum RSSI
    #[argh(option)]
    pub rssi: Option<i16>,

    /// scan timeout
    #[argh(option)]
    pub timeout: Option<u64>,

    /// NDJSON output
    #[argh(switch)]
    pub json: bool,
}

#[derive(FromArgs, Debug)]
/// Enumerate BLE Devices
#[argh(subcommand, name = "enumerate")]
pub struct EnumerateArgs {
    /// read characteristic data
    #[argh(switch)]
    pub read: bool,

    /// filter device name [multiple allowed]
    #[argh(option)]
    pub name: Vec<String>,

    /// filter device uuid [multiple allowed]
    #[argh(option)]
    pub device: Vec<String>,

    /// filter service uuid [multiple allowed]
    #[argh(option)]
    pub service: Vec<String>,

    /// filter characteristic uuid [multiple allowed]
    #[argh(option)]
    pub characteristic: Vec<String>,

    /// minimum RSSI
    #[argh(option)]
    pub rssi: Option<i16>,

    /// scan timeout
    #[argh(option)]
    pub timeout: Option<u64>,

    /// decode format <characteristic_uuid::type>
    #[argh(option)]
    pub decode: Vec<String>,

    /// NDJSON output
    #[argh(switch)]
    pub json: bool,

    /// max number of device matches (note: may be exceeded if multiple
    /// matching tasks running in parallel)
    #[argh(option)]
    pub max: Option<u32>,
}

#[derive(FromArgs, Debug)]
/// Read service data continuously
#[argh(subcommand, name = "poll")]
pub struct PollArgs {
    /// read service data continuously (poll interval in s)
    #[argh(option, default = "5.0")]
    pub interval: f64,

    /// filter device name [multiple allowed]
    #[argh(option)]
    pub name: Vec<String>,

    /// filter device uuid [multiple allowed]
    #[argh(option)]
    pub device: Vec<String>,

    /// filter service uuid [multiple allowed]
    #[argh(option)]
    pub service: Vec<String>,

    /// filter characteristic uuid [multiple allowed]
    #[argh(option)]
    pub characteristic: Vec<String>,

    /// minimum RSSI
    #[argh(option)]
    pub rssi: Option<i16>,

    /// timeout
    #[argh(option)]
    pub timeout: Option<u64>,

    /// decode format <characteristic_uuid::type>
    #[argh(option)]
    pub decode: Vec<String>,

    /// NDJSON output
    #[argh(switch)]
    pub json: bool,
}

#[derive(FromArgs, Debug)]
/// Subscribe/listen for notify events
#[argh(subcommand, name = "notify")]
pub struct NotifyArgs {
    /// filter device name [multiple allowed]
    #[argh(option)]
    pub name: Vec<String>,

    /// filter device uuid [multiple allowed]
    #[argh(option)]
    pub device: Vec<String>,

    /// filter service uuid [multiple allowed]
    #[argh(option)]
    pub service: Vec<String>,

    /// filter characteristic uuid [multiple allowed]
    #[argh(option)]
    pub characteristic: Vec<String>,

    /// minimum RSSI
    #[argh(option)]
    pub rssi: Option<i16>,

    /// timeout
    #[argh(option)]
    pub timeout: Option<u64>,

    /// decode format <characteristic_uuid::type>
    #[argh(option)]
    pub decode: Vec<String>,

    /// NDJSON output
    #[argh(switch)]
    pub json: bool,
}

#[derive(FromArgs, Debug)]
/// Write characteristic data
#[argh(subcommand, name = "write")]
pub struct WriteArgs {
    /// device name
    #[argh(option)]
    pub name: Vec<String>,

    /// device uuid
    #[argh(option)]
    pub device: Vec<String>,

    /// service uuid
    #[argh(option)]
    pub service: Vec<String>,

    /// write characteristic - format: characteristic_uuid::data[_type]
    #[argh(option)]
    pub write: Vec<String>,

    /// minimum RSSI
    #[argh(option)]
    pub rssi: Option<i16>,

    /// scan timeout
    #[argh(option)]
    pub timeout: Option<u64>,

    /// force write without response
    #[argh(switch)]
    pub without_response: bool,

    /// NDJSON output
    #[argh(switch)]
    pub json: bool,
}
