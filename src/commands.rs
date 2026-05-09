use argh::FromArgs;
use serde::{Deserialize, Serialize};

#[derive(FromArgs, Debug, Serialize, Deserialize)]
/// Simple BLE scanner
pub struct Args {
    #[argh(subcommand)]
    pub command: Commands,

    /// dump JSON command object and exit
    #[argh(switch)]
    #[serde(default, skip_serializing)]
    pub dump_json: bool,

    /// update default connect timeout (5s)
    #[argh(option)]
    pub connect_timeout: Option<u64>,

    /// update default enumerate timeout (5s)
    #[argh(option)]
    pub enumerate_timeout: Option<u64>,

    /// update default write timeout (5s)
    #[argh(option)]
    pub write_timeout: Option<u64>,

    /// update default disconnect timeout (2s)
    #[argh(option)]
    pub disconnect_timeout: Option<u64>,

    /// update default max_tasks (10)
    #[argh(option)]
    pub max_tasks: Option<usize>,
}

#[derive(FromArgs, Debug, Serialize, Deserialize)]
#[argh(subcommand)]
#[serde(rename_all = "snake_case")]
pub enum Commands {
    Scan(ScanArgs),
    Enumerate(EnumerateArgs),
    Poll(PollArgs),
    Write(WriteArgs),
    WriteRead(WriteReadArgs),
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

    /// filter device id [multiple allowed]
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

    /// show seen devices
    #[argh(switch)]
    #[serde(default)]
    pub show_seen: bool,

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

    /// filter device id [multiple allowed]
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
    #[argh(option, default = "default_interval()")]
    #[serde(default = "default_interval")]
    pub interval: f64,

    /// filter device name [multiple allowed]
    #[argh(option)]
    #[serde(default)]
    pub name: Vec<String>,

    /// filter device id [multiple allowed]
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

fn default_interval() -> f64 {
    5.0
}

#[derive(FromArgs, Debug, Serialize, Deserialize)]
/// Subscribe/listen for notify events
#[argh(subcommand, name = "notify")]
pub struct NotifyArgs {
    /// filter device name [multiple allowed]
    #[argh(option)]
    #[serde(default)]
    pub name: Vec<String>,

    /// filter device id [multiple allowed]
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

    /// device id
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
/// Write characteristic data and then read response
#[argh(subcommand, name = "write-read")]
pub struct WriteReadArgs {
    /// device name
    #[argh(option)]
    #[serde(default)]
    pub name: Vec<String>,

    /// device id
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

    /// read delay (ms)
    #[argh(option)]
    #[serde(default)]
    pub read_delay: Option<u64>,

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
/// Dump raw BLE advertisement data
#[argh(subcommand, name = "dump")]
pub struct DumpArgs {
    /// filter event type [multiple allowed]
    #[argh(option)]
    #[serde(default)]
    pub event: Vec<String>,

    /// filter device id [multiple allowed]
    #[argh(option)]
    #[serde(default)]
    pub device: Vec<String>,

    /// filter device name [multiple allowed]
    #[argh(option)]
    #[serde(default)]
    pub name: Vec<String>,

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
