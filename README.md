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

(Note that UUIDs can be specified as full or appreviated UUIDs - eg: 0x2a24 = 00002a24-0000-1000-8000-00805f9b34fb)

## Dump Event Filters

The `dump` command supports filtering by event type and device id:

- `--device <uuid>`
- `--event DeviceDiscovered`
- `--event DeviceUpdated`
- `--event ManufacturerDataAdvertisement`
- `--event ServiceDataAdvertisement`
- etc.

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

blescan dump --device 464694e8-0a01-005b-95b2-0ae54239625e --event DeviceDiscovered --event ManufacturerDataAdvertisement 
```
