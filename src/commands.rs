use argh::FromArgs;
use serde::{Deserialize, Serialize};

#[derive(FromArgs, Debug, Serialize, Deserialize)]
/// Simple BLE scanner
pub struct Args {
    #[argh(subcommand)]
    pub command: Commands,

    /// dump JSON command object
    #[argh(switch)]
    #[serde(default, skip_serializing)]
    pub dump_json: bool,
}

#[derive(FromArgs, Debug, Serialize, Deserialize)]
#[argh(subcommand)]
pub enum Commands {
    Scan(ScanArgs),
    Enumerate(EnumerateArgs),
    Poll(PollArgs),
    Write(WriteArgs),
    Notify(NotifyArgs),
    Dump(DumpArgs),
    Run(RunArgs),
}

#[derive(FromArgs, Debug, Serialize, Deserialize)]
/// Scan BLE Devices
#[argh(subcommand, name = "scan")]
pub struct ScanArgs {
    /// filter device name [multiple allowed]
    #[argh(option)]
    #[serde(default)]
    pub name: Vec<String>,

    /// filter device uuid [multiple allowed]
    #[argh(option)]
    #[serde(default)]
    pub device: Vec<String>,

    /// minimum RSSI
    #[argh(option)]
    #[serde(default)]
    pub rssi: Option<i16>,

    /// scan timeout
    #[argh(option)]
    #[serde(default)]
    pub timeout: Option<u64>,

    /// NDJSON output
    #[argh(switch)]
    #[serde(default)]
    pub json: bool,
}

#[derive(FromArgs, Debug, Serialize, Deserialize)]
/// Enumerate BLE Devices
#[argh(subcommand, name = "enumerate")]
pub struct EnumerateArgs {
    /// read characteristic data
    #[argh(switch)]
    #[serde(default)]
    pub read: bool,

    /// filter device name [multiple allowed]
    #[argh(option)]
    #[serde(default)]
    pub name: Vec<String>,

    /// filter device uuid [multiple allowed]
    #[argh(option)]
    #[serde(default)]
    pub device: Vec<String>,

    /// filter service uuid [multiple allowed]
    #[argh(option)]
    #[serde(default)]
    pub service: Vec<String>,

    /// filter characteristic uuid [multiple allowed]
    #[argh(option)]
    #[serde(default)]
    pub characteristic: Vec<String>,

    /// minimum RSSI
    #[argh(option)]
    #[serde(default)]
    pub rssi: Option<i16>,

    /// scan timeout
    #[argh(option)]
    #[serde(default)]
    pub timeout: Option<u64>,

    /// decode format <characteristic_uuid::type>
    #[argh(option)]
    #[serde(default)]
    pub decode: Vec<String>,

    /// file containing decode format <characteristic_uuid::type>
    #[argh(option)]
    #[serde(default)]
    pub decode_file: Vec<String>,

    /// NDJSON output
    #[argh(switch)]
    #[serde(default)]
    pub json: bool,

    /// max number of device matches (note: may be exceeded if multiple
    /// matching tasks running in parallel)
    #[argh(option)]
    #[serde(default)]
    pub max: Option<u32>,
}

#[derive(FromArgs, Debug, Serialize, Deserialize)]
/// Read service data continuously
#[argh(subcommand, name = "poll")]
pub struct PollArgs {
    /// read service data continuously (poll interval in s)
    #[argh(option, default = "5.0")]
    #[serde(default)]
    pub interval: f64,

    /// filter device name [multiple allowed]
    #[argh(option)]
    #[serde(default)]
    pub name: Vec<String>,

    /// filter device uuid [multiple allowed]
    #[argh(option)]
    #[serde(default)]
    pub device: Vec<String>,

    /// filter service uuid [multiple allowed]
    #[argh(option)]
    #[serde(default)]
    pub service: Vec<String>,

    /// filter characteristic uuid [multiple allowed]
    #[argh(option)]
    #[serde(default)]
    pub characteristic: Vec<String>,

    /// minimum RSSI
    #[argh(option)]
    #[serde(default)]
    pub rssi: Option<i16>,

    /// timeout
    #[argh(option)]
    #[serde(default)]
    pub timeout: Option<u64>,

    /// decode format <characteristic_uuid::type>
    #[argh(option)]
    #[serde(default)]
    pub decode: Vec<String>,

    /// file containing decode format <characteristic_uuid::type>
    #[argh(option)]
    #[serde(default)]
    pub decode_file: Vec<String>,

    /// NDJSON output
    #[argh(switch)]
    #[serde(default)]
    pub json: bool,
}

#[derive(FromArgs, Debug, Serialize, Deserialize)]
/// Subscribe/listen for notify events
#[argh(subcommand, name = "notify")]
pub struct NotifyArgs {
    /// filter device name [multiple allowed]
    #[argh(option)]
    #[serde(default)]
    pub name: Vec<String>,

    /// filter device uuid [multiple allowed]
    #[argh(option)]
    #[serde(default)]
    pub device: Vec<String>,

    /// filter service uuid [multiple allowed]
    #[argh(option)]
    #[serde(default)]
    pub service: Vec<String>,

    /// filter characteristic uuid [multiple allowed]
    #[argh(option)]
    #[serde(default)]
    pub characteristic: Vec<String>,

    /// minimum RSSI
    #[argh(option)]
    #[serde(default)]
    pub rssi: Option<i16>,

    /// timeout
    #[argh(option)]
    #[serde(default)]
    pub timeout: Option<u64>,

    /// decode format <characteristic_uuid::type>
    #[argh(option)]
    #[serde(default)]
    pub decode: Vec<String>,

    /// file containing decode format <characteristic_uuid::type>
    #[argh(option)]
    #[serde(default)]
    pub decode_file: Vec<String>,

    /// NDJSON output
    #[argh(switch)]
    #[serde(default)]
    pub json: bool,
}

#[derive(FromArgs, Debug, Serialize, Deserialize)]
/// Write characteristic data
#[argh(subcommand, name = "write")]
pub struct WriteArgs {
    /// device name
    #[argh(option)]
    #[serde(default)]
    pub name: Vec<String>,

    /// device uuid
    #[argh(option)]
    #[serde(default)]
    pub device: Vec<String>,

    /// service uuid
    #[argh(option)]
    #[serde(default)]
    pub service: Vec<String>,

    /// write characteristic - characteristic_uuid::data[_type]
    /// (will exit after all characteristics are written - in case of multiple
    /// matching devices this may cause unexpected results. Use the --name/
    /// --device/--service filters to limit device matches).
    /// (Note that this does not handle partial writes)
    #[argh(option)]
    pub characteristic: Vec<String>,

    /// minimum RSSI
    #[argh(option)]
    #[serde(default)]
    pub rssi: Option<i16>,

    /// scan timeout
    #[argh(option)]
    #[serde(default)]
    pub timeout: Option<u64>,

    /// force write without response
    #[argh(switch)]
    #[serde(default)]
    pub without_response: bool,

    /// NDJSON output
    #[argh(switch)]
    #[serde(default)]
    pub json: bool,
}

#[derive(FromArgs, Debug, Serialize, Deserialize)]
/// Dump raw BLE advertisement data
#[argh(subcommand, name = "dump")]
pub struct DumpArgs {
    /// filter event type [multiple allowed]
    #[argh(option)]
    #[serde(default)]
    pub event: Vec<String>,

    /// filter device uuid [multiple allowed]
    #[argh(option)]
    #[serde(default)]
    pub device: Vec<String>,

    /// scan timeout
    #[argh(option)]
    #[serde(default)]
    pub timeout: Option<u64>,

    /// NDJSON output
    #[argh(switch)]
    #[serde(default)]
    pub json: bool,
}

#[derive(FromArgs, Debug, Serialize, Deserialize)]
/// Run JSON command file
#[argh(subcommand, name = "run")]
pub struct RunArgs {
    #[argh(positional)]
    pub path: String,
}
