use crate::commands::DumpArgs;
use anyhow::anyhow;
use btleplug::api::{Central, CentralEvent, CentralState, ScanFilter, bleuuid::BleUuid};
use btleplug::platform::{Adapter, PeripheralId};
use futures::StreamExt;
use tokio::time::timeout;

use serde::Serialize;
use serde_json::{Value, json};

use std::collections::HashSet;
use std::time::Duration;

use crate::types::DeviceInfo;

#[derive(Serialize)]
struct JsonEvent {
    #[serde(rename = "type")]
    event_type: &'static str,
    data: Value,
}

struct EventWrapper(CentralEvent);

impl EventWrapper {
    pub fn id(&self) -> Option<&PeripheralId> {
        match &self.0 {
            CentralEvent::DeviceDiscovered(id) => Some(id),
            CentralEvent::DeviceUpdated(id) => Some(id),
            CentralEvent::DeviceConnected(id) => Some(id),
            CentralEvent::DeviceDisconnected(id) => Some(id),
            CentralEvent::DeviceServicesModified(id) => Some(id),
            CentralEvent::ManufacturerDataAdvertisement { id, .. } => Some(id),
            CentralEvent::ServiceDataAdvertisement { id, .. } => Some(id),
            CentralEvent::ServicesAdvertisement { id, .. } => Some(id),
            CentralEvent::RssiUpdate { id, .. } => Some(id),
            CentralEvent::StateUpdate(_) => None,
        }
    }
    pub fn event_type(&self) -> &'static str {
        match &self.0 {
            CentralEvent::DeviceDiscovered(_) => "DeviceDiscovered",
            CentralEvent::DeviceUpdated(_) => "DeviceUpdated",
            CentralEvent::DeviceConnected(_) => "DeviceConnected",
            CentralEvent::DeviceDisconnected(_) => "DeviceDisconnected",
            CentralEvent::DeviceServicesModified(_) => "DeviceServicesModified",
            CentralEvent::ManufacturerDataAdvertisement { .. } => "ManufacturerDataAdvertisement",
            CentralEvent::ServiceDataAdvertisement { .. } => "ServiceDataAdvertisement",
            CentralEvent::ServicesAdvertisement { .. } => "ServicesAdvertisement",
            CentralEvent::RssiUpdate { .. } => "RssiUpdate",
            CentralEvent::StateUpdate(_) => "StateUpdate",
        }
    }
    pub fn filter(&self, filter: &EventFilter) -> bool {
        *filter
            == match &self.0 {
                CentralEvent::DeviceDiscovered(_) => EventFilter::DeviceDiscovered,
                CentralEvent::DeviceUpdated(_) => EventFilter::DeviceUpdated,
                CentralEvent::DeviceConnected(_) => EventFilter::DeviceConnected,
                CentralEvent::DeviceDisconnected(_) => EventFilter::DeviceDisconnected,
                CentralEvent::DeviceServicesModified(_) => EventFilter::DeviceServicesModified,
                CentralEvent::ManufacturerDataAdvertisement { .. } => EventFilter::ManufacturerDataAdvertisement,
                CentralEvent::ServiceDataAdvertisement { .. } => EventFilter::ServiceDataAdvertisement,
                CentralEvent::ServicesAdvertisement { .. } => EventFilter::ServicesAdvertisement,
                CentralEvent::RssiUpdate { .. } => EventFilter::RssiUpdate,
                CentralEvent::StateUpdate(_) => EventFilter::StateUpdate,
            }
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum EventFilter {
    DeviceDiscovered,
    DeviceUpdated,
    DeviceConnected,
    DeviceDisconnected,
    DeviceServicesModified,
    ManufacturerDataAdvertisement,
    ServiceDataAdvertisement,
    ServicesAdvertisement,
    RssiUpdate,
    StateUpdate,
}

impl TryFrom<&str> for EventFilter {
    type Error = anyhow::Error;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s.to_lowercase().as_str() {
            "device_discovered" | "devicediscovered" => Ok(EventFilter::DeviceDiscovered),
            "device_updated" | "deviceupdated" => Ok(EventFilter::DeviceUpdated),
            "device_connected" | "deviceconnected" => Ok(EventFilter::DeviceConnected),
            "device_disconnected" | "devicedisconnected" => Ok(EventFilter::DeviceDisconnected),
            "device_services_modified" | "deviceservicesmodified" => Ok(EventFilter::DeviceServicesModified),
            "manufacturer_data_advertisement" | "manufacturerdataadvertisement" => {
                Ok(EventFilter::ManufacturerDataAdvertisement)
            }
            "service_data_advertisement" | "servicedataadvertisement" => Ok(EventFilter::ServiceDataAdvertisement),
            "services_advertisement" | "servicesadvertisement" => Ok(EventFilter::ServicesAdvertisement),
            "rssi_update" | "rssiupdate" => Ok(EventFilter::RssiUpdate),
            "state_update" | "stateupdate" => Ok(EventFilter::StateUpdate),
            _ => Err(anyhow::anyhow!("Invalid event filter")),
        }
    }
}

pub async fn run(central: Adapter, args: DumpArgs) -> anyhow::Result<()> {
    let event_filter = args
        .event
        .iter()
        .map(|s| EventFilter::try_from(s.as_str()))
        .collect::<Result<HashSet<_>, _>>()?;

    central.start_scan(ScanFilter::default()).await?;

    let scan = async {
        let mut events = central.events().await?;
        while let Some(event) = events.next().await {
            let event = EventWrapper(event);
            // Event filter
            if !event_filter.is_empty() && !event_filter.iter().any(|f| event.filter(f)) {
                continue;
            }
            // Device filter
            if !args.device.is_empty() && event.id().is_some_and(|e| !args.device.contains(&e.to_string())) {
                continue;
            }
            match &event {
                EventWrapper(CentralEvent::DeviceDiscovered(id))
                | EventWrapper(CentralEvent::DeviceUpdated(id))
                | EventWrapper(CentralEvent::DeviceConnected(id))
                | EventWrapper(CentralEvent::DeviceDisconnected(id))
                | EventWrapper(CentralEvent::DeviceServicesModified(id))
                | EventWrapper(CentralEvent::ServicesAdvertisement { id, .. })
                | EventWrapper(CentralEvent::RssiUpdate { id, .. }) => match central.peripheral(&id).await {
                    Ok(peripheral) => {
                        let device = DeviceInfo::new(&peripheral).await?;
                        if args.json {
                            let json_event = JsonEvent {
                                event_type: event.event_type(),
                                data: json!({ "device": serde_json::to_value(&device)? }),
                            };
                            println!("{}", serde_json::to_string(&json_event)?);
                        } else {
                            print!("[+] {}: {}", event.event_type(), device);
                        }
                    }
                    Err(e) => {
                        eprintln!("Error retrieving peripheral: {:?}", e);
                    }
                },
                EventWrapper(CentralEvent::ManufacturerDataAdvertisement { id, manufacturer_data }) => {
                    match central.peripheral(&id).await {
                        Ok(peripheral) => {
                            let device = DeviceInfo::new(&peripheral).await?;
                            if args.json {
                                let manufacturer_data: Vec<serde_json::Value> = manufacturer_data
                                    .iter()
                                    .map(|(company_id, data)| {
                                        json!({
                                            "company_id": company_id,
                                            "data": format!("0x{}", hex::encode(data))
                                        })
                                    })
                                    .collect();

                                let json_event = JsonEvent {
                                    event_type: event.event_type(),
                                    data: json!({ "device": serde_json::to_value(&device)?,
                                                  "manufacturer_data": manufacturer_data
                                    }),
                                };
                                println!("{}", serde_json::to_string(&json_event)?);
                            } else {
                                print!("[+] {}: {}", event.event_type(), device);
                                for (company_id, data) in manufacturer_data.iter() {
                                    println!("    └─ CompanyData: {:0X}: 0x{} ", company_id, hex::encode(data));
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Error retrieving peripheral: {:?}", e);
                        }
                    }
                }
                EventWrapper(CentralEvent::ServiceDataAdvertisement { id, service_data }) => {
                    match central.peripheral(&id).await {
                        Ok(peripheral) => {
                            let device = DeviceInfo::new(&peripheral).await?;
                            if args.json {
                                let service_data: Vec<serde_json::Value> = service_data
                                    .iter()
                                    .map(|(uuid, data)| {
                                        json!({
                                            "uuid": uuid.to_short_string(),
                                            "data": format!("0x{}", hex::encode(data))
                                        })
                                    })
                                    .collect();

                                let json_event = JsonEvent {
                                    event_type: event.event_type(),
                                    data: json!({ "device": serde_json::to_value(&device)?,
                                                  "service_data": service_data
                                    }),
                                };
                                println!("{}", serde_json::to_string(&json_event)?);
                            } else {
                                print!("[+] {}: {}", event.event_type(), device);
                                for (uuid, data) in service_data.iter() {
                                    println!("    └─ ServiceData: {}: 0x{} ", uuid, hex::encode(data));
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Error retrieving peripheral: {:?}", e);
                        }
                    }
                }
                EventWrapper(CentralEvent::StateUpdate(state)) => {
                    let state = match state {
                        CentralState::Unknown => "Unknown",
                        CentralState::PoweredOn => "PoweredOn",
                        CentralState::PoweredOff => "PoweredOff",
                    };
                    if args.json {
                        let json_event = JsonEvent {
                            event_type: event.event_type(),
                            data: json!({ "state": state }),
                        };
                        println!("{}", serde_json::to_string(&json_event)?);
                    } else {
                        print!("[+] {}: {}", event.event_type(), state);
                    }
                }
            };
        }
        Ok::<(), anyhow::Error>(())
    };

    if let Some(t) = args.timeout {
        if !args.json {
            println!("Dumping BLE advertisements: Timeout {t} secs");
        }
        match timeout(Duration::from_secs(t), scan).await {
            Ok(result) => result.map_err(|e| anyhow!("Scan Error: {e}"))?,
            Err(_) => println!("\n[!] Timeout reached. Stopping scan."),
        }
    } else {
        if !args.json {
            println!("Dumping BLE advertisements: Ctrl+C to stop");
        }
        scan.await.map_err(|e| anyhow!("Scan Error: {e}"))?
    }

    Ok(())
}
