use anyhow::Context;
use btleplug::api::CharPropFlags;
use uuid::Uuid;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::characteristic_data::{CharData, CharFormat};

pub fn format_properties(props: &CharPropFlags) -> String {
    let mut p = Vec::new();
    if props.contains(CharPropFlags::BROADCAST) {
        p.push("Broadcast");
    }
    if props.contains(CharPropFlags::READ) {
        p.push("Read");
    }
    if props.contains(CharPropFlags::WRITE) {
        p.push("Write");
    }
    if props.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE) {
        p.push("WriteNoResp");
    }
    if props.contains(CharPropFlags::NOTIFY) {
        p.push("Notify");
    }
    if props.contains(CharPropFlags::INDICATE) {
        p.push("Indicate");
    }
    if props.contains(CharPropFlags::AUTHENTICATED_SIGNED_WRITES) {
        p.push("AuthSignedWrite");
    }
    format!("[{}]", p.join(","))
}

pub fn parse_uuid(s: &str) -> Result<uuid::Uuid, uuid::Error> {
    if s.len() == 4 {
        // 16-bit UUID
        let full = format!("0000{}-0000-1000-8000-00805f9b34fb", s.to_lowercase());
        uuid::Uuid::parse_str(&full)
    } else if s.len() == 6 && s.starts_with("0x") {
        // 16-bit UUID (0x prexfix)
        let s = &s[2..];
        let full = format!("0000{}-0000-1000-8000-00805f9b34fb", s.to_lowercase());
        uuid::Uuid::parse_str(&full)
    } else if s.len() == 8 {
        // 32-bit UUID
        let full = format!("{}-0000-1000-8000-00805f9b34fb", s.to_lowercase());
        uuid::Uuid::parse_str(&full)
    } else if s.len() == 10 && s.starts_with("0x") {
        // 32-bit UUID (0x prefix)
        let s = &s[2..];
        let full = format!("{}-0000-1000-8000-00805f9b34fb", s.to_lowercase());
        uuid::Uuid::parse_str(&full)
    } else {
        uuid::Uuid::parse_str(s)
    }
}

pub fn uuid_filter(filters: &Vec<String>) -> anyhow::Result<Arc<HashSet<Uuid>>> {
    Ok(Arc::new(
        filters
            .iter()
            .map(|s| parse_uuid(s))
            .collect::<Result<HashSet<Uuid>, _>>()
            .context("Error Parsing UUID")?,
    ))
}

pub fn parse_decoder(decoders: &Vec<String>) -> anyhow::Result<Arc<HashMap<Uuid, CharFormat>>> {
    Ok(Arc::new(
        decoders
            .iter()
            .map(|s| {
                s.split_once("::").context("Invalid Format").and_then(|(uuid, fmt)| {
                    let uuid = parse_uuid(uuid)?;
                    let fmt = CharFormat::try_from(fmt)?;
                    Ok((uuid, fmt))
                })
            })
            .collect::<Result<HashMap<_, _>, _>>()
            .context("Error Parsing Decode Mapping")?,
    ))
}

pub fn parse_write(characteristics: &Vec<String>) -> anyhow::Result<Arc<HashMap<Uuid, Vec<u8>>>> {
    Ok(Arc::new(
        characteristics
            .iter()
            .map(|s| {
                s.split_once("::").context("Invalid Format").and_then(|(uuid, data)| {
                    let uuid = parse_uuid(uuid)?;
                    let data = CharData::try_from(data)?.to_vec().clone();
                    Ok((uuid, data))
                })
            })
            .collect::<Result<HashMap<_, _>, _>>()
            .context("Error Parsing Write Data")?,
    ))
}
