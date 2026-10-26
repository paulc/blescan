use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use btleplug::api::{Central, ScanFilter};
use btleplug::platform::Adapter;
use console::Term;
use futures::StreamExt;
use tokio::join;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};

use crate::commands::MonitorArgs;
use crate::event::{EventInfo, EventWrapper, ManufacturerData};
use crate::types::DeviceInfo;
use crate::util::make_regex_filter;

#[derive(Debug, Clone)]
struct BleDevice {
    pub device: DeviceInfo,
    pub manufacturer_data: BTreeMap<u16, ManufacturerData>,
    pub last_seen: Instant,
}

impl BleDevice {
    pub fn new(device: DeviceInfo) -> Self {
        Self {
            device,
            manufacturer_data: BTreeMap::new(),
            last_seen: Instant::now(),
        }
    }
    pub fn update_device(&mut self, update: &DeviceInfo) {
        // Avoid upodating from empty fields
        if update.name.is_some() {
            self.device.name = update.name.clone();
        }
        if update.rssi.is_some() {
            self.device.rssi = update.rssi;
        }
        update.services.iter().for_each(|(k, v)| {
            self.device.services.insert(k.clone(), v.clone());
        });
        self.last_seen = Instant::now();
    }
}

static DEVICES: LazyLock<Mutex<BTreeMap<String, BleDevice>>> = LazyLock::new(|| Mutex::new(BTreeMap::new()));

pub async fn run(central: Adapter, args: MonitorArgs) -> anyhow::Result<()> {
    let name_filter = make_regex_filter(&args.name)?;

    central.start_scan(ScanFilter::default()).await?;

    let scan = async {
        let mut events = central.events().await?;
        while let Some(event) = events.next().await {
            let event = EventWrapper(event);
            // Device filter
            if !args.device.is_empty() && event.id().is_some_and(|e| !args.device.contains(&e.to_string())) {
                continue;
            }
            let event_info = event.get_event_info(&central).await?;
            // Name filter
            if !name_filter.is_empty()
                && !event_info
                    .get_name()
                    .is_some_and(|name| name_filter.iter().any(|r| r.is_match(name)))
            {
                continue;
            }
            {
                let mut devices = DEVICES.lock().await;
                match event_info {
                    EventInfo::DeviceEvent { device, .. } => match devices.entry(device.id.clone()) {
                        Entry::Occupied(mut entry) => {
                            entry.get_mut().update_device(&device);
                        }
                        Entry::Vacant(entry) => {
                            entry.insert(BleDevice::new(device));
                        }
                    },
                    EventInfo::ManufacturerDataAdvertisement {
                        device,
                        manufacturer_data,
                        ..
                    } => match devices.entry(device.id.clone()) {
                        Entry::Occupied(mut entry) => {
                            let e = entry.get_mut();
                            e.update_device(&device);
                            manufacturer_data.iter().for_each(|md| {
                                e.manufacturer_data.insert(md.manufacturer_id, md.clone());
                            })
                        }
                        Entry::Vacant(entry) => {
                            let mut d = BleDevice::new(device);
                            manufacturer_data.iter().for_each(|md| {
                                d.manufacturer_data.insert(md.manufacturer_id, md.clone());
                            });
                            entry.insert(d);
                        }
                    },
                    EventInfo::ServiceDataAdvertisement { device, .. } => match devices.entry(device.id.clone()) {
                        Entry::Occupied(mut entry) => {
                            let e = entry.get_mut();
                            e.update_device(&device);
                            // XXX We ignore ServiceData
                        }
                        Entry::Vacant(entry) => {
                            entry.insert(BleDevice::new(device));
                        }
                    },

                    EventInfo::StateUpdate { .. } => {}
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    };

    // We run the display loop in a separate task so that we dont block on scan output
    let display = async {
        loop {
            let now = Instant::now();
            let _ = Term::stdout().clear_screen();
            // Only lock mutex to grab active devices
            let active = {
                let devices = DEVICES.lock().await;
                // Only show values seen within args.device_timeout secs
                devices
                    .values()
                    .filter_map(|v| {
                        if (now - v.last_seen).as_secs() < args.device_timeout {
                            Some(v)
                        } else {
                            None
                        }
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            };
            for d in active {
                print!("[+] Device: {}", d.device);
                for ManufacturerData {
                    manufacturer_id,
                    manufacturer_data,
                } in d.manufacturer_data.values()
                {
                    println!(
                        "    └─ ManufacturerData: {:0X}: {} ",
                        manufacturer_id,
                        hex::encode(manufacturer_data)
                    );
                }
            }
            sleep(Duration::from_millis(args.update)).await;
        }
    };

    // Clear screen
    let _ = Term::stdout().clear_screen();

    if let Some(t) = args.timeout {
        if timeout(Duration::from_secs(t), async { join!(scan, display) })
            .await
            .is_err()
        {
            eprintln!("[-] Timeout");
        }
    } else {
        join!(scan, display);
    }
    Ok(())
}
