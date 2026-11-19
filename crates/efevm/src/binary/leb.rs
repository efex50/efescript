//! LEB128 variable-length integer encoding and decoding (unsigned and signed,
//! 32 and 64 bit), following the WebAssembly rules for over-long encodings.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LebError {
    /// The input ended before a byte without the continuation bit was seen.
    UnexpectedEnd,
    /// The encoding is longer than the maximum for the type, or the final byte
    /// sets bits that do not fit into the type.
    TooLong,
}

const CONT: u8 = 0x80;
const PAYLOAD: u8 = 0x7F;
const SIGN_BIT: u8 = 0x40;

/// Maximum number of bytes for a LEB128 value of the given bit width.
const fn max_len(bits: u32) -> u32 {
    bits.div_ceil(7)
}

pub(crate) fn write_u32(out: &mut Vec<u8>, v: u32) {
    write_unsigned(out, u64::from(v));
}

pub(crate) fn write_u64(out: &mut Vec<u8>, v: u64) {
    write_unsigned(out, v);
}

pub(crate) fn write_i32(out: &mut Vec<u8>, v: i32) {
    write_signed(out, i64::from(v));
}

pub(crate) fn write_i64(out: &mut Vec<u8>, v: i64) {
    write_signed(out, v);
}

fn write_unsigned(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let low = (v as u8) & PAYLOAD;
        v >>= 7;
        if v == 0 {
            out.push(low);
            return;
        }
        out.push(low | CONT);
    }
}

fn write_signed(out: &mut Vec<u8>, mut v: i64) {
    loop {
        let low = (v as u8) & PAYLOAD;
        v >>= 7;
        let done = (v == 0 && (low & SIGN_BIT) == 0) || (v == -1 && (low & SIGN_BIT) != 0);
        if done {
            out.push(low);
            return;
        }
        out.push(low | CONT);
    }
}

/// Reads an unsigned LEB128 value of at most `bits` bits starting at `*pos`.
/// On success `*pos` is advanced past the value; on error it is unchanged.
fn read_unsigned(bytes: &[u8], pos: &mut usize, bits: u32) -> Result<u64, LebError> {
    let max = max_len(bits);
    let start = *pos;
    let mut result: u64 = 0;
    for i in 0..max {
        let idx = start.checked_add(i as usize).ok_or(LebError::UnexpectedEnd)?;
        let byte = *bytes.get(idx).ok_or(LebError::UnexpectedEnd)?;
        let payload = u64::from(byte & PAYLOAD);
        let shift = 7 * i;
        let last = i + 1 == max;
        if last {
            if (byte & CONT) != 0 {
                return Err(LebError::TooLong);
            }
            // Bits of the final group that lie beyond the type width must be zero.
            let avail = bits - shift;
            if (payload >> avail) != 0 {
                return Err(LebError::TooLong);
            }
        }
        result |= payload << shift;
        if (byte & CONT) == 0 {
            *pos = idx + 1;
            return Ok(result);
        }
    }
    // Every byte up to the maximum had a continuation bit.
    Err(LebError::TooLong)
}

/// Reads a signed LEB128 value of at most `bits` bits starting at `*pos`.
/// On success `*pos` is advanced past the value; on error it is unchanged.
fn read_signed(bytes: &[u8], pos: &mut usize, bits: u32) -> Result<i64, LebError> {
    let max = max_len(bits);
    let start = *pos;
    let mut result: u64 = 0;
    for i in 0..max {
        let idx = start.checked_add(i as usize).ok_or(LebError::UnexpectedEnd)?;
        let byte = *bytes.get(idx).ok_or(LebError::UnexpectedEnd)?;
        let payload = byte & PAYLOAD;
        let shift = 7 * i;
        let last = i + 1 == max;
        result |= u64::from(payload) << shift;
        if (byte & CONT) != 0 {
            if last {
                return Err(LebError::TooLong);
            }
            continue;
        }
        if last {
            // The unused high bits of the final group must all equal the sign
            // bit of the value (bit `avail - 1` of the group).
            let avail = bits - shift;
            let sign_set = ((payload >> (avail - 1)) & 1) != 0;
            let high = payload >> avail;
            let expected = if sign_set { PAYLOAD >> avail } else { 0 };
            if high != expected {
                return Err(LebError::TooLong);
            }
        }
        // Sign-extend from the last group written.
        let next_shift = 7 * (i + 1);
        if next_shift < 64 && (byte & SIGN_BIT) != 0 {
            result |= u64::MAX << next_shift;
        }
        *pos = idx + 1;
        return Ok(result as i64);
    }
    Err(LebError::TooLong)
}

pub(crate) fn read_u32(bytes: &[u8], pos: &mut usize) -> Result<u32, LebError> {
    read_unsigned(bytes, pos, 32).map(|v| v as u32)
}

pub(crate) fn read_u64(bytes: &[u8], pos: &mut usize) -> Result<u64, LebError> {
    read_unsigned(bytes, pos, 64)
}

pub(crate) fn read_i32(bytes: &[u8], pos: &mut usize) -> Result<i32, LebError> {
    read_signed(bytes, pos, 32).map(|v| v as i32)
}

pub(crate) fn read_i64(bytes: &[u8], pos: &mut usize) -> Result<i64, LebError> {
    read_signed(bytes, pos, 64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tiny xorshift64 generator for deterministic pseudo-random values.
    struct XorShift(u64);

    impl XorShift {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
    }

    fn min_len_unsigned(v: u64) -> usize {
        let bits = (64 - v.leading_zeros()) as usize;
        std::cmp::max(1, bits.div_ceil(7))
    }

    fn min_len_signed(v: i64) -> usize {
        for n in 1..=10usize {
            let lim = 1i128 << (7 * n - 1);
            let w = i128::from(v);
            if w >= -lim && w < lim {
                return n;
            }
        }
        10
    }

    fn enc_u64(v: u64) -> Vec<u8> {
        let mut b = Vec::new();
        write_u64(&mut b, v);
        b
    }

    fn enc_i64(v: i64) -> Vec<u8> {
        let mut b = Vec::new();
        write_i64(&mut b, v);
        b
    }

    fn check_u32(v: u32) {
        let mut b = Vec::new();
        write_u32(&mut b, v);
        assert_eq!(b.len(), min_len_unsigned(u64::from(v)), "minimal length for {v}");
        let mut p = 0;
        assert_eq!(read_u32(&b, &mut p), Ok(v));
        assert_eq!(p, b.len());
    }

    fn check_u64(v: u64) {
        let b = enc_u64(v);
        assert_eq!(b.len(), min_len_unsigned(v), "minimal length for {v}");
        let mut p = 0;
        assert_eq!(read_u64(&b, &mut p), Ok(v));
        assert_eq!(p, b.len());
    }

    fn check_i32(v: i32) {
        let mut b = Vec::new();
        write_i32(&mut b, v);
        assert_eq!(b.len(), min_len_signed(i64::from(v)), "minimal length for {v}");
        let mut p = 0;
        assert_eq!(read_i32(&b, &mut p), Ok(v));
        assert_eq!(p, b.len());
    }

    fn check_i64(v: i64) {
        let b = enc_i64(v);
        assert_eq!(b.len(), min_len_signed(v), "minimal length for {v}");
        let mut p = 0;
        assert_eq!(read_i64(&b, &mut p), Ok(v));
        assert_eq!(p, b.len());
    }

    #[test]
    fn round_trip_boundaries() {
        for v in [0u32, 1, 63, 64, 127, 128, 255, 16383, 16384, u32::MAX] {
            check_u32(v);
        }
        for v in [0u64, 1, 63, 64, 127, 128, 255, 16383, 16384, u32::MAX as u64, u64::MAX] {
            check_u64(v);
        }
        for v in [i32::MIN, i32::MAX, -1, -64, -65, 0, 1, 63, 64, 127, 128, -128, -129] {
            check_i32(v);
        }
        for v in [i64::MIN, i64::MAX, -1, -64, -65, 0, 1, 63, 64, 127, 128, -128, -129] {
            check_i64(v);
        }
    }

    #[test]
    fn round_trip_random() {
        let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
        for _ in 0..4000 {
            check_u32(rng.next() as u32);
            check_u64(rng.next());
            check_i32(rng.next() as i32);
            check_i64(rng.next() as i64);
            // Small magnitudes exercise the short encodings.
            let small = (rng.next() % 300) as i64 - 150;
            check_i32(small as i32);
            check_i64(small);
        }
    }

    #[test]
    fn exact_encodings() {
        let mut b = Vec::new();
        write_u32(&mut b, 624485);
        assert_eq!(b, [0xE5, 0x8E, 0x26]);

        let mut b = Vec::new();
        write_i32(&mut b, -123456);
        assert_eq!(b, [0xC0, 0xBB, 0x78]);

        let mut b = Vec::new();
        write_i64(&mut b, -1);
        assert_eq!(b, [0x7F]);

        let mut b = Vec::new();
        write_u32(&mut b, 0);
        assert_eq!(b, [0x00]);

        let mut b = Vec::new();
        write_i32(&mut b, -1);
        assert_eq!(b, [0x7F]);

        let mut b = Vec::new();
        write_i32(&mut b, i32::MIN);
        assert_eq!(b, [0x80, 0x80, 0x80, 0x80, 0x78]);
    }

    #[test]
    fn writers_append_to_existing_buffer() {
        let mut b = vec![0xAA];
        write_u32(&mut b, 1);
        write_i32(&mut b, -1);
        assert_eq!(b, [0xAA, 0x01, 0x7F]);
    }

    #[test]
    fn rejects_overlong_u32() {
        let mut p = 0;
        assert_eq!(
            read_u32(&[0x80, 0x80, 0x80, 0x80, 0x80, 0x00], &mut p),
            Err(LebError::TooLong)
        );
        assert_eq!(p, 0);
    }

    #[test]
    fn five_continuation_bytes_is_too_long() {
        // The 5th byte still has the continuation bit, so the maximum length is
        // exceeded; this is reported as TooLong even with no bytes following.
        let mut p = 0;
        assert_eq!(read_u32(&[0x80; 5], &mut p), Err(LebError::TooLong));
        assert_eq!(p, 0);
        let mut p = 0;
        assert_eq!(read_i32(&[0x80; 5], &mut p), Err(LebError::TooLong));
        assert_eq!(p, 0);
    }

    #[test]
    fn rejects_bits_beyond_width_unsigned() {
        let mut p = 0;
        assert_eq!(
            read_u32(&[0xFF, 0xFF, 0xFF, 0xFF, 0x1F], &mut p),
            Err(LebError::TooLong)
        );
        let mut p = 0;
        assert_eq!(read_u32(&[0xFF, 0xFF, 0xFF, 0xFF, 0x0F], &mut p), Ok(u32::MAX));
        assert_eq!(p, 5);
    }

    #[test]
    fn signed_final_byte_must_match_sign() {
        let mut p = 0;
        assert_eq!(read_i32(&[0xFF, 0xFF, 0xFF, 0xFF, 0x7F], &mut p), Ok(-1));
        assert_eq!(p, 5);
        let mut p = 0;
        assert_eq!(
            read_i32(&[0xFF, 0xFF, 0xFF, 0xFF, 0x4F], &mut p),
            Err(LebError::TooLong)
        );
        assert_eq!(p, 0);
        let mut p = 0;
        assert_eq!(
            read_i32(&[0xFF, 0xFF, 0xFF, 0xFF, 0x0F], &mut p),
            Err(LebError::TooLong)
        );
        // i32::MIN: the 5th byte is 0x78 (bits 3..6 all equal to the sign bit 1 of
        // bit 3 is 1 and bits 4..6 are 1 -> 0x78 has bits 3..6 set).
        let mut p = 0;
        assert_eq!(read_i32(&[0x80, 0x80, 0x80, 0x80, 0x78], &mut p), Ok(i32::MIN));
    }

    #[test]
    fn signed_64_final_byte_rules() {
        let mut b = enc_i64(i64::MIN);
        assert_eq!(b.len(), 10);
        let mut p = 0;
        assert_eq!(read_i64(&b, &mut p), Ok(i64::MIN));
        // 0x00 as the 10th byte is a valid zero-padded 0; 0x7E gives a value
        // bit 0 of 0 with high bits set, which does not match the sign: invalid.
        b[9] = 0x7E;
        let mut p = 0;
        assert_eq!(read_i64(&b, &mut p), Err(LebError::TooLong));
        assert_eq!(p, 0);
    }

    #[test]
    fn unsigned_64_limits() {
        let mut p = 0;
        assert_eq!(read_u64(&[0xFF; 9].iter().copied().chain([0x01]).collect::<Vec<_>>(), &mut p), Ok(u64::MAX));
        assert_eq!(p, 10);

        let mut p = 0;
        let eleven = [0x80u8; 10].iter().copied().chain([0x00]).collect::<Vec<_>>();
        assert_eq!(read_u64(&eleven, &mut p), Err(LebError::TooLong));
        assert_eq!(p, 0);

        // Bits beyond 64 set in the 10th byte.
        let mut p = 0;
        let bad = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x03];
        assert_eq!(read_u64(&bad, &mut p), Err(LebError::TooLong));
    }

    #[test]
    fn unexpected_end() {
        let mut p = 0;
        assert_eq!(read_u32(&[0x80], &mut p), Err(LebError::UnexpectedEnd));
        assert_eq!(p, 0);
        let mut p = 0;
        assert_eq!(read_u32(&[], &mut p), Err(LebError::UnexpectedEnd));
        let mut p = 0;
        assert_eq!(read_i32(&[], &mut p), Err(LebError::UnexpectedEnd));
        let mut p = 0;
        assert_eq!(read_u64(&[0x80, 0x80], &mut p), Err(LebError::UnexpectedEnd));
        let mut p = 0;
        assert_eq!(read_i64(&[0xC0], &mut p), Err(LebError::UnexpectedEnd));
    }

    #[test]
    fn zero_padded_accepted() {
        let mut p = 0;
        assert_eq!(read_u32(&[0x80, 0x00], &mut p), Ok(0));
        assert_eq!(p, 2);
        let mut p = 0;
        assert_eq!(read_u64(&[0x80, 0x80, 0x00], &mut p), Ok(0));
        assert_eq!(p, 3);
        let mut p = 0;
        assert_eq!(read_i32(&[0x80, 0x00], &mut p), Ok(0));
        assert_eq!(p, 2);
    }

    #[test]
    fn pos_unchanged_on_error() {
        let mut p = 1;
        assert_eq!(read_u32(&[0x01, 0x80], &mut p), Err(LebError::UnexpectedEnd));
        assert_eq!(p, 1);
        let mut p = 1;
        assert_eq!(read_i64(&[0x01, 0x80, 0x80], &mut p), Err(LebError::UnexpectedEnd));
        assert_eq!(p, 1);
    }

    #[test]
    fn reads_from_position() {
        let bytes = [0xAA, 0xE5, 0x8E, 0x26, 0x7F];
        let mut p = 1;
        assert_eq!(read_u32(&bytes, &mut p), Ok(624485));
        assert_eq!(p, 4);
        assert_eq!(read_i32(&bytes, &mut p), Ok(-1));
        assert_eq!(p, 5);
    }

    #[test]
    fn pos_past_end_is_unexpected_end() {
        let bytes = [0x01u8];
        let mut p = 5;
        assert_eq!(read_u32(&bytes, &mut p), Err(LebError::UnexpectedEnd));
        assert_eq!(p, 5);
        let mut p = 1;
        assert_eq!(read_i64(&bytes, &mut p), Err(LebError::UnexpectedEnd));
        assert_eq!(p, 1);
        let mut p = usize::MAX;
        assert_eq!(read_u64(&bytes, &mut p), Err(LebError::UnexpectedEnd));
        assert_eq!(p, usize::MAX);
        let mut p = usize::MAX;
        assert_eq!(read_i32(&[], &mut p), Err(LebError::UnexpectedEnd));
        assert_eq!(p, usize::MAX);
    }

    #[test]
    fn fuzz_readers_never_panic() {
        let mut rng = XorShift(0xDEAD_BEEF_CAFE_F00D);
        for _ in 0..20_000 {
            let len = (rng.next() % 13) as usize;
            let mut bytes = Vec::with_capacity(len);
            for _ in 0..len {
                bytes.push(rng.next() as u8);
            }
            // Bias toward continuation bytes so long encodings are exercised.
            if rng.next() % 2 == 0 {
                for b in bytes.iter_mut() {
                    if rng.next() % 3 != 0 {
                        *b |= CONT;
                    }
                }
            }
            let pos0 = match rng.next() % 8 {
                0 => usize::MAX,
                1 => usize::MAX - 1,
                _ => (rng.next() as usize) % (len + 4),
            };
            let mut p = pos0;
            let _ = read_u32(&bytes, &mut p);
            assert!(p == pos0 || p <= bytes.len());
            let mut p = pos0;
            let _ = read_u64(&bytes, &mut p);
            assert!(p == pos0 || p <= bytes.len());
            let mut p = pos0;
            let _ = read_i32(&bytes, &mut p);
            assert!(p == pos0 || p <= bytes.len());
            let mut p = pos0;
            let _ = read_i64(&bytes, &mut p);
            assert!(p == pos0 || p <= bytes.len());
        }
    }
}
