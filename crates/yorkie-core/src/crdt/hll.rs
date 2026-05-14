use crate::{Result, YorkieError};

const HLL_PRECISION: usize = 14;
const HLL_REGISTER_COUNT: usize = 1 << HLL_PRECISION;

const PRIME64_X1: u64 = 0x9e37_79b1_85eb_ca87;
const PRIME64_X2: u64 = 0xc2b2_ae3d_27d4_eb4f;
const PRIME64_X3: u64 = 0x1656_67b1_9e37_79f9;
const PRIME64_X4: u64 = 0x85eb_ca77_c2b2_ae63;
const PRIME64_X5: u64 = 0x27d4_eb2f_1656_67c5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hll {
    registers: [u8; HLL_REGISTER_COUNT],
}

impl Hll {
    pub(crate) fn new() -> Self {
        Self {
            registers: [0; HLL_REGISTER_COUNT],
        }
    }

    pub(crate) fn add(&mut self, value: &str) -> bool {
        let hash = xxhash64(value);
        let index = (hash >> (64 - HLL_PRECISION)) as usize;
        let remaining = (hash << HLL_PRECISION) | (1u64 << (HLL_PRECISION - 1));
        let rho = remaining.leading_zeros() as u8 + 1;

        if rho > self.registers[index] {
            self.registers[index] = rho;
            return true;
        }

        false
    }

    pub(crate) fn count(&self) -> u64 {
        let m = HLL_REGISTER_COUNT as f64;
        let alpha = 0.7213 / (1.0 + 1.079 / m);
        let mut sum = 0.0;
        let mut zeros = 0;

        for register in self.registers {
            sum += 2f64.powi(-(i32::from(register)));
            if register == 0 {
                zeros += 1;
            }
        }

        let mut estimate = alpha * m * m / sum;
        if estimate <= 2.5 * m && zeros > 0 {
            estimate = m * (m / f64::from(zeros)).ln();
        }

        estimate.round() as u64
    }

    pub(crate) fn merge(&mut self, other: &Self) {
        for (target, source) in self.registers.iter_mut().zip(other.registers.iter()) {
            if *source > *target {
                *target = *source;
            }
        }
    }

    pub(crate) fn to_bytes(&self) -> Vec<u8> {
        self.registers.to_vec()
    }

    pub(crate) fn restore(&mut self, data: &[u8]) -> Result<()> {
        if data.len() != HLL_REGISTER_COUNT {
            return Err(YorkieError::InvalidPrimitiveBytes {
                primitive_type: "HLL registers",
                expected: HLL_REGISTER_COUNT,
                actual: data.len(),
            });
        }

        self.registers.copy_from_slice(data);
        Ok(())
    }
}

impl Default for Hll {
    fn default() -> Self {
        Self::new()
    }
}

fn xxhash64(input: &str) -> u64 {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut offset = 0;
    let seed = 0u64;

    let mut hash = if len >= 32 {
        let mut v1 = seed.wrapping_add(PRIME64_X1).wrapping_add(PRIME64_X2);
        let mut v2 = seed.wrapping_add(PRIME64_X2);
        let mut v3 = seed;
        let mut v4 = seed.wrapping_sub(PRIME64_X1);

        while offset <= len - 32 {
            v1 = xx_round(v1, read_u64_le(bytes, offset));
            offset += 8;
            v2 = xx_round(v2, read_u64_le(bytes, offset));
            offset += 8;
            v3 = xx_round(v3, read_u64_le(bytes, offset));
            offset += 8;
            v4 = xx_round(v4, read_u64_le(bytes, offset));
            offset += 8;
        }

        let mut hash = v1
            .rotate_left(1)
            .wrapping_add(v2.rotate_left(7))
            .wrapping_add(v3.rotate_left(12))
            .wrapping_add(v4.rotate_left(18));
        hash = xx_merge_round(hash, v1);
        hash = xx_merge_round(hash, v2);
        hash = xx_merge_round(hash, v3);
        xx_merge_round(hash, v4)
    } else {
        seed.wrapping_add(PRIME64_X5)
    };

    hash = hash.wrapping_add(len as u64);

    while offset + 8 <= len {
        let k1 = xx_round(0, read_u64_le(bytes, offset));
        hash = (hash ^ k1)
            .rotate_left(27)
            .wrapping_mul(PRIME64_X1)
            .wrapping_add(PRIME64_X4);
        offset += 8;
    }

    if offset + 4 <= len {
        hash ^= u64::from(read_u32_le(bytes, offset)).wrapping_mul(PRIME64_X1);
        hash = hash
            .rotate_left(23)
            .wrapping_mul(PRIME64_X2)
            .wrapping_add(PRIME64_X3);
        offset += 4;
    }

    while offset < len {
        hash ^= u64::from(bytes[offset]).wrapping_mul(PRIME64_X5);
        hash = hash.rotate_left(11).wrapping_mul(PRIME64_X1);
        offset += 1;
    }

    hash = (hash ^ (hash >> 33)).wrapping_mul(PRIME64_X2);
    hash = (hash ^ (hash >> 29)).wrapping_mul(PRIME64_X3);
    hash ^ (hash >> 32)
}

fn xx_round(acc: u64, input: u64) -> u64 {
    acc.wrapping_add(input.wrapping_mul(PRIME64_X2))
        .rotate_left(31)
        .wrapping_mul(PRIME64_X1)
}

fn xx_merge_round(acc: u64, value: u64) -> u64 {
    let value = xx_round(0, value);
    (acc ^ value)
        .wrapping_mul(PRIME64_X1)
        .wrapping_add(PRIME64_X4)
}

fn read_u64_le(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

#[cfg(test)]
mod tests {
    use super::{Hll, HLL_REGISTER_COUNT};

    #[test]
    fn starts_with_zero_count() {
        let hll = Hll::new();

        assert_eq!(0, hll.count());
    }

    #[test]
    fn adds_single_element_once() {
        let mut hll = Hll::new();

        assert!(hll.add("user-1"));
        assert!(!hll.add("user-1"));
        assert_eq!(1, hll.count());
    }

    #[test]
    fn counts_many_unique_elements_within_error_margin() {
        let mut hll = Hll::new();
        let count = 100_000;

        for index in 0..count {
            hll.add(&format!("user-{index}"));
        }

        let estimate = hll.count();
        let error_rate = (estimate.abs_diff(count) as f64) / (count as f64);
        assert!(error_rate < 0.05, "estimate={estimate}");
    }

    #[test]
    fn merges_by_taking_register_maximums() {
        let mut left = Hll::new();
        let mut right = Hll::new();

        left.add("user-1");
        left.add("user-2");
        right.add("user-2");
        right.add("user-3");

        left.merge(&right);

        assert_eq!(3, left.count());
    }

    #[test]
    fn serializes_and_restores_registers() -> crate::Result<()> {
        let mut hll = Hll::new();
        hll.add("user-1");
        hll.add("user-2");
        hll.add("user-3");

        let bytes = hll.to_bytes();
        let mut restored = Hll::new();
        restored.restore(&bytes)?;

        assert_eq!(HLL_REGISTER_COUNT, bytes.len());
        assert_eq!(hll.count(), restored.count());
        Ok(())
    }
}
