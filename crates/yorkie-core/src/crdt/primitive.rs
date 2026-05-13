use super::element::CrdtElementMeta;
use crate::json::escape_json_string;
use crate::{Result, TimeTicket, YorkieError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrimitiveType {
    Null,
    Boolean,
    Integer,
    Long,
    Double,
    String,
    Bytes,
    Date,
}

impl PrimitiveType {
    fn name(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Boolean => "boolean",
            Self::Integer => "integer",
            Self::Long => "long",
            Self::Double => "double",
            Self::String => "string",
            Self::Bytes => "bytes",
            Self::Date => "date",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PrimitiveValue {
    Null,
    Boolean(bool),
    Integer(i32),
    Long(i64),
    Double(f64),
    String(String),
    Bytes(Vec<u8>),
    Date(i64),
}

impl PrimitiveValue {
    fn primitive_type(&self) -> PrimitiveType {
        match self {
            Self::Null => PrimitiveType::Null,
            Self::Boolean(_) => PrimitiveType::Boolean,
            Self::Integer(_) => PrimitiveType::Integer,
            Self::Long(_) => PrimitiveType::Long,
            Self::Double(_) => PrimitiveType::Double,
            Self::String(_) => PrimitiveType::String,
            Self::Bytes(_) => PrimitiveType::Bytes,
            Self::Date(_) => PrimitiveType::Date,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DataSize {
    pub(crate) data: usize,
    pub(crate) meta: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CrdtPrimitive {
    meta: CrdtElementMeta,
    value_type: PrimitiveType,
    value: PrimitiveValue,
}

impl CrdtPrimitive {
    pub(crate) fn new(value: PrimitiveValue, created_at: TimeTicket) -> Self {
        Self {
            value_type: value.primitive_type(),
            value,
            meta: CrdtElementMeta::new(created_at),
        }
    }

    pub(crate) fn value_from_bytes(
        primitive_type: PrimitiveType,
        bytes: &[u8],
    ) -> Result<PrimitiveValue> {
        match primitive_type {
            PrimitiveType::Null => Ok(PrimitiveValue::Null),
            PrimitiveType::Boolean => Ok(PrimitiveValue::Boolean(
                bytes.first().copied().unwrap_or(0) != 0,
            )),
            PrimitiveType::Integer => {
                let bytes = fixed_bytes::<4>(primitive_type, bytes)?;
                Ok(PrimitiveValue::Integer(i32::from_le_bytes(bytes)))
            }
            PrimitiveType::Long => {
                let bytes = fixed_bytes::<8>(primitive_type, bytes)?;
                Ok(PrimitiveValue::Long(i64::from_le_bytes(bytes)))
            }
            PrimitiveType::Double => {
                let bytes = fixed_bytes::<8>(primitive_type, bytes)?;
                Ok(PrimitiveValue::Double(f64::from_le_bytes(bytes)))
            }
            PrimitiveType::String => String::from_utf8(bytes.to_vec())
                .map(PrimitiveValue::String)
                .map_err(|_| YorkieError::InvalidPrimitiveUtf8),
            PrimitiveType::Bytes => Ok(PrimitiveValue::Bytes(bytes.to_vec())),
            PrimitiveType::Date => {
                let bytes = fixed_bytes::<8>(primitive_type, bytes)?;
                Ok(PrimitiveValue::Date(u64::from_le_bytes(bytes) as i64))
            }
        }
    }

    pub(crate) fn created_at(&self) -> &TimeTicket {
        self.meta.created_at()
    }

    pub(crate) fn moved_at(&self) -> Option<&TimeTicket> {
        self.meta.moved_at()
    }

    pub(crate) fn removed_at(&self) -> Option<&TimeTicket> {
        self.meta.removed_at()
    }

    pub(crate) fn set_moved_at(&mut self, moved_at: Option<TimeTicket>) -> bool {
        self.meta.set_moved_at(moved_at)
    }

    pub(crate) fn set_removed_at(&mut self, removed_at: Option<TimeTicket>) {
        self.meta.set_removed_at(removed_at);
    }

    pub(crate) fn data_size(&self) -> DataSize {
        DataSize {
            data: self.value_size(),
            meta: self.meta.meta_usage(),
        }
    }

    pub(crate) fn to_json(&self) -> String {
        match &self.value {
            PrimitiveValue::Null => "null".to_owned(),
            PrimitiveValue::Boolean(value) => value.to_string(),
            PrimitiveValue::Integer(value) => value.to_string(),
            PrimitiveValue::Long(value) => value.to_string(),
            PrimitiveValue::Double(value) => double_to_json(*value),
            PrimitiveValue::String(value) => format!("\"{}\"", escape_json_string(value)),
            PrimitiveValue::Bytes(value) => format!("\"{}\"", base64_encode(value)),
            PrimitiveValue::Date(value) => format!("\"{}\"", iso_string_from_millis(*value)),
        }
    }

    pub(crate) fn to_sorted_json(&self) -> String {
        self.to_json()
    }

    pub(crate) fn deepcopy(&self) -> Self {
        self.clone()
    }

    pub(crate) fn primitive_type(&self) -> PrimitiveType {
        self.value_type
    }

    pub(crate) fn value(&self) -> &PrimitiveValue {
        &self.value
    }

    pub(crate) fn is_numeric_type(&self) -> bool {
        matches!(
            self.value_type,
            PrimitiveType::Integer | PrimitiveType::Long | PrimitiveType::Double
        )
    }

    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        match &self.value {
            PrimitiveValue::Null => Vec::new(),
            PrimitiveValue::Boolean(value) => vec![u8::from(*value)],
            PrimitiveValue::Integer(value) => value.to_le_bytes().to_vec(),
            PrimitiveValue::Long(value) => value.to_le_bytes().to_vec(),
            PrimitiveValue::Double(value) => value.to_le_bytes().to_vec(),
            PrimitiveValue::String(value) => value.as_bytes().to_vec(),
            PrimitiveValue::Bytes(value) => value.clone(),
            PrimitiveValue::Date(value) => (*value as u64).to_le_bytes().to_vec(),
        }
    }

    fn value_size(&self) -> usize {
        match &self.value {
            PrimitiveValue::Null => 8,
            PrimitiveValue::Boolean(_) => 4,
            PrimitiveValue::Integer(_) => 4,
            PrimitiveValue::Long(_) => 8,
            PrimitiveValue::Double(_) => 8,
            PrimitiveValue::String(value) => value.encode_utf16().count() * 2,
            PrimitiveValue::Bytes(value) => value.len(),
            PrimitiveValue::Date(_) => 8,
        }
    }
}

fn fixed_bytes<const N: usize>(primitive_type: PrimitiveType, bytes: &[u8]) -> Result<[u8; N]> {
    bytes
        .try_into()
        .map_err(|_| YorkieError::InvalidPrimitiveBytes {
            primitive_type: primitive_type.name(),
            expected: N,
            actual: bytes.len(),
        })
}

fn double_to_json(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }

    if value == f64::INFINITY {
        return "Infinity".to_owned();
    }

    if value == f64::NEG_INFINITY {
        return "-Infinity".to_owned();
    }

    value.to_string()
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);

        encoded.push(TABLE[(first >> 2) as usize] as char);
        encoded.push(TABLE[(((first & 0b0000_0011) << 4) | (second >> 4)) as usize] as char);

        if chunk.len() > 1 {
            encoded.push(TABLE[(((second & 0b0000_1111) << 2) | (third >> 6)) as usize] as char);
        } else {
            encoded.push('=');
        }

        if chunk.len() > 2 {
            encoded.push(TABLE[(third & 0b0011_1111) as usize] as char);
        } else {
            encoded.push('=');
        }
    }

    encoded
}

fn iso_string_from_millis(millis: i64) -> String {
    let seconds = millis.div_euclid(1000);
    let milliseconds = millis.rem_euclid(1000);
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3600;
    let minute = (seconds_of_day % 3600) / 60;
    let second = seconds_of_day % 60;

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{milliseconds:03}Z")
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };

    (year as i32, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::{CrdtPrimitive, PrimitiveType, PrimitiveValue};
    use crate::{TimeTicket, TIME_TICKET_SIZE};

    #[test]
    fn detects_primitive_types() {
        let values = [
            (PrimitiveValue::Null, PrimitiveType::Null),
            (PrimitiveValue::Boolean(false), PrimitiveType::Boolean),
            (PrimitiveValue::Integer(i32::MAX), PrimitiveType::Integer),
            (PrimitiveValue::Double(1.79), PrimitiveType::Double),
            (
                PrimitiveValue::String("4".to_owned()),
                PrimitiveType::String,
            ),
            (PrimitiveValue::Long(i64::MAX), PrimitiveType::Long),
            (PrimitiveValue::Bytes(vec![65, 66]), PrimitiveType::Bytes),
            (PrimitiveValue::Date(819_170_640_000), PrimitiveType::Date),
        ];

        for (value, primitive_type) in values {
            let primitive = CrdtPrimitive::new(value, TimeTicket::initial());
            assert_eq!(primitive_type, primitive.primitive_type());
        }
    }

    #[test]
    fn roundtrips_primitive_values_through_bytes() -> crate::Result<()> {
        let values = [
            PrimitiveValue::Null,
            PrimitiveValue::Boolean(false),
            PrimitiveValue::Integer(i32::MAX),
            PrimitiveValue::Double(1.79),
            PrimitiveValue::String("4".to_owned()),
            PrimitiveValue::Long(i64::MAX),
            PrimitiveValue::Bytes(vec![65, 66]),
            PrimitiveValue::Date(819_170_640_000),
        ];

        for value in values {
            let primitive = CrdtPrimitive::new(value.clone(), TimeTicket::initial());
            assert_eq!(
                value,
                CrdtPrimitive::value_from_bytes(primitive.primitive_type(), &primitive.to_bytes())?
            );
        }

        Ok(())
    }

    #[test]
    fn encodes_primitive_values_to_expected_bytes() {
        assert_eq!(
            Vec::<u8>::new(),
            CrdtPrimitive::new(PrimitiveValue::Null, TimeTicket::initial()).to_bytes()
        );
        assert_eq!(
            vec![1],
            CrdtPrimitive::new(PrimitiveValue::Boolean(true), TimeTicket::initial()).to_bytes()
        );
        assert_eq!(
            vec![4, 3, 2, 1],
            CrdtPrimitive::new(PrimitiveValue::Integer(0x0102_0304), TimeTicket::initial())
                .to_bytes()
        );
        assert_eq!(
            vec![255; 8],
            CrdtPrimitive::new(PrimitiveValue::Long(-1), TimeTicket::initial()).to_bytes()
        );
        assert_eq!(
            1.79f64.to_le_bytes().to_vec(),
            CrdtPrimitive::new(PrimitiveValue::Double(1.79), TimeTicket::initial()).to_bytes()
        );
        assert_eq!(
            vec![65, 66],
            CrdtPrimitive::new(
                PrimitiveValue::String("AB".to_owned()),
                TimeTicket::initial()
            )
            .to_bytes()
        );
        assert_eq!(
            vec![65, 66],
            CrdtPrimitive::new(PrimitiveValue::Bytes(vec![65, 66]), TimeTicket::initial())
                .to_bytes()
        );
        assert_eq!(
            819_170_640_000_u64.to_le_bytes().to_vec(),
            CrdtPrimitive::new(PrimitiveValue::Date(819_170_640_000), TimeTicket::initial())
                .to_bytes()
        );
    }

    #[test]
    fn serializes_primitive_values_to_json() {
        let string = CrdtPrimitive::new(
            PrimitiveValue::String("hello\n".to_owned()),
            TimeTicket::initial(),
        );
        assert_eq!("\"hello\\n\"", string.to_json());

        let bytes = CrdtPrimitive::new(PrimitiveValue::Bytes(vec![65, 66]), TimeTicket::initial());
        assert_eq!("\"QUI=\"", bytes.to_json());

        let date = CrdtPrimitive::new(PrimitiveValue::Date(819_170_640_000), TimeTicket::initial());
        assert_eq!("\"1995-12-17T03:24:00.000Z\"", date.to_json());
        assert_eq!(date.to_json(), date.to_sorted_json());

        let nan = CrdtPrimitive::new(PrimitiveValue::Double(f64::NAN), TimeTicket::initial());
        assert_eq!("NaN", nan.to_json());

        let infinity =
            CrdtPrimitive::new(PrimitiveValue::Double(f64::INFINITY), TimeTicket::initial());
        assert_eq!("Infinity", infinity.to_json());
    }

    #[test]
    fn reports_value_and_metadata_size() {
        let mut primitive = CrdtPrimitive::new(
            PrimitiveValue::String("😀".to_owned()),
            TimeTicket::initial(),
        );

        assert_eq!(
            super::DataSize {
                data: 4,
                meta: TIME_TICKET_SIZE,
            },
            primitive.data_size()
        );

        primitive.set_removed_at(Some(TimeTicket::new(1, 0, "a")));
        assert_eq!(TIME_TICKET_SIZE * 2, primitive.data_size().meta);
    }

    #[test]
    fn deep_copies_value_and_metadata() {
        let mut primitive = CrdtPrimitive::new(PrimitiveValue::Integer(1), TimeTicket::initial());
        let moved_at = TimeTicket::new(1, 0, "a");
        let removed_at = TimeTicket::new(2, 0, "a");
        primitive.set_moved_at(Some(moved_at.clone()));
        primitive.set_removed_at(Some(removed_at.clone()));

        let copied = primitive.deepcopy();

        assert_eq!(PrimitiveType::Integer, copied.primitive_type());
        assert_eq!(&PrimitiveValue::Integer(1), copied.value());
        assert_eq!(primitive.created_at(), copied.created_at());
        assert_eq!(Some(&moved_at), copied.moved_at());
        assert_eq!(Some(&removed_at), copied.removed_at());
    }

    #[test]
    fn identifies_numeric_types() {
        assert!(
            CrdtPrimitive::new(PrimitiveValue::Integer(1), TimeTicket::initial()).is_numeric_type()
        );
        assert!(
            CrdtPrimitive::new(PrimitiveValue::Long(1), TimeTicket::initial()).is_numeric_type()
        );
        assert!(
            CrdtPrimitive::new(PrimitiveValue::Double(1.0), TimeTicket::initial())
                .is_numeric_type()
        );
        assert!(!CrdtPrimitive::new(
            PrimitiveValue::String("1".to_owned()),
            TimeTicket::initial()
        )
        .is_numeric_type());
    }
}
