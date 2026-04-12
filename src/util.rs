use btleplug::api::CharPropFlags;

use hex;

pub fn format_properties(props: CharPropFlags) -> String {
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
        // 8-bit UUID
        let full = format!("0000{}-0000-1000-8000-00805f9b34fb", s.to_lowercase());
        uuid::Uuid::parse_str(&full)
    } else if s.len() == 6 && s.starts_with("0x") {
        // 8-bit UUID (0x prexfix)
        let s = &s[2..];
        let full = format!("0000{}-0000-1000-8000-00805f9b34fb", s.to_lowercase());
        uuid::Uuid::parse_str(&full)
    } else if s.len() == 8 {
        // 16-bit UUID
        let full = format!("{}-0000-1000-8000-00805f9b34fb", s.to_lowercase());
        uuid::Uuid::parse_str(&full)
    } else if s.len() == 10 && s.starts_with("0x") {
        // 16-bit UUID (0x prefix)
        let s = &s[2..];
        let full = format!("{}-0000-1000-8000-00805f9b34fb", s.to_lowercase());
        uuid::Uuid::parse_str(&full)
    } else {
        uuid::Uuid::parse_str(s)
    }
}

pub fn hex_to_vec(s: &str) -> Result<Vec<u8>, hex::FromHexError> {
    // Strip "0x" if necessary
    let cleaned = s.strip_prefix("0x").unwrap_or(s);

    // The hex crate automatically ignores whitespace
    hex::decode(cleaned)
}
