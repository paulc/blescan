use std::fmt::{self, Display, Formatter};
use std::num::{ParseFloatError, ParseIntError};
use std::str::FromStr;

use serde_json::Value;

#[derive(Debug, Clone)]
pub enum CharDataError {
    ParseIntError(ParseIntError),
    ParseFloatError(ParseFloatError),
    FormatError(String),
}

impl Display for CharDataError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            CharDataError::ParseIntError(msg) => write!(f, "Invalid integer: {}", msg),
            CharDataError::ParseFloatError(msg) => write!(f, "Invalid float: {}", msg),
            CharDataError::FormatError(msg) => write!(f, "Invalid format: {}", msg),
        }
    }
}

impl std::error::Error for CharDataError {}

#[derive(Debug, Clone)]
pub enum CharFormat {
    Bool,
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,
    F32,
    F64,
    Utf8,
}

impl TryFrom<&str> for CharFormat {
    type Error = CharDataError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "bool" => Ok(CharFormat::Bool),
            "u8" => Ok(CharFormat::U8),
            "i8" => Ok(CharFormat::I8),
            "u16" => Ok(CharFormat::U16),
            "i16" => Ok(CharFormat::I16),
            "u32" => Ok(CharFormat::U32),
            "i32" => Ok(CharFormat::I32),
            "u64" => Ok(CharFormat::U64),
            "i64" => Ok(CharFormat::I64),
            "f32" => Ok(CharFormat::F32),
            "f64" => Ok(CharFormat::F64),
            "utf8" => Ok(CharFormat::Utf8),
            _ => Err(CharDataError::FormatError("Invalid Format".into())),
        }
    }
}

impl CharFormat {
    pub fn decode_value(&self, data: &[u8]) -> anyhow::Result<Value> {
        let v = match self {
            CharFormat::Bool => serde_json::to_value(u8::from_le_bytes(TryInto::<[u8; 1]>::try_into(data)?) != 0)?,
            CharFormat::U8 => serde_json::to_value(u8::from_le_bytes(TryInto::<[u8; 1]>::try_into(data)?))?,
            CharFormat::I8 => serde_json::to_value(i8::from_le_bytes(TryInto::<[u8; 1]>::try_into(data)?))?,
            CharFormat::U16 => serde_json::to_value(u16::from_le_bytes(TryInto::<[u8; 2]>::try_into(data)?))?,
            CharFormat::I16 => serde_json::to_value(i16::from_le_bytes(TryInto::<[u8; 2]>::try_into(data)?))?,
            CharFormat::U32 => serde_json::to_value(u32::from_le_bytes(TryInto::<[u8; 4]>::try_into(data)?))?,
            CharFormat::I32 => serde_json::to_value(i32::from_le_bytes(TryInto::<[u8; 4]>::try_into(data)?))?,
            CharFormat::U64 => serde_json::to_value(u64::from_le_bytes(TryInto::<[u8; 8]>::try_into(data)?))?,
            CharFormat::I64 => serde_json::to_value(i64::from_le_bytes(TryInto::<[u8; 8]>::try_into(data)?))?,
            CharFormat::F32 => serde_json::to_value(f32::from_le_bytes(TryInto::<[u8; 4]>::try_into(data)?))?,
            CharFormat::F64 => serde_json::to_value(f64::from_le_bytes(TryInto::<[u8; 8]>::try_into(data)?))?,
            CharFormat::Utf8 => serde_json::to_value(TryInto::<String>::try_into(data.to_vec())?)?,
        };
        Ok(v)
    }
}

#[derive(Debug, Clone)]
pub struct CharData(Vec<u8>);

impl CharData {
    #[allow(unused)]
    pub fn new(data: &[u8]) -> Self {
        Self(data.to_vec())
    }
    pub fn to_vec(&self) -> &Vec<u8> {
        &self.0
    }
}

/// Macro to create From<_> implementation for int types
macro_rules! chardata_from_numeric {
    ($($num_type:ty),*) => {
        $(
            impl From<$num_type> for CharData {
                fn from(value: $num_type) -> Self {
                    Self(value.to_le_bytes().to_vec())
                }
            }
        )*
    };
}
chardata_from_numeric!(i8, u8, i16, u16, i32, u32, i64, u64, f32, f64);

/// Macro to parse typed int (with optional 0x prefix) into CharData
macro_rules! parse_int_type {
    ($value:expr, $int_type:ty) => {{
        let v = $value.trim();
        if v.starts_with("0x") {
            <$int_type>::from_str_radix(&v[2..], 16)
        } else {
            <$int_type>::from_str_radix(v, 10)
        }
        .map(|v| CharData::from(v))
        .map_err(|e| CharDataError::ParseIntError(e))
    }};
}

/// Parse string value - format is value[_type] (value is hex if type omitted)
impl TryFrom<&str> for CharData {
    type Error = CharDataError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value.split_once("_") {
            Some((v, "bool")) => match v.to_lowercase().as_str() {
                "true" => Ok(CharData(vec![1_u8])),
                "false" => Ok(CharData(vec![0_u8])),
                _ => Err(CharDataError::FormatError("Invalid Bool".into())),
            },
            Some((v, "u8")) => parse_int_type!(v, u8),
            Some((v, "i8")) => parse_int_type!(v, i8),
            Some((v, "u16")) => parse_int_type!(v, u16),
            Some((v, "i16")) => parse_int_type!(v, i16),
            Some((v, "u32")) => parse_int_type!(v, u32),
            Some((v, "i32")) => parse_int_type!(v, i32),
            Some((v, "u64")) => parse_int_type!(v, u64),
            Some((v, "i64")) => parse_int_type!(v, i64),
            Some((v, "f32")) => f32::from_str(v)
                .map(|f| CharData(f.to_le_bytes().to_vec()))
                .map_err(|e| CharDataError::ParseFloatError(e)),
            Some((v, "f64")) => f64::from_str(v)
                .map(|f| CharData(f.to_le_bytes().to_vec()))
                .map_err(|e| CharDataError::ParseFloatError(e)),
            Some((v, "utf8")) => Ok(CharData(v.as_bytes().to_vec())),
            Some(_) => Err(CharDataError::FormatError("Invalid Format".into())),
            None => {
                // No format suffix - assume raw hex data (possibly with 0x prefix)
                let v = value.strip_prefix("0x").unwrap_or(value);
                hex::decode(v)
                    .map(|v| CharData(v))
                    .map_err(|e| CharDataError::FormatError(e.to_string()))
            }
        }
    }
}
