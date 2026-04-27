use btleplug::api::{Central, CentralEvent, ScanFilter};
use btleplug::platform::{Adapter, Peripheral, PeripheralId};
use futures::Stream;
use futures::stream::StreamExt;
use regex::Regex;

use std::collections::HashSet;
use std::pin::Pin;

use crate::types::DeviceInfo;

/// Wraps BLE event stream and filtering logic
pub struct DeviceScanner {
    central: Adapter,
    events: Pin<Box<dyn Stream<Item = CentralEvent> + Send>>,
    seen: HashSet<PeripheralId>,
    rssi_filter: Option<i16>,
    name_filter: Vec<Regex>,
    device_filter: Vec<String>,
    filter_seen: bool,
}

impl DeviceScanner {
    /// Create scanner
    pub async fn start(
        central: Adapter,
        rssi_filter: Option<i16>,
        name_filter: Vec<Regex>,
        device_filter: Vec<String>,
        filter_seen: bool,
    ) -> anyhow::Result<Self> {
        // ScanFilter only checks for services in the Advertisement payload
        // rather then the full list of GATT services (which need connection)
        central.start_scan(ScanFilter::default()).await?;
        let events = central.events().await?;

        Ok(Self {
            central,
            events: Box::pin(events),
            seen: HashSet::new(),
            rssi_filter,
            name_filter,
            device_filter,
            filter_seen,
        })
    }

    /// Pull the next event and returns it if it passes all device filters
    pub async fn next_match(&mut self) -> anyhow::Result<Option<(Peripheral, DeviceInfo)>> {
        while let Some(event) = self.events.next().await {
            if let CentralEvent::DeviceDiscovered(id) | CentralEvent::DeviceUpdated(id) = event {
                if self.seen.contains(&id) {
                    continue;
                }
                if let Ok(peripheral) = self.central.peripheral(&id).await
                    && let Ok(device) = DeviceInfo::new(&peripheral).await
                    && self
                        .rssi_filter
                        .is_none_or(|rssi_min| device.rssi.is_some_and(|rssi| rssi >= rssi_min))
                    && (self.name_filter.is_empty()
                        || device
                            .name
                            .clone()
                            .is_some_and(|name| self.name_filter.iter().any(|r| r.is_match(&name))))
                    && (self.device_filter.is_empty() || self.device_filter.contains(&device.id))
                {
                    if self.filter_seen {
                        self.seen.insert(id.clone());
                    }
                    return Ok(Some((peripheral, device)));
                }
            }
        }
        Ok(None) // Stream ended
    }
}
