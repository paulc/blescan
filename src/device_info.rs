use hex;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub rssi: i16,
    pub services: Vec<ServiceInfo>,
}

impl std::fmt::Display for DeviceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{} | {} | {} dBm", self.id, self.name, self.rssi)?;
        for s in &self.services {
            write!(f, "{}", s)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub uuid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(skip_deserializing)]
    pub service_type: Option<&'static str>,
    pub characteristics: Vec<CharacteristicInfo>,
}

impl std::fmt::Display for ServiceInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.characteristics.is_empty() {
            writeln!(f, "    Service: {}", self.uuid,)?;
        } else {
            writeln!(
                f,
                "    Service: {} {}({} characteristics)",
                self.uuid,
                if let Some(t) = self.service_type {
                    format!(" [{}] ", t)
                } else {
                    "".to_string()
                },
                self.characteristics.len()
            )?;
            for c in &self.characteristics {
                write!(f, "{}", c)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacteristicInfo {
    pub uuid: String,
    pub properties: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(skip_deserializing)]
    pub char_type: Option<&'static str>,
    #[serde(serialize_with = "serialize_hex", deserialize_with = "deserialize_hex")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Vec<u8>>,
}

impl std::fmt::Display for CharacteristicInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "    └─ Char: {} {}", self.uuid, self.properties)?;
        if let Some(t) = self.char_type {
            writeln!(f, "       Type: {}", t)?;
        }
        if let Some(ref value) = self.value {
            writeln!(f, "       Value: 0x{}", hex::encode(value))?;
        }
        Ok(())
    }
}

fn serialize_hex<S>(bytes: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match bytes {
        Some(v) => {
            let hex = format!("0x{}", hex::encode(v));
            serializer.serialize_str(&hex)
        }
        None => serializer.serialize_none(),
    }
}

fn deserialize_hex<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    match opt {
        Some(s) => {
            let s = s.strip_prefix("0x").unwrap_or(&s); // Strip 0x
            hex::decode(s).map(Some).map_err(serde::de::Error::custom)
        }
        None => Ok(None),
    }
}
