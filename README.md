# blescan

A simple command-line BLE scanner and debugging tool (using the `btleplug` Rust
crate).  Supports human readable text and JSON outputs (for further
filtering/processing).

## Commands

| Command | Description |
|---------|-------------|
| `scan` | List nearby BLE devices |
| `enumerate` | Connect and list services/characteristics |
| `poll` | Continuously read characteristic values |
| `notify` | Subscribe to notification events |
| `write` | Write data to characteristics |
| `write-read` | Write and then read data to characteristics (for protocols using a write-then-read pattern) |
| `dump` | Raw advertisement event stream with optional event filtering |
| `run` | Run command from JSON file |

## Filtering

All discovery commands support:

- `--name <regex>` - Filter by device name (multiple allowed)
- `--device <id>` - Filter by device id (multiple allowed)
- `--service <uuid>` - Filter by service UUID (multiple allowed)
- `--characteristic <uuid>` - Filter by characteristic UUID (multiple allowed)
- `--rssi <dbm>` - Minimum signal strength
- `--timeout <secs>` - Scan duration

Name, Device ID and RSSI filtering are performed on the advertisment data (so are
fast), service/characteristic filters are applied after connection/enumeration.

(Note that UUIDs can be specified as full or abbreviated BLE UUIDs - eg: `0x2a24`
= `00002a24-0000-1000-8000-00805f9b34fb`)

## Dump Event Filters

The `dump` command supports filtering by event type and device id:

- `--device <id>`
- `--event DeviceDiscovered`
- `--event DeviceUpdated`
- `--event ManufacturerDataAdvertisement`
- `--event ServiceDataAdvertisement`
- etc.

## Decoding/Encoding

Characteristic data can be decoded by specifying the data format using the
`--decode <uuid::format>` flag (format can be: bool, utf8, f32/64, u8/16/32/64,
i8/16/32/64). 

The data format can also be specified as a struct of the above formats using 
`struct<u32,u8,...>`.

If no format is matched then the raw hex output is shown.

Multiple `--decode` flags can be provided, for large numbers of definitions
one or more files containing line separated decode definitions can be specified
using `--decode-file <file>` (blank and comment lines allowed).

Characteristic data can be encoded for the `write` command using the format
`--characteristic <uuid::data[_format]>`.

If format is specified (same formats as decode) then the data is encoded 
using this format, if no format is specified data is assumed to be hex bytes.

## Run

The `blescan run <command.json>` sub-command will run the command defined in
the specified json file. The JSON file format is an object containing the
command type and associated cli parameters as an object (empty parameters can
be skipped). e.g.

```
{
  "enumerate": {
    "read": true,
    "name": [],
    "device": [],
    "service": [
      "00000001-7104-4a99-8a78-02108a60f098"
    ],
    "characteristic": [],
    "rssi": null,
    "timeout": 10,
    "decode": [
      "00000002-7104-4a99-8a78-02108a60f098::u32",
      "00000003-7104-4a99-8a78-02108a60f098::utf8",
      "00000005-7104-4a99-8a78-02108a60f098::bool",
      "00000006-7104-4a99-8a78-02108a60f098::f32"
    ],
    "decode_file": [],
    "json": true,
    "max": 1
  }
}
```

It is possible to dump to equivalent JSON for a CLI command using 
`blescan --dump-json <command> [..args]`.

You can pass the json file from stdin using `blescan run -- -`.

## Usage

```
Usage: blescan [--dump-json] [--connect-timeout <connect-timeout>] [--enumerate-timeout <enumerate-timeout>] [--write-timeout <write-timeout>] [--disconnect-timeout <disconnect-timeout>] [--max-tasks <max-tasks>] <command> [<args>]

Simple BLE scanner

Options:
  --dump-json       dump JSON command object and exit
  --connect-timeout update default connect timeout (5s)
  --enumerate-timeout
                    update default enumerate timeout (5s)
  --write-timeout   update default write timeout (5s)
  --disconnect-timeout
                    update default disconnect timeout (2s)
  --max-tasks       update default max_tasks (10)
  --help, help      display usage information

Commands:
  scan              Scan BLE Devices
  enumerate         Enumerate BLE Devices
  poll              Read service data continuously
  write             Write characteristic data
  write-read        Write characteristic data and then read response
  notify            Subscribe/listen for notify events
  dump              Dump raw BLE advertisement data
  run               Run JSON command file

```

```
Usage: blescan scan [--name <name...>] [--device <device...>] [--rssi <rssi>] [--timeout <timeout>] [--show-seen] [--json]

Scan BLE Devices

Options:
  --name            filter device name [multiple allowed]
  --device          filter device id [multiple allowed]
  --rssi            minimum RSSI
  --timeout         scan timeout
  --show-seen       show seen devices
  --json            NDJSON output
  --help, help      display usage information

```

```
Usage: blescan enumerate [--read] [--name <name...>] [--device <device...>] [--service <service...>] [--characteristic <characteristic...>] [--rssi <rssi>] [--timeout <timeout>] [--decode <decode...>] [--decode-file <decode-file...>] [--json] [--max <max>]

Enumerate BLE Devices

Options:
  --read            read characteristic data
  --name            filter device name [multiple allowed]
  --device          filter device id [multiple allowed]
  --service         filter service uuid [multiple allowed]
  --characteristic  filter characteristic uuid [multiple allowed]
  --rssi            minimum RSSI
  --timeout         scan timeout
  --decode          decode format <characteristic_uuid::type>
  --decode-file     file containing decode format <characteristic_uuid::type>
  --json            NDJSON output
  --max             max number of device matches (note: may be exceeded if
                    multiple matching tasks running in parallel)
  --help, help      display usage information

```

```
Usage: blescan poll [--interval <interval>] [--name <name...>] [--device <device...>] [--service <service...>] [--characteristic <characteristic...>] [--rssi <rssi>] [--timeout <timeout>] [--decode <decode...>] [--decode-file <decode-file...>] [--json]

Read service data continuously

Options:
  --interval        read service data continuously (poll interval in s)
  --name            filter device name [multiple allowed]
  --device          filter device id [multiple allowed]
  --service         filter service uuid [multiple allowed]
  --characteristic  filter characteristic uuid [multiple allowed]
  --rssi            minimum RSSI
  --timeout         timeout
  --decode          decode format <characteristic_uuid::type>
  --decode-file     file containing decode format <characteristic_uuid::type>
  --json            NDJSON output
  --help, help      display usage information

```

```
Usage: blescan write [--name <name...>] [--device <device...>] [--service <service...>] [--characteristic <characteristic...>] [--rssi <rssi>] [--timeout <timeout>] [--without-response] [--json]

Write characteristic data

Options:
  --name            device name
  --device          device id
  --service         service uuid
  --characteristic  write characteristic - characteristic_uuid::data[_type]
                    (will exit after all characteristics are written - in case
                    of multiple matching devices this may cause unexpected
                    results. Use the --name/ --device/--service filters to limit
                    device matches). (Note that this does not handle partial
                    writes)
  --rssi            minimum RSSI
  --timeout         scan timeout
  --without-response
                    force write without response
  --json            NDJSON output
  --help, help      display usage information

```

```
Usage: blescan notify [--name <name...>] [--device <device...>] [--service <service...>] [--characteristic <characteristic...>] [--rssi <rssi>] [--timeout <timeout>] [--decode <decode...>] [--decode-file <decode-file...>] [--json]

Subscribe/listen for notify events

Options:
  --name            filter device name [multiple allowed]
  --device          filter device id [multiple allowed]
  --service         filter service uuid [multiple allowed]
  --characteristic  filter characteristic uuid [multiple allowed]
  --rssi            minimum RSSI
  --timeout         timeout
  --decode          decode format <characteristic_uuid::type>
  --decode-file     file containing decode format <characteristic_uuid::type>
  --json            NDJSON output
  --help, help      display usage information

```

```
Usage: blescan dump [--event <event...>] [--device <device...>] [--name <name...>] [--timeout <timeout>] [--json]

Dump raw BLE advertisement data

Options:
  --event           filter event type [multiple allowed]
  --device          filter device id [multiple allowed]
  --name            filter device name [multiple allowed]
  --timeout         scan timeout
  --json            NDJSON output
  --help, help      display usage information

```

```
Usage: blescan run [--] <path>

Run JSON command file

Positional Arguments:
  path

Options:
  --help, help      display usage information

```

## Examples

Scan for devices:
```
blescan scan
```

Scan with filters:
```
blescan scan --name "Sensor" --rssi -70 --timeout 30
```

Enumerate services and read values with JSON output:
```
blescan enumerate --name "MyDevice" --read --json
```

Enumerate and read all devices with matching service and characteristic:
```
blescan enumerate --service 0x180a --characteristic 0x2a24 --read
```

Poll temperature every 5 seconds:
```
blescan poll --name "Thermometer" --service 0x1809 --characteristic 0x2a1c --interval 5
```

Listen for notifications:
```
blescan notify --name "HeartRate" --service 0x180d
```

Write hex data:
```
blescan write --name "LED" --characteristic "0x2a56::ff00"
```

Write with type suffix:
```
blescan write --name "Config" --characteristic "0x2a57::100_u16"
```

Decode values during read:
```
blescan enumerate --read --decode "0x2a1c::i16"
```

Decode struct (decodes org.bluetooth.characteristic.current_time into YYYY,MM,DD,HH,MM,SS):
```
blescan enumerate --read --service 0x1805 --characteristic 0x2a2b --decode '0x2a2b::struct<u16,u8,u8,u8,u8,u8>'

```

Dump raw advertisements as JSON:
```
blescan dump --json
```

Filter dump by event type:
```
blescan dump --device 464694e8-0a01-005b-95b2-0ae54239625e --event DeviceDiscovered --event ManufacturerDataAdvertisement 
```

## Building

```
cargo build --release
```

Executable is `target/release/blescan`

Note that you need Rust 2024 edition (Rust 1.85 stable).

(Build/tested on MacOS - the underlying `btleplug` crate should work on Linux/Windows but I haven't tested this)

