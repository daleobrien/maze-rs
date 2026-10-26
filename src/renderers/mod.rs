use std::fmt::Write as _;
use std::num::NonZeroUsize;
use std::ops::Range;

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
#[must_use]
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
#[must_use]
pub fn py_float_repr(x: f64) -> String {
    if x.is_finite() && x.fract() == 0.0 && x.abs() < 1e16 {
        format!("{x:.1}")
    } else {
        format!("{x}")
    }
}

/// Below this much work (items times cost per item) starting threads costs
/// more than it saves.
const PARALLEL_MIN_WORK: usize = 4096;

/// Split `0..len` into consecutive bands, run `f` on each (in parallel when
/// there is enough work) and return the results in order.
pub(crate) fn map_bands<T: Send>(len: usize, cost_per_item: usize, f: impl Fn(Range<usize>) -> T + Sync) -> Vec<T> {
    let threads = std::thread::available_parallelism().map_or(1, NonZeroUsize::get);
    if threads == 1 || len * cost_per_item < PARALLEL_MIN_WORK {
        return vec![f(0..len)];
    }
    let bands = threads.min(len);
    let band = |k: usize| k * len / bands..(k + 1) * len / bands;
    std::thread::scope(|scope| {
        let f = &f;
        let handles: Vec<_> = (1..bands).map(|k| scope.spawn(move || f(band(k)))).collect();
        let mut results = vec![f(band(0))];
        results.extend(handles.into_iter().map(|h| h.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))));
        results
    })
}

/// The measurements of a tile with sides `s`, shared by the SVG and PDF
/// renderers and named as in the Python version: `g` is the gap between a
/// wall and the cell's edge, and the rest place the three arcs that make a
/// dead end's rounded cap.
pub(crate) struct Geometry {
    pub s: f64,
    pub g: f64,
    pub n: f64,
    pub r: f64,
    pub q: f64,
    pub v: f64,
    /// In radians.
    pub theta: f64,
}

impl Geometry {
    pub fn new(s: f64) -> Self {
        let g = s * 0.2;
        let k = 0.5;

        let n = -(g / k) + 0.5 * (s - ((g * (4.0 * g - 3.0 * g * k + 2.0 * k * s)) / k).sqrt());

        let r = g / k;
        let q = n + r;
        let v = (g * (-1.0 + k)) / k;

        let theta = ((2.0 * g - 2.0 * g * k + k * s) / (2.0 * g - g * k + k * s)).asin();

        Geometry { s, g, n, r, q, v, theta }
    }
}

/// Append `x` with `digits` decimal places, exactly as `format!("{x:.digits$}")`
/// would, but without going through the general float formatter.
///
/// The value is scaled by a power of ten using exact integer arithmetic and
/// rounded half to even, which is what the standard library does.
pub fn push_fixed(out: &mut String, x: f64, digits: usize) {
    push_fixed_impl(out, x, digits, false);
}

/// Like [`push_fixed`], then strips trailing zeros after the decimal point
/// (and the point itself if nothing is left after it).
///
/// Returns the value of the written number, i.e. what parsing it back would
/// give.
pub fn push_fixed_trimmed(out: &mut String, x: f64, digits: usize) -> f64 {
    push_fixed_impl(out, x, digits, true)
}

fn push_fixed_impl(out: &mut String, x: f64, digits: usize, trim: bool) -> f64 {
    const POW10: [u64; 7] = [1, 10, 100, 1_000, 10_000, 100_000, 1_000_000];
    // beyond this the scaled value might not fit in 53 bits
    if !x.is_finite() || x.abs() >= 1e9 || digits >= POW10.len() {
        return push_fixed_slow(out, x, digits, trim);
    }

    // x = mantissa * 2^exponent
    let bits = x.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1u64 << 52) - 1);
    let (mantissa, exponent) = if biased == 0 { (fraction, -1074) } else { (fraction | (1u64 << 52), biased - 1075) };

    let unit = POW10[digits];
    let scaled = u128::from(mantissa) * u128::from(unit);
    let q = if exponent >= 0 {
        // |x| < 1e9, so the shift can't overflow
        scaled << exponent
    } else {
        let shift = exponent.unsigned_abs();
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
    } as u64;

    // digits are written backwards into a small buffer
    let mut buf = [0u8; 32];
    let mut pos = buf.len();
    let (mut int_part, mut frac) = (q / unit, q % unit);
    let mut frac_digits = digits;
    if trim {
        while frac_digits > 0 && frac % 10 == 0 {
            frac /= 10;
            frac_digits -= 1;
        }
    }
    if frac_digits > 0 {
        for _ in 0..frac_digits {
            pos -= 1;
            buf[pos] = b'0' + (frac % 10) as u8;
            frac /= 10;
        }
        pos -= 1;
        buf[pos] = b'.';
    }
    loop {
        pos -= 1;
        buf[pos] = b'0' + (int_part % 10) as u8;
        int_part /= 10;
        if int_part == 0 {
            break;
        }
    }
    if x.is_sign_negative() {
        pos -= 1;
        buf[pos] = b'-';
    }
    out.push_str(std::str::from_utf8(&buf[pos..]).expect("only ASCII was written"));

    // q and unit are exact, so this division rounds the decimal just like
    // parsing it would
    let value = q as f64 / unit as f64;
    if x.is_sign_negative() {
        -value
    } else {
        value
    }
}

fn push_fixed_slow(out: &mut String, x: f64, digits: usize, trim: bool) -> f64 {
    let start = out.len();
    write!(out, "{x:.digits$}").expect("writing to a String cannot fail");
    if trim && out[start..].contains('.') {
        while out.ends_with('0') {
            out.pop();
        }
        if out.ends_with('.') {
            out.pop();
        }
    }
    out[start..].parse().unwrap_or(x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_fixed(x: f64) {
        for digits in 0..=6 {
            let mut got = String::new();
            push_fixed(&mut got, x, digits);
            let want = format!("{x:.digits$}");
            assert_eq!(got, want, "x={x:e} digits={digits}");

            let mut trimmed = String::new();
            let value = push_fixed_trimmed(&mut trimmed, x, digits);
            let want = if want.contains('.') { want.trim_end_matches('0').trim_end_matches('.') } else { &want };
            assert_eq!(trimmed, want, "x={x:e} digits={digits}");
            assert_eq!(value.to_bits(), trimmed.parse::<f64>().unwrap().to_bits(), "x={x:e} digits={digits}");
        }
    }

    #[test]
    fn fixed_matches_std_format() {
        let specials = [
            0.0,
            -0.0,
            0.5,
            1.5,
            2.5,
            -2.5,
            0.125,
            0.375,
            1.005,
            2.675,
            0.045,
            1e-7,
            5e-7,
            1e-300,
            5e-324,
            123456.5,
            9.995,
            99.995,
            597.6,
            -27.028571428571,
        ];
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
