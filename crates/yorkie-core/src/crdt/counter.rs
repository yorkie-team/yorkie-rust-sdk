use super::element::{CrdtElementMeta, DataSize};
use super::hll::Hll;
use super::primitive::{CrdtPrimitive, PrimitiveValue};
use crate::{Result, TimeTicket, YorkieError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CounterType {
    Integer,
    Long,
    IntegerDedup,
}

impl CounterType {
    fn name(self) -> &'static str {
        match self {
            Self::Integer => "integer counter",
            Self::Long => "long counter",
            Self::IntegerDedup => "integer dedup counter",
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
    hll: Option<Hll>,
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
            hll: value_type.is_dedup().then(Hll::new),
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
            CounterType::Integer | CounterType::IntegerDedup => {
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
                CounterType::Integer | CounterType::IntegerDedup => 4,
                CounterType::Long => 8,
            } + self
                .hll
                .as_ref()
                .map(|hll| hll.to_bytes().len())
                .unwrap_or(0),
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
        matches!(
            self.value_type,
            CounterType::Integer | CounterType::Long | CounterType::IntegerDedup
        )
    }

    pub(crate) fn is_dedup(&self) -> bool {
        self.value_type.is_dedup()
    }

    pub(crate) fn increase(&mut self, operand: &CrdtPrimitive) -> Result<()> {
        if self.is_dedup() {
            return Err(YorkieError::InvalidCounterOperation(
                "dedup counter requires actor".to_owned(),
            ));
        }

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
            CounterType::IntegerDedup => unreachable!("dedup counters require actor"),
        };

        Ok(())
    }

    pub(crate) fn increase_dedup(&mut self, operand: &CrdtPrimitive, actor: &str) -> Result<()> {
        if !self.is_dedup() {
            return self.increase(operand);
        }

        if !operand.is_numeric_type() {
            return Err(YorkieError::UnexpectedCrdtElement {
                id: operand.created_at().to_id_string(),
                expected: "numeric primitive",
            });
        }

        if actor.is_empty() {
            return Err(YorkieError::InvalidCounterOperation(
                "dedup counter requires actor".to_owned(),
            ));
        }

        if !is_unit_increment(operand.value()) {
            return Err(YorkieError::InvalidCounterOperation(
                "dedup counter only supports increment by 1".to_owned(),
            ));
        }

        if self.hll.get_or_insert_with(Hll::new).add(actor) {
            self.recompute_dedup_value();
        }

        Ok(())
    }

    pub(crate) fn hll_bytes(&self) -> Option<Vec<u8>> {
        self.hll.as_ref().map(Hll::to_bytes)
    }

    pub(crate) fn restore_hll(&mut self, data: &[u8]) -> Result<()> {
        let hll = self.hll.get_or_insert_with(Hll::new);
        hll.restore(data)?;
        self.recompute_dedup_value();
        Ok(())
    }

    fn recompute_dedup_value(&mut self) {
        if let (CounterType::IntegerDedup, Some(hll)) = (self.value_type, self.hll.as_ref()) {
            self.value = CounterValue::Integer(hll.count() as i32);
        }
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

impl CounterType {
    fn is_dedup(self) -> bool {
        matches!(self, Self::IntegerDedup)
    }
}

fn normalize_value(value_type: CounterType, value: CounterValue) -> CounterValue {
    match value_type {
        CounterType::Integer => CounterValue::Integer(integer_value(value)),
        CounterType::Long => CounterValue::Long(long_value(value)),
        CounterType::IntegerDedup => CounterValue::Integer(0),
    }
}

fn is_unit_increment(value: &PrimitiveValue) -> bool {
    match value {
        PrimitiveValue::Integer(value) => *value == 1,
        PrimitiveValue::Long(value) => *value == 1,
        PrimitiveValue::Double(value) => *value == 1.0,
        _ => false,
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
    use crate::{TimeTicket, YorkieError, TIME_TICKET_SIZE};

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

        let dedup = CrdtCounter::create(
            CounterType::IntegerDedup,
            CounterValue::Integer(10),
            TimeTicket::initial(),
        );
        assert_eq!(CounterType::IntegerDedup, dedup.counter_type());
        assert!(dedup.is_dedup());
        assert_eq!(CounterValue::Integer(0), dedup.value());
        assert_eq!("0", dedup.to_json());
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
        let dedup = CrdtCounter::create(
            CounterType::IntegerDedup,
            CounterValue::Integer(0),
            ticket(3),
        );

        assert_eq!(
            CounterValue::Integer(-1),
            CrdtCounter::value_from_bytes(CounterType::Integer, &integer.to_bytes())?
        );
        assert_eq!(
            CounterValue::Long(-1),
            CrdtCounter::value_from_bytes(CounterType::Long, &long.to_bytes())?
        );
        assert_eq!(
            CounterValue::Integer(0),
            CrdtCounter::value_from_bytes(CounterType::IntegerDedup, &dedup.to_bytes())?
        );
        Ok(())
    }

    #[test]
    fn increases_dedup_counters_once_per_actor() -> crate::Result<()> {
        let mut counter = CrdtCounter::create(
            CounterType::IntegerDedup,
            CounterValue::Integer(0),
            ticket(1),
        );
        let operand = primitive(PrimitiveValue::Integer(1), ticket(2));

        counter.increase_dedup(&operand, "user-1")?;
        counter.increase_dedup(&operand, "user-1")?;
        counter.increase_dedup(&operand, "user-2")?;
        counter.increase_dedup(&operand, "user-3")?;

        assert_eq!(CounterValue::Integer(3), counter.value());
        assert_eq!("3", counter.to_json());
        assert_eq!(4 + 16_384, counter.data_size().data);
        Ok(())
    }

    #[test]
    fn rejects_invalid_dedup_increases() {
        let mut counter = CrdtCounter::create(
            CounterType::IntegerDedup,
            CounterValue::Integer(0),
            ticket(1),
        );

        let missing_actor = counter
            .increase_dedup(&primitive(PrimitiveValue::Integer(1), ticket(2)), "")
            .unwrap_err();
        assert_eq!(
            YorkieError::InvalidCounterOperation("dedup counter requires actor".to_owned()),
            missing_actor
        );

        let non_unit = counter
            .increase_dedup(&primitive(PrimitiveValue::Integer(2), ticket(3)), "user-1")
            .unwrap_err();
        assert_eq!(
            YorkieError::InvalidCounterOperation(
                "dedup counter only supports increment by 1".to_owned()
            ),
            non_unit
        );
    }

    #[test]
    fn restores_hll_state_and_deepcopies_independently() -> crate::Result<()> {
        let mut counter = CrdtCounter::create(
            CounterType::IntegerDedup,
            CounterValue::Integer(0),
            ticket(1),
        );
        let operand = primitive(PrimitiveValue::Integer(1), ticket(2));
        counter.increase_dedup(&operand, "user-1")?;
        counter.increase_dedup(&operand, "user-2")?;

        let bytes = counter.hll_bytes().unwrap();
        let mut restored = CrdtCounter::create(
            CounterType::IntegerDedup,
            CounterValue::Integer(0),
            ticket(3),
        );
        restored.restore_hll(&bytes)?;
        assert_eq!(CounterValue::Integer(2), restored.value());

        let copy = restored.deepcopy();
        restored.increase_dedup(&operand, "user-3")?;

        assert_eq!(CounterValue::Integer(3), restored.value());
        assert_eq!(CounterValue::Integer(2), copy.value());
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
