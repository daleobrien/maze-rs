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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_repr_matches_python() {
        assert_eq!(py_float_repr(600.0), "600.0");
        assert_eq!(py_float_repr(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(py_float_repr(0.5), "0.5");
    }
}
