# blescan

A simple command-line BLE scanner and debugging tool (using the `btleplug` Rust
crate).  Supports human readable text, JSON outputs (for further
filtering/processing), and a JavsScript scripting mode for custom
processing.

## Commands

| Command | Description |
|---------|-------------|
| `scan` | List nearby BLE devices |
| `monitor` | Monitor nearby BLE devices (tty output, tracks updates)|
| `enumerate` | Connect and list services/characteristics |
| `poll` | Continuously read characteristic values |
| `notify` | Subscribe to notification events |
| `write` | Write data to characteristics |
| `write-read` | Write and then read data to characteristics (for protocols using a write-then-read pattern) |
| `dump` | Raw advertisement event stream with optional event filtering |
| `run` | Run command from JSON file |
| `sse` | HTTP server - run JSON command and stream output as SSE |
| `js` | Run JavaScript script |

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
`--decode <uuid::format>` flag, format can be: bool, f32/64, u8/16/32/64,
i8/16/32/64, utf8, bytes (hex-encoded). 

To decode a struct `format` can be a comma separated list of formats
(utf8/bytes read until the of the data, other formats read the appropriate 
number of bytes). The decoder always returns a list so if there is only 
one struct element this will still be returned as a list.

If no format is matched then the raw hex output is shown.

Multiple `--decode` flags can be provided, for large numbers of definitions
one or more files containing line separated decode definitions can be specified
using `--decode-file <file>` (blank and comment lines allowed).

Characteristic data can be encoded for the `write` command using the format
`--characteristic <uuid::format=data>`.

If format is specified (same formats as decode) then the data is encoded using
this format (fields are comma separated), if no format is specified data is
assumed to be hex bytes.

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

## SSE Server

`blescan sse` runs an HTTP server which accepts a JSON command (same format as
`run`) as a POST to the SSE endpoint (default `/sse`) and streams the command
output back as `text/event-stream`. Each request runs in a separate `blescan`
process (killed when the client disconnects).

```
curl -N -d '{"scan":{"timeout":10}}' http://127.0.0.1:3000/sse
```

Output is always JSON (the `json` parameter is ignored). Events:

- `message` - command output (one JSON object per event)
- `stderr` - stderr line (JSON string)
- `exit` - process exit `{"code":<n>}`

Options:

- `--bind <addr>` - bind address (default `127.0.0.1:3000`)
- `--endpoint <path>` - SSE endpoint (default `/sse`)
- `--allow <cmd,..>` - limit available commands (default: all except `js`/`run`)
- `--index <file>` - serve file as `/` (default: built-in debug page)
- `--html <path:file>` - serve additional html page (multiple allowed)
- `--cors` - send permissive CORS headers
- `--debug` - log requests to stderr

The built-in debug pages (always available at `/debug/index.html`) allow a
command to be built interactively and the output viewed as a table (`monitor`
is shown as an updating device list).

Note that the server has no authentication and allows commands to read server
side `--decode-file` files - bind to localhost (default) or use `--allow` to
limit commands if exposed.

## JavsScript Mode

In JavaScript mode (`blescan js`) a QuickJS scripting environment (using
rquickjs) is available which provides a BLE `scan` function and `device` object
to allow custom processing (JS mode requires the `quickjs` feature - enabled by
default - note that you should not allow untrusted JS scripts and disable this
feature if blescan is remotely accessible). The JS environment supports
execution of script files (`blescan js --file <file>`), literal javascript
(`blescan js --script <script>`), a REPL (`blescan js --repl`), or call JS
functions with an optional JSON argument (`blescan js --call f --arg '{ "a":1
}'`). 

These can be combined, and all share the same JS context so objects defined in
one JS call are available in subsequent ones. This allows you to define utility
functions in a file and then run a REPL with these functions available using
`blescan js --file utility.js --repl`, or call a function defined in a separate
file from the command line using `--call`.

Note that QuickJS doesnt support top-level await (and almost all of the BLE
related functions are async) so in most cases you need to define an async 
functiopn (or IIFE) or handle the promises manually using `.then()`. The 
runtime will attempt to complete pending tasks on exit (can be interrupted
with Ctrl-C).

In REPL more you can use the `--resolve-promise` flag to automatically resolve
promises when they are returned from REPL commands, this is handy for
interactive testing but note that this can block the Ctrl-C handler if you
are in an async loop. 

The `scan({ rssi?, name?|names?, device?|devices?, filter_seen? })` function 
returns an async iterator returning `device`.

The `device` object provides the following methods:

```
    async device.connect()
    async device.disconnect()
    async device.enumerate()
    async device.read([characteristic,..], as_array_buffer?) -> { uuid: value }
        // chars: ["uuid"] or ["uuid::fmt"]; fmt decodes, otherwise hex / ArrayBuffer.
        // (also supports single characteristic which is automatically wrapped as [])
    async device.write(characteristic, value, withoutResponse?)
        //   characteristic: "uuid" or "uuid::fmt"
        //   value: JS value (scalar) or array 
        //          passed through as JSON; without a fmt, the JS shim has already
        //          normalised a hex string / ArrayBuffer / Uint8Array to hex.
    async device.updateRssi()
    async device.snapshot()
        // returns device state (may or may-not have been enumerated)
    async device.subscribe([characteristic,..]) -> [uuid...]
        // chars: ["uuid"] or ["uuid::fmt"]; a fmt registers a decoder for notification data
    async device.unsubscribe([characteristic,..]) -> [uuid...]
        // chars: ["uuid"] or ["uuid::fmt"] (fmt ignored); stops notifications 
    async device.notifications(as_array_buffer?) -> async notifications iterator
    async device.on_notification(cb, asArrayBuffer?) -> stop_fn
        // Call callback with notification data 

```

For examples of the scripting API see exaples section below or the examples in
the `./scripts/` directory
    
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
  scan              Scan BLE Devices (note that this only shoes the first
                    DeviceDiscovered message and does not update discovered
                    devices with DeviceUpdated/ServicesAdvertisement etc)
  monitor           Monitor BLE Devices (TTY output/tracks device updates)
  enumerate         Enumerate BLE Devices
  poll              Read service data continuously
  write             Write characteristic data
  write-read        Write characteristic data and then read response
  notify            Subscribe/listen for notify events
  dump              Dump raw BLE advertisement data
  run               Run JSON command file
  js                Run JS script with BLE `scan(opts)` function installed in
                    globals   `scan({{rssi?, name?|names?, device?|devices?,
                    filter_seen? }})`

```

```
Usage: blescan scan [--name <name...>] [--device <device...>] [--rssi <rssi>] [--timeout <timeout>] [--show-seen] [--json]

Scan BLE Devices (note that this only shoes the first DeviceDiscovered message and does not update discovered devices with DeviceUpdated/ServicesAdvertisement etc)

Options:
  --name            filter device name [multiple allowed]
  --device          filter device id [multiple allowed]
  --rssi            minimum RSSI
  --timeout         scan timeout
  --show-seen       show seen devices
  --json            NDJSON output
  --help, help      display usage information


JSON Params:
    {
      "scan": {
        "name": [],
        "device": [],
        "rssi": null,
        "timeout": null,
        "show_seen": false,
        "json": false
      }
    }
```

```
Usage: blescan monitor [--name <name...>] [--device <device...>] [--rssi <rssi>] [--timeout <timeout>] [--device-timeout <device-timeout>] [--update <update>] [--json]

Monitor BLE Devices (TTY output/tracks device updates)

Options:
  --name            filter device name [multiple allowed]
  --device          filter device id [multiple allowed]
  --rssi            minimum RSSI
  --timeout         scan timeout
  --device-timeout  device timeout
  --update          update (ms)
  --json            NDJSON output
  --help, help      display usage information


JSON Params:
    {
      "monitor": {
        "name": [],
        "device": [],
        "rssi": null,
        "timeout": null,
        "device_timeout": 5,
        "update": 500,
        "json": false
      }
    }
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


JSON Params:
    {
      "enumerate": {
        "read": false,
        "name": [],
        "device": [],
        "service": [],
        "characteristic": [],
        "rssi": null,
        "timeout": null,
        "decode": [],
        "decode_file": [],
        "json": false,
        "max": null
      }
    }
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


JSON Params:
    {
      "poll": {
        "interval": 5.0,
        "name": [],
        "device": [],
        "service": [],
        "characteristic": [],
        "rssi": null,
        "timeout": null,
        "decode": [],
        "decode_file": [],
        "json": false
      }
    }
```

```
Usage: blescan write [--name <name...>] [--device <device...>] [--service <service...>] [--characteristic <characteristic...>] [--rssi <rssi>] [--timeout <timeout>] [--without-response] [--json]

Write characteristic data

Options:
  --name            device name
  --device          device id
  --service         service uuid
  --characteristic  write characteristic - characteristic_uuid::type=data (will
                    exit after all characteristics are written - in case of
                    multiple matching devices this may cause unexpected results.
                    Use the --name/ --device/--service filters to limit device
                    matches). (Note that this does not handle partial writes)
  --rssi            minimum RSSI
  --timeout         scan timeout
  --without-response
                    force write without response
  --json            NDJSON output
  --help, help      display usage information


JSON Params:
    {
      "write": {
        "name": [],
        "device": [],
        "service": [],
        "characteristic": [],
        "rssi": null,
        "timeout": null,
        "without_response": false,
        "json": false
      }
    }
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


JSON Params:
    {
      "notify": {
        "name": [],
        "device": [],
        "service": [],
        "characteristic": [],
        "rssi": null,
        "timeout": null,
        "decode": [],
        "decode_file": [],
        "json": false
      }
    }
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


JSON Params:
    {
      "dump": {
        "event": [],
        "device": [],
        "name": [],
        "timeout": null,
        "json": false
      }
    }
```

```
Usage: blescan run [--] <path>

Run JSON command file

Positional Arguments:
  path

Options:
  --help, help      display usage information


JSON Params:
    Required positional arguments not provided:
        path

    Run blescan --help for more information.
```

```
Usage: blescan js [--file <file...>] [--script <script...>] [--repl] [--call <call...>] [--arg <arg...>] [--resolve-promise]

Run JS script with BLE `scan(opts)` function installed in globals   `scan({rssi?, name?|names?, device?|devices?, filter_seen? })`

Options:
  --file            JS file (multiple allowed)
  --script          JS script literal (multiple allowed)
  --repl            JS REPL
  --call            call JS function
  --arg             call args (json)
  --resolve-promise REPL resolve top-level promises
  --help, help      display usage information


JSON Params:
    {
      "js": {
        "file": [],
        "script": [],
        "repl": false,
        "call": [],
        "arg": [],
        "resolve_promise": false
      }
    }
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

Monitor output with 10s device timeout (interactively updating TTY output)
```
blescan monitoy --device-timeout 10
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
blescan write --name "LED" --characteristic "0x2a56=ff00"
```

Write with type suffix:
```
blescan write --name "Config" --characteristic "0x2a57::u8,u8=10,20"
```

Write-read (use different format for decode)
```
blescan write-read --name "Config" --characteristic "0x2a57::u8,u8=10,20" --decode "0x2a57::u16"
```

Decode values during read:
```
blescan enumerate --read --decode "0x2a1c::i16"
```

Decode struct (decodes org.bluetooth.characteristic.current_time into YYYY,MM,DD,HH,MM,SS):
```
blescan enumerate --read --service 0x1805 --characteristic 0x2a2b --decode '0x2a2b::u16,u8,u8,u8,u8,u8'

```

Dump raw advertisements as JSON:
```
blescan dump --json
```

Filter dump by event type:
```
blescan dump --device 464694e8-0a01-005b-95b2-0ae54239625e --event DeviceDiscovered --event ManufacturerDataAdvertisement 
```

Scan using JS
```
blescan js --script '(async () => { for await (const dev of scan({ names: [], filter_seen: true })) { console.log(await dev.snapshot()) }})()'
```

Enumerate devices using JS callback 
```
blescan js \
    --script 'const cb = async (dev) => { dev.connect().then(async () => { await dev.enumerate(); console.log(JSON.stringify(await dev.snapshot(),null,2)); await dev.disconnect(); })}' \
    --script 'const scanner = async (names) => { for await (const dev of scan({ names, filter_seen: true })) { cb(dev) }}' \
    --call scanner --arg '["INA219","TV"]'
```

Subscribe to NOTIFY characteristic
```
blescan js --file - <<EOM
    const scanner = async (names,cb) => {
        for await (const dev of scan({ names, filter_seen: true })) {
                cb(dev);
        }
    }
    const subscribe_cb = async (dev) => {
        dev.connect().then(async () => {
            await dev.enumerate();
            await dev.subscribe(["00000003-9b04-4347-98ff-57e8f7803509::f32,f32"]);
            await dev.on_notification((n) => console.log(n));
        })
    }
    scanner(["INA219"],subscribe_cb);
EOM
```

Interactively interact with device (using --resolve-promise)
```
blescan js --repl --resolve-promise
>>> const scanner = scan({name:"INA219"})
>>> scanner.next()
>>> const dev = _.value
>>> scanner.close()
>>> dev.connect()
>>> dev.enumerate()
```

## Building

```
cargo build --release
```

Executable is `target/release/blescan`

Note that you need Rust 2024 edition (Rust 1.85 stable).

(Build/tested on MacOS - the underlying `btleplug` crate should work on Linux/Windows but I haven't tested this)

