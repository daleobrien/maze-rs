use std::fmt::Write as _;

pub mod pdf;
pub mod svg;
pub mod text;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PageSize {
    A4,
    A0,
    Letter,
}

/// Page size in points (portrait).
pub fn page_size(page: PageSize) -> (f64, f64) {
    let dpi = 72.0;
    match page {
        PageSize::A4 => (8.3 * dpi, 11.7 * dpi),
        PageSize::A0 => (33.1 * dpi, 46.8 * dpi),
        // US Letter
        PageSize::Letter => (8.5 * dpi, 11.0 * dpi),
    }
}

/// Format a float the way Python's `str(float)` does for the magnitudes used
/// here (shortest round-trip digits, always with a decimal point).
pub fn py_float_repr(x: f64) -> String {
    if x.is_finite() && x.fract() == 0.0 && x.abs() < 1e16 {
        format!("{x:.1}")
    } else {
        format!("{x}")
    }
}

/// Append `x` with `digits` decimal places, exactly as `format!("{x:.digits$}")`
/// would, but without going through the general float formatter.
///
/// The value is scaled by a power of ten using exact integer arithmetic and
/// rounded half to even, which is what the standard library does.
pub fn push_fixed(out: &mut String, x: f64, digits: usize) {
    const POW10: [u128; 7] = [1, 10, 100, 1_000, 10_000, 100_000, 1_000_000];
    if !x.is_finite() || x.abs() >= (1u64 << 53) as f64 || digits >= POW10.len() {
        write!(out, "{x:.digits$}").unwrap();
        return;
    }

    // x = mantissa * 2^exponent
    let bits = x.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1u64 << 52) - 1);
    let (mantissa, exponent) = if biased == 0 { (fraction, -1074) } else { (fraction | (1u64 << 52), biased - 1075) };

    let scaled = u128::from(mantissa) * POW10[digits];
    let q = if exponent >= 0 {
        // |x| < 2^53, so the shift can't overflow
        scaled << exponent
    } else {
        let shift = (-exponent) as u32;
        if shift >= 128 {
            0
        } else {
            let q = scaled >> shift;
            let rem = scaled & ((1u128 << shift) - 1);
            let half = 1u128 << (shift - 1);
            if rem > half || (rem == half && q & 1 == 1) {
                q + 1
            } else {
                q
            }
        }
    };

    if x.is_sign_negative() {
        out.push('-');
    }
    let unit = POW10[digits];
    write!(out, "{}", q / unit).unwrap();
    if digits > 0 {
        write!(out, ".{:0digits$}", q % unit).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_fixed(x: f64) {
        for digits in 0..=6 {
            let mut got = String::new();
            push_fixed(&mut got, x, digits);
            assert_eq!(got, format!("{x:.digits$}"), "x={x:e} digits={digits}");
        }
    }

    #[test]
    fn fixed_matches_std_format() {
        let specials = [0.0, -0.0, 0.5, 1.5, 2.5, -2.5, 0.125, 0.375, 1.005, 2.675, 0.045, 1e-7, 5e-7, 1e-300, 5e-324, 123456.5, 9.995, 99.995, 597.6, -27.028571428571];
        for x in specials {
            check_fixed(x);
        }
        // exact ties at every scale, and pseudo-random values
        for k in 0..20_000i64 {
            check_fixed(k as f64 / 8.0);
            check_fixed(-(k as f64) / 1024.0);
        }
        let mut state = 0x1234_5678_9abc_def0u64;
        for _ in 0..200_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let unit = (state >> 11) as f64 / (1u64 << 53) as f64;
            let scale = [1e-6, 1e-3, 1.0, 37.0, 600.0, 3000.0, 1e6, 1e12][(state & 7) as usize];
            check_fixed((unit - 0.25) * scale);
        }
    }

    #[test]
    fn float_repr_matches_python() {
        assert_eq!(py_float_repr(600.0), "600.0");
        assert_eq!(py_float_repr(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(py_float_repr(0.5), "0.5");
    }
}
