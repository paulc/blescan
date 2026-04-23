use crate::filter::device_match;
use crate::types::DeviceInfo;
use btleplug::api::{Central, CentralEvent, ScanFilter};
use btleplug::platform::{Adapter, Peripheral, PeripheralId};
use futures::Stream;
use futures::stream::StreamExt;
use regex::Regex;
use std::collections::HashSet;
use std::pin::Pin;

/// Wraps the boilerplate BLE event stream and filtering logic
pub struct DeviceScanner {
    central: Adapter,
    events: Pin<Box<dyn Stream<Item = CentralEvent> + Send>>,
    seen: HashSet<PeripheralId>,
    rssi_filter: Option<i16>,
    name_filter: Vec<Regex>,
    device_filter: Vec<String>,
}

impl DeviceScanner {
    pub async fn start(
        central: Adapter,
        rssi_filter: Option<i16>,
        name_filter: Vec<Regex>,
        device_filter: Vec<String>,
    ) -> anyhow::Result<Self> {
        central.start_scan(ScanFilter::default()).await?;
        let events = central.events().await?;

        Ok(Self {
            central,
            events: Box::pin(events),
            seen: HashSet::new(),
            rssi_filter,
            name_filter,
            device_filter,
        })
    }

    /// Pulls the next event and returns it if it passes all device filters
    pub async fn next_match(&mut self) -> anyhow::Result<Option<(Peripheral, DeviceInfo)>> {
        while let Some(event) = self.events.next().await {
            if let CentralEvent::DeviceDiscovered(id) = event {
                if self.seen.contains(&id) {
                    continue;
                }
                self.seen.insert(id.clone());
                if let Ok(peripheral) = self.central.peripheral(&id).await {
                    if let Ok(device) = DeviceInfo::new(&peripheral).await {
                        if device_match(&device, &self.rssi_filter, &self.name_filter, &self.device_filter) {
                            return Ok(Some((peripheral, device)));
                        }
                    }
                }
            }
        }
        Ok(None) // Stream ended
    }
}
