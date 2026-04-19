# blescan

A simple BLE scanner and debugging tool (using the `btleplug` Rust crate).

## Commands

| Command | Description |
|---------|-------------|
| `scan` | List nearby BLE devices |
| `enumerate` | Connect and list services/characteristics |
| `poll` | Continuously read characteristic values |
| `notify` | Subscribe to notification events |
| `write` | Write data to characteristics |
| `dump` | Raw advertisement event stream with optional event filtering |

## Filtering

All discovery commands support:

- `--name <regex>` - Filter by device name (multiple allowed)
- `--device <uuid>` - Filter by device UUID (multiple allowed)
- `--service <uuid>` - Filter by service UUID
- `--characteristic <uuid>` - Filter by characteristic UUID
- `--rssi <dbm>` - Minimum signal strength
- `--timeout <secs>` - Scan duration

(Note that UUIDs can be specified as full or appreviated UUIDs - eg: `0x2a24` = `00002a24-0000-1000-8000-00805f9b34fb`)

## Dump Event Filters

The `dump` command supports filtering by event type and device id:

- `--device <uuid>`
- `--event DeviceDiscovered`
- `--event DeviceUpdated`
- `--event ManufacturerDataAdvertisement`
- `--event ServiceDataAdvertisement`
- etc.

## Usage

```
Usage: blescan scan [--name <name...>] [--device <device...>] [--rssi <rssi>] [--timeout <timeout>] [--json]

Scan BLE Devices

Options:
  --name            filter device name [multiple allowed]
  --device          filter device uuid [multiple allowed]
  --rssi            minimum RSSI
  --timeout         scan timeout
  --json            NDJSON output
  --help, help      display usage information

```

```
Usage: blescan enumerate [--read] [--name <name...>] [--device <device...>] [--service <service...>] [--characteristic <characteristic...>] [--rssi <rssi>] [--timeout <timeout>] [--decode <decode...>] [--json] [--max <max>]

Enumerate BLE Devices

Options:
  --read            read characteristic data
  --name            filter device name [multiple allowed]
  --device          filter device uuid [multiple allowed]
  --service         filter service uuid [multiple allowed]
  --characteristic  filter characteristic uuid [multiple allowed]
  --rssi            minimum RSSI
  --timeout         scan timeout
  --decode          decode format <characteristic_uuid::type>
  --json            NDJSON output
  --max             max number of device matches (note: may be exceeded if
                    multiple matching tasks running in parallel)
  --help, help      display usage information
```

```
Usage: blescan poll [--interval <interval>] [--name <name...>] [--device <device...>] [--service <service...>] [--characteristic <characteristic...>] [--rssi <rssi>] [--timeout <timeout>] [--decode <decode...>] [--json]

Read service data continuously

Options:
  --interval        read service data continuously (poll interval in s)
  --name            filter device name [multiple allowed]
  --device          filter device uuid [multiple allowed]
  --service         filter service uuid [multiple allowed]
  --characteristic  filter characteristic uuid [multiple allowed]
  --rssi            minimum RSSI
  --timeout         timeout
  --decode          decode format <characteristic_uuid::type>
  --json            NDJSON output
  --help, help      display usage information
```

```
Usage: blescan notify [--name <name...>] [--device <device...>] [--service <service...>] [--characteristic <characteristic...>] [--rssi <rssi>] [--timeout <timeout>] [--decode <decode...>] [--json]

Subscribe/listen for notify events

Options:
  --name            filter device name [multiple allowed]
  --device          filter device uuid [multiple allowed]
  --service         filter service uuid [multiple allowed]
  --characteristic  filter characteristic uuid [multiple allowed]
  --rssi            minimum RSSI
  --timeout         timeout
  --decode          decode format <characteristic_uuid::type>
  --json            NDJSON output
  --help, help      display usage information
```

```
Usage: blescan write [--name <name...>] [--device <device...>] [--service <service...>] [--write <write...>] [--rssi <rssi>] [--timeout <timeout>] [--without-response] [--json]

Write characteristic data

Options:
  --name            device name
  --device          device uuid
  --service         service uuid
  --write           write characteristic - characteristic_uuid::data[_type]
                    (will exit after all characteristics are written - in case
                    of multiple matching devices this may cause unexpected
                    results. Use the --name/ --device/--service filters to limit
                    device matches)
  --rssi            minimum RSSI
  --timeout         scan timeout
  --without-response
                    force write without response
  --json            NDJSON output
  --help, help      display usage information
```

```
Usage: blescan dump [--event <event...>] [--device <device...>] [--timeout <timeout>] [--json]

Dump raw BLE advertisement data

Options:
  --event           filter event type [multiple allowed]
  --device          filter device uuid [multiple allowed]
  --timeout         scan timeout
  --json            NDJSON output
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

Enumerate services and read values:
```
blescan enumerate --name "MyDevice" --read
```

Enumerate and read all devices with matching services and characteristic:
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
blescan write --name "LED" --write "0x2a56::ff00"
```

Write with type suffix:
```
blescan write --name "Config" --write "0x2a57::100_u16"
```

Decode values during read:
```
blescan enumerate --read --decode "0x2a1c::i16"
```

Dump raw advertisements as JSON:
```
blescan dump --json
```

Filter dump by event type:
```
blescan dump --device 464694e8-0a01-005b-95b2-0ae54239625e --event DeviceDiscovered --event ManufacturerDataAdvertisement 
```
