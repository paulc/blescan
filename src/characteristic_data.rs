use std::fmt::{self, Display, Formatter};
use std::num::{ParseFloatError, ParseIntError};
use std::str::FromStr;

use serde_json::Value;

/// Match upto next delimter or end of string - return match/rest
fn match_delimiter<'a>(s: &'a str, delimeter: char) -> (&'a str, Option<&'a str>) {
    match s.split_once(delimeter) {
        Some((left, right)) => (left, Some(right)),
        None => (s, None),
    }
}

/// Match matching braces (allowing nested) - returns match/rest
fn _match_braces<'a>(s: &'a str, start: char, end: char) -> Result<(Option<&'a str>, Option<&'a str>), CharDataError> {
    let mut depth = 0;
    if !s.starts_with(start) {
        return Err(CharDataError::Format("Start delimeter not found".into()));
    }
    for (i, c) in s.char_indices() {
        if c == start {
            depth += 1;
        } else if c == end {
            depth -= 1;
        }
        if depth == 0 {
            let (next, rest) = s.split_at(i + 1);
            return Ok((Some(&next[1..i]), (!rest.is_empty()).then_some(rest)));
        }
    }
    // Unbalanced braces
    return Err(CharDataError::Format("Unbalanced Braces".into()));
}

#[derive(Debug, Clone)]
pub enum CharDataError {
    ParseInt(ParseIntError),
    ParseFloat(ParseFloatError),
    ParseHex(hex::FromHexError),
    Format(String),
}

impl Display for CharDataError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            CharDataError::ParseInt(msg) => write!(f, "Invalid integer: {}", msg),
            CharDataError::ParseFloat(msg) => write!(f, "Invalid float: {}", msg),
            CharDataError::ParseHex(msg) => write!(f, "Invalid hex data: {}", msg),
            CharDataError::Format(msg) => write!(f, "Invalid format: {}", msg),
        }
    }
}

impl std::error::Error for CharDataError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharFormat(Vec<Field>);

impl TryFrom<&str> for CharFormat {
    type Error = CharDataError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Ok(Self(
            s.split(",").map(|f| f.try_into()).collect::<Result<Vec<Field>, _>>()?,
        ))
    }
}

impl Default for CharFormat {
    fn default() -> Self {
        Self(vec![Field::Bytes])
    }
}

impl CharFormat {
    pub fn decode(&self, data: &[u8]) -> anyhow::Result<Value> {
        let mut data = data;
        let mut out = Vec::<Value>::new();
        for f in self.0.iter() {
            if let Some((next, rest)) = f.data_len().and_then(|len| data.split_at_checked(len)) {
                out.push(f.decode(next)?);
                data = rest;
            } else {
                out.push(f.decode(data)?);
            }
        }
        Ok(serde_json::to_value(out)?)
    }
    pub fn parse(&self, s: &str) -> anyhow::Result<CharData> {
        let mut s = s;
        let mut data = CharData::empty();
        for f in self.0.iter() {
            let (d, rest) = CharData::parse(f, s)?;
            data.push(&d);
            if let Some(rest) = rest {
                s = rest;
            } else {
                break;
            }
        }
        Ok(data)
    }
    pub fn fields<'a>(&'a self) -> &'a [Field] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Field {
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
    Bytes,
}

impl TryFrom<&str> for Field {
    type Error = CharDataError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "bool" => Ok(Field::Bool),
            "u8" => Ok(Field::U8),
            "i8" => Ok(Field::I8),
            "u16" => Ok(Field::U16),
            "i16" => Ok(Field::I16),
            "u32" => Ok(Field::U32),
            "i32" => Ok(Field::I32),
            "u64" => Ok(Field::U64),
            "i64" => Ok(Field::I64),
            "f32" => Ok(Field::F32),
            "f64" => Ok(Field::F64),
            "utf8" => Ok(Field::Utf8),
            "bytes" => Ok(Field::Bytes),
            _ => Err(CharDataError::Format("Invalid Format".into())),
        }
    }
}

impl Field {
    pub fn data_len(&self) -> Option<usize> {
        match self {
            Field::Bool | Field::U8 | Field::I8 => Some(1),
            Field::U16 | Field::I16 => Some(2),
            Field::U32 | Field::I32 | Field::F32 => Some(4),
            Field::U64 | Field::I64 | Field::F64 => Some(8),
            _ => None,
        }
    }
    pub fn decode(&self, data: &[u8]) -> anyhow::Result<Value> {
        let v = match self {
            Field::Bool => serde_json::to_value(u8::from_le_bytes(TryInto::<[u8; 1]>::try_into(data)?) != 0)?,
            Field::U8 => serde_json::to_value(u8::from_le_bytes(TryInto::<[u8; 1]>::try_into(data)?))?,
            Field::I8 => serde_json::to_value(i8::from_le_bytes(TryInto::<[u8; 1]>::try_into(data)?))?,
            Field::U16 => serde_json::to_value(u16::from_le_bytes(TryInto::<[u8; 2]>::try_into(data)?))?,
            Field::I16 => serde_json::to_value(i16::from_le_bytes(TryInto::<[u8; 2]>::try_into(data)?))?,
            Field::U32 => serde_json::to_value(u32::from_le_bytes(TryInto::<[u8; 4]>::try_into(data)?))?,
            Field::I32 => serde_json::to_value(i32::from_le_bytes(TryInto::<[u8; 4]>::try_into(data)?))?,
            Field::U64 => serde_json::to_value(u64::from_le_bytes(TryInto::<[u8; 8]>::try_into(data)?))?,
            Field::I64 => serde_json::to_value(i64::from_le_bytes(TryInto::<[u8; 8]>::try_into(data)?))?,
            Field::F32 => serde_json::to_value(f32::from_le_bytes(TryInto::<[u8; 4]>::try_into(data)?))?,
            Field::F64 => serde_json::to_value(f64::from_le_bytes(TryInto::<[u8; 8]>::try_into(data)?))?,
            Field::Utf8 => serde_json::to_value(TryInto::<String>::try_into(data.to_vec())?)?,
            Field::Bytes => serde_json::to_value(TryInto::<String>::try_into(hex::encode(data))?)?,
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
    pub fn empty() -> Self {
        Self(Vec::new())
    }
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
    pub fn push(&mut self, v: &CharData) {
        self.0.extend_from_slice(&v.0);
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
        if let Some(v) = v.strip_prefix("0x") {
            <$int_type>::from_str_radix(v, 16)
        } else {
            v.parse::<$int_type>()
        }
        .map(|v| CharData::from(v))
        .map_err(|e| CharDataError::ParseInt(e))
    }};
}

impl TryFrom<&str> for CharData {
    type Error = CharDataError;
    fn try_from(/* XXX */ _value: &str) -> Result<Self, Self::Error> {
        Ok(CharData::empty())
    }
}

impl CharData {
    /// Parse string value - format is value[::type] (value is hex if type omitted)
    pub fn parse<'a>(fmt: &Field, s: &'a str) -> Result<(CharData, Option<&'a str>), CharDataError> {
        let (data, rest) = match fmt {
            Field::Bool => {
                let (v, rest) = match_delimiter(s, ',');
                match v.to_lowercase().as_str() {
                    "true" => (CharData(vec![1_u8]), rest),
                    "false" => (CharData(vec![0_u8]), rest),
                    _ => Err(CharDataError::Format("Invalid Bool".into()))?,
                }
            }
            Field::U8 => {
                let (v, rest) = match_delimiter(s, ',');
                (parse_int_type!(v, u8)?, rest)
            }
            Field::I8 => {
                let (v, rest) = match_delimiter(s, ',');
                (parse_int_type!(v, i8)?, rest)
            }
            Field::U16 => {
                let (v, rest) = match_delimiter(s, ',');
                (parse_int_type!(v, u16)?, rest)
            }
            Field::I16 => {
                let (v, rest) = match_delimiter(s, ',');
                (parse_int_type!(v, i16)?, rest)
            }
            Field::U32 => {
                let (v, rest) = match_delimiter(s, ',');
                (parse_int_type!(v, u32)?, rest)
            }
            Field::I32 => {
                let (v, rest) = match_delimiter(s, ',');
                (parse_int_type!(v, i32)?, rest)
            }
            Field::U64 => {
                let (v, rest) = match_delimiter(s, ',');
                (parse_int_type!(v, u64)?, rest)
            }
            Field::I64 => {
                let (v, rest) = match_delimiter(s, ',');
                (parse_int_type!(v, i64)?, rest)
            }
            Field::F32 => {
                let (v, rest) = match_delimiter(s, ',');
                (
                    f32::from_str(v)
                        .map(|f| CharData(f.to_le_bytes().to_vec()))
                        .map_err(CharDataError::ParseFloat)?,
                    rest,
                )
            }
            Field::F64 => {
                let (v, rest) = match_delimiter(s, ',');
                (
                    f64::from_str(v)
                        .map(|f| CharData(f.to_le_bytes().to_vec()))
                        .map_err(CharDataError::ParseFloat)?,
                    rest,
                )
            }
            Field::Utf8 => {
                let (v, rest) = match_delimiter(s, ','); // XXX Note: doesn't support embedded ',' - use Bytes
                (CharData(v.as_bytes().to_vec()), rest)
            }
            Field::Bytes => {
                let (v, rest) = match_delimiter(s, ',');
                let data = if let Some(v) = v.strip_prefix("0x") {
                    hex::decode(v).map_err(CharDataError::ParseHex)?
                } else {
                    hex::decode(v).map_err(CharDataError::ParseHex)?
                };
                (CharData(data), rest)
            }
        };
        Ok((data, rest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Field::*;

    #[test]
    fn test_char_format_try_from() {
        for (s, f) in [
            ("u8", CharFormat(vec![U8])),
            ("u8,u16,u32", CharFormat(vec![U8, U16, U32])),
        ] {
            let res = CharFormat::try_from(s);
            // println!("{s} -> {f:?}");
            assert_eq!(res.unwrap(), f);
        }
    }
    #[test]
    fn test_char_format_try_from_err() {
        for s in ["error", "u8,error,u32"] {
            let res = CharFormat::try_from(s);
            // println!("{s} -> {res:?}");
            assert_eq!(res.is_err(), true);
        }
    }
    #[test]
    fn test_char_format_parse() {
        for (s, v, d) in [
            ("u8", "55", vec![55]),
            ("u8,u16,u32", "55,66,77", vec![55, 66, 0, 77, 0, 0, 0]),
            ("u8,utf8", "1,AAAA", vec![1, 65, 65, 65, 65]),
            ("u8,utf8", "1,AAAA\x00\x00", vec![1, 65, 65, 65, 65, 0, 0]),
            ("bytes", "0a0b0c", vec![10, 11, 12]),
            ("bytes", "0x0A0B0C", vec![10, 11, 12]),
            ("bool,bool", "true,false", vec![1, 0]),
        ] {
            let res = CharFormat::try_from(s).unwrap().parse(v);
            // println!("{s} {v} -> {res:?}");
            assert_eq!(res.unwrap().as_slice(), d.as_slice());
        }
    }
    #[test]
    fn test_char_format_decode() {
        for (s, v, d) in [
            ("u8", "[55]", vec![55]),
            ("u8,u16,u32", "[55,66,77]", vec![55, 66, 0, 77, 0, 0, 0]),
            ("u8,utf8", "[1,\"AAAA\"]", vec![1, 65, 65, 65, 65]),
            ("u8,utf8", "[1,\"AAAA\\u0000\\u0000\"]", vec![1, 65, 65, 65, 65, 0, 0]),
            ("bytes", "[\"0a0b0c\"]", vec![10, 11, 12]),
        ] {
            let res = CharFormat::try_from(s).unwrap().decode(d.as_slice()).unwrap();
            // println!("{s} -> {d:?} -> {:?}", res.to_string());
            assert_eq!(res.to_string(), v);
        }
    }
}
