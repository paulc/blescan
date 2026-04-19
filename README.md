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
| `dump` | Raw advertisement event stream |

## Filtering

All discovery commands support:

- `--name <regex>` - Filter by device name (multiple allowed)
- `--device <uuid>` - Filter by device UUID (multiple allowed)
- `--service <uuid>` - Filter by service UUID
- `--characteristic <uuid>` - Filter by characteristic UUID
- `--rssi <dbm>` - Minimum signal strength
- `--timeout <secs>` - Scan duration

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

Poll temperature every 5 seconds:
```
blescan poll --name "Thermometer" --service 1809 --characteristic 2a1c --interval 5
```

Listen for notifications:
```
blescan notify --name "HeartRate" --service 180d
```

Write hex data:
```
blescan write --name "LED" --write "2a56::ff00"
```

Write with type suffix:
```
blescan write --name "Config" --write "2a57::100_u16"
```

Decode values during read:
```
blescan enumerate --read --decode "2a1c::i16"
```

Dump raw advertisements as JSON:
```
blescan dump --json
```
