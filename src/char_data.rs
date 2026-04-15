use std::fmt::{self, Display, Formatter};
use std::num::ParseIntError;

pub enum CharFormat {
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    U64,
    I64,
    Utf8,
}

impl TryFrom<&str> for CharFormat {
    type Error = CharDataError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "u8" => Ok(CharFormat::U8),
            "i8" => Ok(CharFormat::I8),
            "u16" => Ok(CharFormat::U16),
            "i16" => Ok(CharFormat::I16),
            "u32" => Ok(CharFormat::U32),
            "i32" => Ok(CharFormat::I32),
            "u64" => Ok(CharFormat::U64),
            "i64" => Ok(CharFormat::I64),
            "utf8" => Ok(CharFormat::Utf8),
            _ => Err(CharDataError::FormatError("Invalid Format".into())),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CharData(Vec<u8>);

impl CharData {
    pub fn to_vec(&self) -> &Vec<u8> {
        &self.0
    }
    pub fn decode(&self, format: CharFormat) -> String {
        match format {
            CharFormat::U8 => TryInto::<u8>::try_into(self).map(|v| format!("{}", v)),
            CharFormat::I8 => TryInto::<i8>::try_into(self).map(|v| format!("{}", v)),
            CharFormat::U16 => TryInto::<u16>::try_into(self).map(|v| format!("{}", v)),
            CharFormat::I16 => TryInto::<i16>::try_into(self).map(|v| format!("{}", v)),
            CharFormat::U32 => TryInto::<u32>::try_into(self).map(|v| format!("{}", v)),
            CharFormat::I32 => TryInto::<i32>::try_into(self).map(|v| format!("{}", v)),
            CharFormat::U64 => TryInto::<u64>::try_into(self).map(|v| format!("{}", v)),
            CharFormat::I64 => TryInto::<i64>::try_into(self).map(|v| format!("{}", v)),
            CharFormat::Utf8 => TryInto::<String>::try_into(self),
        }
        .unwrap_or("<Invalid Format>".into())
    }
}

#[derive(Debug, Clone)]
pub enum CharDataError {
    ParseIntError(ParseIntError),
    FormatError(String),
}

impl Display for CharDataError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            CharDataError::ParseIntError(msg) => write!(f, "Invalid integer: {}", msg),
            CharDataError::FormatError(msg) => write!(f, "Invalid format: {}", msg),
        }
    }
}

impl std::error::Error for CharDataError {}

/// Macro to create From<_> implementation for int types
macro_rules! chardata_from_int {
    ($($int_type:ty),*) => {
        $(
            impl From<$int_type> for CharData {
                fn from(value: $int_type) -> Self {
                    Self(value.to_le_bytes().to_vec())
                }
            }
        )*
    };
}
chardata_from_int!(i8, u8, i16, u16, i32, u32, i64, u64);

/// Macro to create Into<_> implementation for int types
macro_rules! chardata_into_int {
    ($(($int_type:ty, $byte_size:expr)),* $(,)?) => {
        $(
            impl TryInto<$int_type> for &CharData {
                type Error = CharDataError;
                fn try_into(self) -> Result<$int_type, Self::Error> {
                    let array: [u8; $byte_size] = self
                        .0
                        .clone()
                        .try_into()
                        .map_err(|_| CharDataError::FormatError("Invalid length".into()))?;
                    Ok(<$int_type>::from_le_bytes(array))
                }
            }
        )*
    };
}
chardata_into_int!(
    (u8, 1),
    (i8, 1),
    (u16, 2),
    (i16, 2),
    (u32, 4),
    (i32, 4),
    (u64, 8),
    (i64, 8),
);

impl TryInto<String> for &CharData {
    type Error = CharDataError;
    fn try_into(self) -> Result<String, Self::Error> {
        String::from_utf8(self.0.clone()).map_err(|e| CharDataError::FormatError(e.to_string()))
    }
}

/// Macro to parse typed int (with optional 0x prexix) into CharData
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
            Some((v, "u8")) => parse_int_type!(v, u8),
            Some((v, "i8")) => parse_int_type!(v, i8),
            Some((v, "u16")) => parse_int_type!(v, u16),
            Some((v, "i16")) => parse_int_type!(v, i16),
            Some((v, "u32")) => parse_int_type!(v, u32),
            Some((v, "i32")) => parse_int_type!(v, i32),
            Some((v, "u64")) => parse_int_type!(v, u64),
            Some((v, "i64")) => parse_int_type!(v, i64),
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
