use super::element::{CrdtElementMeta, DataSize};
use super::primitive::{CrdtPrimitive, PrimitiveValue};
use crate::{Result, TimeTicket, YorkieError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CounterType {
    Integer,
    Long,
}

impl CounterType {
    fn name(self) -> &'static str {
        match self {
            Self::Integer => "integer counter",
            Self::Long => "long counter",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum CounterValue {
    Integer(i32),
    Long(i64),
    Double(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CrdtCounter {
    meta: CrdtElementMeta,
    value_type: CounterType,
    value: CounterValue,
}

impl CrdtCounter {
    pub(crate) fn new(
        value_type: CounterType,
        value: CounterValue,
        created_at: TimeTicket,
    ) -> Self {
        Self {
            meta: CrdtElementMeta::new(created_at),
            value_type,
            value: normalize_value(value_type, value),
        }
    }

    pub(crate) fn create(
        value_type: CounterType,
        value: CounterValue,
        created_at: TimeTicket,
    ) -> Self {
        Self::new(value_type, value, created_at)
    }

    pub(crate) fn value_from_bytes(
        counter_type: CounterType,
        bytes: &[u8],
    ) -> Result<CounterValue> {
        match counter_type {
            CounterType::Integer => {
                let bytes = fixed_bytes::<4>(counter_type, bytes)?;
                Ok(CounterValue::Integer(i32::from_le_bytes(bytes)))
            }
            CounterType::Long => {
                let bytes = fixed_bytes::<8>(counter_type, bytes)?;
                Ok(CounterValue::Long(i64::from_le_bytes(bytes)))
            }
        }
    }

    pub(crate) fn created_at(&self) -> &TimeTicket {
        self.meta.created_at()
    }

    pub(crate) fn id(&self) -> &TimeTicket {
        self.meta.id()
    }

    pub(crate) fn moved_at(&self) -> Option<&TimeTicket> {
        self.meta.moved_at()
    }

    pub(crate) fn removed_at(&self) -> Option<&TimeTicket> {
        self.meta.removed_at()
    }

    pub(crate) fn positioned_at(&self) -> &TimeTicket {
        self.meta.positioned_at()
    }

    pub(crate) fn set_moved_at(&mut self, moved_at: Option<TimeTicket>) -> bool {
        self.meta.set_moved_at(moved_at)
    }

    pub(crate) fn set_removed_at(&mut self, removed_at: Option<TimeTicket>) {
        self.meta.set_removed_at(removed_at);
    }

    pub(crate) fn remove(&mut self, removed_at: Option<TimeTicket>) -> bool {
        self.meta.remove(removed_at)
    }

    pub(crate) fn is_removed(&self) -> bool {
        self.meta.is_removed()
    }

    pub(crate) fn meta_usage(&self) -> usize {
        self.meta.meta_usage()
    }

    pub(crate) fn data_size(&self) -> DataSize {
        DataSize {
            data: match self.value_type {
                CounterType::Integer => 4,
                CounterType::Long => 8,
            },
            meta: self.meta_usage(),
        }
    }

    pub(crate) fn counter_type(&self) -> CounterType {
        self.value_type
    }

    pub(crate) fn value(&self) -> CounterValue {
        self.value
    }

    pub(crate) fn is_numeric_type(&self) -> bool {
        matches!(self.value_type, CounterType::Integer | CounterType::Long)
    }

    pub(crate) fn increase(&mut self, operand: &CrdtPrimitive) -> Result<()> {
        if !self.is_numeric_type() || !operand.is_numeric_type() {
            return Err(YorkieError::UnexpectedCrdtElement {
                id: operand.created_at().to_id_string(),
                expected: "numeric primitive",
            });
        }

        self.value = match self.value_type {
            CounterType::Integer => {
                let current = integer_value(self.value);
                CounterValue::Integer(current.wrapping_add(primitive_to_integer(operand)?))
            }
            CounterType::Long => {
                let current = long_value(self.value);
                CounterValue::Long(current.wrapping_add(primitive_to_long(operand)?))
            }
        };

        Ok(())
    }

    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        match self.value {
            CounterValue::Integer(value) => value.to_le_bytes().to_vec(),
            CounterValue::Long(value) => value.to_le_bytes().to_vec(),
            CounterValue::Double(value) => value.to_le_bytes().to_vec(),
        }
    }

    pub(crate) fn to_json(&self) -> String {
        match self.value {
            CounterValue::Integer(value) => value.to_string(),
            CounterValue::Long(value) => value.to_string(),
            CounterValue::Double(value) => value.trunc().to_string(),
        }
    }

    pub(crate) fn to_sorted_json(&self) -> String {
        self.to_json()
    }

    pub(crate) fn deepcopy(&self) -> Self {
        self.clone()
    }
}

fn normalize_value(value_type: CounterType, value: CounterValue) -> CounterValue {
    match value_type {
        CounterType::Integer => CounterValue::Integer(integer_value(value)),
        CounterType::Long => CounterValue::Long(long_value(value)),
    }
}

fn primitive_to_integer(primitive: &CrdtPrimitive) -> Result<i32> {
    match primitive.value() {
        PrimitiveValue::Integer(value) => Ok(*value),
        PrimitiveValue::Long(value) => Ok(*value as i32),
        PrimitiveValue::Double(value) => Ok(double_to_integer(*value)),
        _ => Err(YorkieError::UnexpectedCrdtElement {
            id: primitive.created_at().to_id_string(),
            expected: "numeric primitive",
        }),
    }
}

fn primitive_to_long(primitive: &CrdtPrimitive) -> Result<i64> {
    match primitive.value() {
        PrimitiveValue::Integer(value) => Ok(i64::from(*value)),
        PrimitiveValue::Long(value) => Ok(*value),
        PrimitiveValue::Double(value) => Ok(double_to_long(*value)),
        _ => Err(YorkieError::UnexpectedCrdtElement {
            id: primitive.created_at().to_id_string(),
            expected: "numeric primitive",
        }),
    }
}

fn integer_value(value: CounterValue) -> i32 {
    match value {
        CounterValue::Integer(value) => value,
        CounterValue::Long(value) => value as i32,
        CounterValue::Double(value) => double_to_integer(value),
    }
}

fn long_value(value: CounterValue) -> i64 {
    match value {
        CounterValue::Integer(value) => i64::from(value),
        CounterValue::Long(value) => value,
        CounterValue::Double(value) => double_to_long(value),
    }
}

fn double_to_integer(value: f64) -> i32 {
    if !value.is_finite() {
        return value as i32;
    }

    let wrapped = value.trunc().rem_euclid(4_294_967_296.0);
    (wrapped as u32) as i32
}

fn double_to_long(value: f64) -> i64 {
    if !value.is_finite() {
        return value as i64;
    }

    let wrapped = value.trunc().rem_euclid(18_446_744_073_709_551_616.0);
    (wrapped as u64) as i64
}

fn fixed_bytes<const N: usize>(counter_type: CounterType, bytes: &[u8]) -> Result<[u8; N]> {
    bytes
        .try_into()
        .map_err(|_| YorkieError::InvalidPrimitiveBytes {
            primitive_type: counter_type.name(),
            expected: N,
            actual: bytes.len(),
        })
}

#[cfg(test)]
mod tests {
    use super::{CounterType, CounterValue, CrdtCounter};
    use crate::crdt::primitive::{CrdtPrimitive, PrimitiveValue};
    use crate::{TimeTicket, TIME_TICKET_SIZE};

    #[test]
    fn creates_integer_and_long_counters() {
        let integer = CrdtCounter::create(
            CounterType::Integer,
            CounterValue::Long(i64::from(i32::MAX) + 1),
            TimeTicket::initial(),
        );
        let long = CrdtCounter::create(
            CounterType::Long,
            CounterValue::Double(10.5),
            TimeTicket::initial(),
        );

        assert_eq!(CounterType::Integer, integer.counter_type());
        assert_eq!(CounterValue::Integer(i32::MIN), integer.value());
        assert_eq!("-2147483648", integer.to_json());
        assert_eq!(CounterType::Long, long.counter_type());
        assert_eq!(CounterValue::Long(10), long.value());
        assert_eq!("10", long.to_json());
    }

    #[test]
    fn increases_numeric_counters() -> crate::Result<()> {
        let mut integer =
            CrdtCounter::create(CounterType::Integer, CounterValue::Integer(10), ticket(1));
        let mut long = CrdtCounter::create(CounterType::Long, CounterValue::Long(100), ticket(2));

        integer.increase(&primitive(PrimitiveValue::Integer(5), ticket(3)))?;
        integer.increase(&primitive(PrimitiveValue::Long(3), ticket(4)))?;
        integer.increase(&primitive(PrimitiveValue::Double(1.9), ticket(5)))?;
        long.increase(&primitive(PrimitiveValue::Integer(5), ticket(6)))?;
        long.increase(&primitive(PrimitiveValue::Long(3), ticket(7)))?;
        long.increase(&primitive(PrimitiveValue::Double(1.9), ticket(8)))?;

        assert_eq!(CounterValue::Integer(19), integer.value());
        assert_eq!(CounterValue::Long(109), long.value());
        Ok(())
    }

    #[test]
    fn wraps_integer_and_long_overflow() -> crate::Result<()> {
        let mut integer = CrdtCounter::create(
            CounterType::Integer,
            CounterValue::Integer(i32::MAX),
            ticket(1),
        );
        let mut long =
            CrdtCounter::create(CounterType::Long, CounterValue::Long(i64::MAX), ticket(2));

        integer.increase(&primitive(PrimitiveValue::Integer(1), ticket(3)))?;
        long.increase(&primitive(PrimitiveValue::Long(1), ticket(4)))?;

        assert_eq!(CounterValue::Integer(i32::MIN), integer.value());
        assert_eq!(CounterValue::Long(i64::MIN), long.value());
        Ok(())
    }

    #[test]
    fn wraps_double_operands_like_fixed_width_numbers() -> crate::Result<()> {
        let mut integer =
            CrdtCounter::create(CounterType::Integer, CounterValue::Integer(0), ticket(1));
        let mut long = CrdtCounter::create(CounterType::Long, CounterValue::Long(0), ticket(2));

        integer.increase(&primitive(
            PrimitiveValue::Double(f64::from(i32::MAX) + 1.0),
            ticket(3),
        ))?;
        long.increase(&primitive(
            PrimitiveValue::Double(9_223_372_036_854_775_808.0),
            ticket(4),
        ))?;

        assert_eq!(CounterValue::Integer(i32::MIN), integer.value());
        assert_eq!(CounterValue::Long(i64::MIN), long.value());
        Ok(())
    }

    #[test]
    fn roundtrips_counter_values_through_bytes() -> crate::Result<()> {
        let integer =
            CrdtCounter::create(CounterType::Integer, CounterValue::Integer(-1), ticket(1));
        let long = CrdtCounter::create(CounterType::Long, CounterValue::Long(-1), ticket(2));

        assert_eq!(
            CounterValue::Integer(-1),
            CrdtCounter::value_from_bytes(CounterType::Integer, &integer.to_bytes())?
        );
        assert_eq!(
            CounterValue::Long(-1),
            CrdtCounter::value_from_bytes(CounterType::Long, &long.to_bytes())?
        );
        Ok(())
    }

    #[test]
    fn deepcopies_counter_metadata() {
        let mut counter =
            CrdtCounter::create(CounterType::Integer, CounterValue::Integer(1), ticket(1));
        counter.set_moved_at(Some(ticket(2)));
        counter.set_removed_at(Some(ticket(3)));

        let copy = counter.deepcopy();

        assert_eq!(counter, copy);
        assert_eq!(TIME_TICKET_SIZE * 3, copy.data_size().meta);
    }

    fn primitive(value: PrimitiveValue, created_at: TimeTicket) -> CrdtPrimitive {
        CrdtPrimitive::new(value, created_at)
    }

    fn ticket(lamport: i64) -> TimeTicket {
        TimeTicket::new(lamport, 0, "a")
    }
}
