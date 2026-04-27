use crate::types::DeviceInfo;
use btleplug::api::{Central, CentralEvent, CentralState, bleuuid::BleUuid};
use btleplug::platform::{Adapter, PeripheralId};
use serde::Serialize;
use uuid::Uuid;

use crate::util::serialize_hex;

pub struct EventWrapper(pub CentralEvent);

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
    pub async fn get_event_info(&self, central: &Adapter) -> anyhow::Result<EventInfo> {
        match self {
            EventWrapper(CentralEvent::DeviceDiscovered(id))
            | EventWrapper(CentralEvent::DeviceUpdated(id))
            | EventWrapper(CentralEvent::DeviceConnected(id))
            | EventWrapper(CentralEvent::DeviceDisconnected(id))
            | EventWrapper(CentralEvent::DeviceServicesModified(id))
            | EventWrapper(CentralEvent::ServicesAdvertisement { id, .. })
            | EventWrapper(CentralEvent::RssiUpdate { id, .. }) => {
                let peripheral = central.peripheral(id).await?;
                let event_type = self.event_type();
                let device = DeviceInfo::new(&peripheral).await?;
                Ok(EventInfo::DeviceEvent { event_type, device })
            }
            EventWrapper(CentralEvent::ManufacturerDataAdvertisement { id, manufacturer_data }) => {
                let peripheral = central.peripheral(id).await?;
                let event_type = self.event_type();
                let device = DeviceInfo::new(&peripheral).await?;
                let manufacturer_data = manufacturer_data
                    .iter()
                    .map(|(k, v)| ManufacturerData {
                        manufacturer_id: *k,
                        manufacturer_data: v.clone(),
                    })
                    .collect::<Vec<_>>();
                Ok(EventInfo::ManufacturerDataAdvertisement {
                    event_type,
                    device,
                    manufacturer_data,
                })
            }
            EventWrapper(CentralEvent::ServiceDataAdvertisement { id, service_data }) => {
                let peripheral = central.peripheral(id).await?;
                let event_type = self.event_type();
                let device = DeviceInfo::new(&peripheral).await?;
                let service_data = service_data
                    .iter()
                    .map(|(k, v)| ServiceData {
                        service_id: *k,
                        service_data: v.clone(),
                    })
                    .collect::<Vec<_>>();
                Ok(EventInfo::ServiceDataAdvertisement {
                    event_type,
                    device,
                    service_data,
                })
            }
            EventWrapper(CentralEvent::StateUpdate(state)) => {
                let event_type = self.event_type();
                let state = match state {
                    CentralState::Unknown => "Unknown",
                    CentralState::PoweredOn => "PoweredOn",
                    CentralState::PoweredOff => "PoweredOff",
                };
                Ok(EventInfo::StateUpdate { event_type, state })
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum EventInfo {
    DeviceEvent {
        event_type: &'static str,
        device: DeviceInfo,
    },
    ManufacturerDataAdvertisement {
        event_type: &'static str,
        device: DeviceInfo,
        manufacturer_data: Vec<ManufacturerData>,
    },
    ServiceDataAdvertisement {
        event_type: &'static str,
        device: DeviceInfo,
        service_data: Vec<ServiceData>,
    },
    StateUpdate {
        event_type: &'static str,
        state: &'static str,
    },
}

impl EventInfo {
    pub fn get_name(&self) -> Option<&str> {
        match self {
            EventInfo::DeviceEvent { device, .. }
            | EventInfo::ManufacturerDataAdvertisement { device, .. }
            | EventInfo::ServiceDataAdvertisement { device, .. } => device.name.as_deref(),
            EventInfo::StateUpdate { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceData {
    service_id: Uuid,
    #[serde(serialize_with = "serialize_hex")]
    service_data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ManufacturerData {
    manufacturer_id: u16,
    #[serde(serialize_with = "serialize_hex")]
    manufacturer_data: Vec<u8>,
}

impl std::fmt::Display for EventInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventInfo::DeviceEvent { event_type, device } => write!(f, "{}: {}", event_type, device)?,
            EventInfo::ManufacturerDataAdvertisement {
                event_type,
                device,
                manufacturer_data,
            } => {
                write!(f, "{}: {}", event_type, device)?;
                for ManufacturerData {
                    manufacturer_id,
                    manufacturer_data,
                } in manufacturer_data.iter()
                {
                    writeln!(
                        f,
                        "    └─ ManufacturerData: {:0X}: {} ",
                        manufacturer_id,
                        hex::encode(manufacturer_data)
                    )?
                }
            }
            EventInfo::ServiceDataAdvertisement {
                event_type,
                device,
                service_data,
            } => {
                write!(f, "{}: {}", event_type, device)?;
                for ServiceData {
                    service_id,
                    service_data,
                } in service_data.iter()
                {
                    writeln!(
                        f,
                        "    └─ ServiceData: {}: {} ",
                        service_id.to_short_string(),
                        hex::encode(service_data)
                    )?
                }
            }
            EventInfo::StateUpdate { event_type, state } => {
                write!(f, "{}: {}", event_type, state)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum EventFilter {
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
