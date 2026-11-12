//! Paths as data, for the styled and ink renderers: walls made of lines and
//! circular arcs, written as SVG path data or as PDF operators, read back
//! from the SVG path data the plain renderer writes, or flattened into
//! polylines.

use std::f64::consts::PI;
use std::fmt::Write as _;

use crate::renderers::{push_fixed_trimmed, push_pair};

pub(crate) type Point = (f64, f64);

/// Decimal places in SVG and in PDF output.
const SVG_DIGITS: usize = 2;
const PDF_DIGITS: usize = 3;

pub(crate) enum Segment {
    Line(Point),
    Arc { radius: f64, large: bool, sweep: bool, to: Point },
}

/// One continuous wall.
pub(crate) struct Chain {
    pub start: Point,
    pub segments: Vec<Segment>,
}

pub(crate) fn distance(p: Point, q: Point) -> f64 {
    (q.0 - p.0).hypot(q.1 - p.1)
}

/// The point `r` along the way from `from` towards `to`.
pub(crate) fn towards(from: Point, to: Point, r: f64) -> Point {
    let d = distance(from, to);
    (from.0 + (to.0 - from.0) * r / d, from.1 + (to.1 - from.1) * r / d)
}

/// `x` as it reads back once written as SVG.
fn svg_rounded(x: f64) -> f64 {
    push_fixed_trimmed(&mut String::new(), x, SVG_DIGITS)
}

pub(crate) fn push_pdf_xy(out: &mut String, p: Point) {
    push_pair(out, p, PDF_DIGITS);
}

pub(crate) fn polyline_chain(points: &[Point]) -> Chain {
    Chain { start: points[0], segments: points[1..].iter().map(|&p| Segment::Line(p)).collect() }
}

impl Chain {
    /// Each point moved by `f`.
    pub(crate) fn map_points(self, f: impl Fn(Point) -> Point) -> Chain {
        Chain {
            start: f(self.start),
            segments: self
                .segments
                .into_iter()
                .map(|segment| match segment {
                    Segment::Line(p) => Segment::Line(f(p)),
                    Segment::Arc { radius, large, sweep, to } => Segment::Arc { radius, large, sweep, to: f(to) },
                })
                .collect(),
        }
    }

    /// The chain with every number as written in SVG, so a tile drawn from
    /// it in PDF or in ink matches its SVG drawing exactly.
    pub(crate) fn svg_rounded(self) -> Chain {
        let mut chain = self.map_points(|(x, y)| (svg_rounded(x), svg_rounded(y)));
        for segment in &mut chain.segments {
            if let Segment::Arc { radius, .. } = segment {
                *radius = svg_rounded(*radius);
            }
        }
        chain
    }

    pub(crate) fn push_svg(&self, out: &mut String) {
        out.push('M');
        push_pair(out, self.start, SVG_DIGITS);
        for segment in &self.segments {
            match *segment {
                Segment::Line(to) => {
                    out.push('L');
                    push_pair(out, to, SVG_DIGITS);
                }
                Segment::Arc { radius, large, sweep, to } => {
                    out.push('A');
                    push_pair(out, (radius, radius), SVG_DIGITS);
                    w!(out, " 0 {} {} ", u8::from(large), u8::from(sweep));
                    push_pair(out, to, SVG_DIGITS);
                }
            }
        }
    }

    pub(crate) fn push_pdf(&self, out: &mut String) {
        push_pdf_xy(out, self.start);
        out.push_str(" m\n");
        let mut at = self.start;
        for segment in &self.segments {
            match *segment {
                Segment::Line(to) => {
                    push_pdf_xy(out, to);
                    out.push_str(" l\n");
                    at = to;
                }
                Segment::Arc { radius, large, sweep, to } => {
                    push_pdf_arc(out, at, to, radius, large, sweep);
                    at = to;
                }
            }
        }
    }
}

/// Parse SVG path data written by this crate (`M`, `L` and `A` commands)
/// back into chains.
pub(crate) fn parse_path(d: &str) -> Vec<Chain> {
    let mut chains: Vec<Chain> = Vec::new();
    let mut command = b'M';
    let mut numbers: Vec<f64> = Vec::with_capacity(7);
    let bytes = d.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphabetic() {
            command = c;
            numbers.clear();
            i += 1;
        } else if c == b'-' || c == b'.' || c.is_ascii_digit() {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i] == b'.' || bytes[i].is_ascii_digit()) {
                i += 1;
            }
            numbers.push(d[start..i].parse().expect("the renderer writes plain numbers"));
            let needed = if command == b'A' { 7 } else { 2 };
            if numbers.len() == needed {
                let to = (numbers[needed - 2], numbers[needed - 1]);
                match (command, chains.last_mut()) {
                    (b'M', _) | (_, None) => chains.push(Chain { start: to, segments: Vec::new() }),
                    (b'A', Some(chain)) => chain.segments.push(Segment::Arc {
                        radius: numbers[0],
                        large: numbers[3] != 0.0,
                        sweep: numbers[4] != 0.0,
                        to,
                    }),
                    (_, Some(chain)) => chain.segments.push(Segment::Line(to)),
                }
                numbers.clear();
            }
        } else {
            i += 1;
        }
    }
    chains
}

/// A circular arc in centre form.
struct CentreArc {
    cx: f64,
    cy: f64,
    r: f64,
    /// The angle of the start, and how far the arc turns (negative when
    /// anticlockwise), in radians.
    theta: f64,
    delta: f64,
}

/// An SVG circular arc from `from` to `to` (SVG 1.1 F.6.5, with rx = ry and
/// no rotation) in centre form, or `None` if its ends are the same point.
fn centre_arc(from: Point, to: Point, radius: f64, large: bool, sweep: bool) -> Option<CentreArc> {
    let (x1p, y1p) = ((from.0 - to.0) / 2.0, (from.1 - to.1) / 2.0);
    let d2 = x1p * x1p + y1p * y1p;
    if d2 < 1e-12 {
        return None;
    }
    // the radius was rounded when written, so it may be a touch too small
    let r = radius.max(d2.sqrt());
    let sign = if large == sweep { -1.0 } else { 1.0 };
    let coef = sign * ((r * r - d2) / d2).max(0.0).sqrt();
    let (cxp, cyp) = (coef * y1p, -coef * x1p);
    let (cx, cy) = (cxp + (from.0 + to.0) / 2.0, cyp + (from.1 + to.1) / 2.0);
    let theta = (y1p - cyp).atan2(x1p - cxp);
    let mut delta = (-y1p - cyp).atan2(-x1p - cxp) - theta;
    if sweep && delta < 0.0 {
        delta += 2.0 * PI;
    } else if !sweep && delta > 0.0 {
        delta -= 2.0 * PI;
    }
    Some(CentreArc { cx, cy, r, theta, delta })
}

/// An SVG circular arc as PDF Bezier curves.
fn push_pdf_arc(out: &mut String, from: Point, to: Point, radius: f64, large: bool, sweep: bool) {
    let Some(CentreArc { cx, cy, r, theta, delta }) = centre_arc(from, to, radius, large, sweep) else {
        return;
    };
    let n = (delta.abs() / (PI / 2.0)).ceil().max(1.0) as usize;
    let step = delta / n as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan() * r;
    for i in 0..n {
        let (t0, t1) = (theta + step * i as f64, theta + step * (i + 1) as f64);
        let p0 = (cx + r * t0.cos(), cy + r * t0.sin());
        let p3 = if i + 1 == n { to } else { (cx + r * t1.cos(), cy + r * t1.sin()) };
        let c1 = (p0.0 - k * t0.sin(), p0.1 + k * t0.cos());
        let c2 = (p3.0 + k * t1.sin(), p3.1 - k * t1.cos());
        for p in [c1, c2, p3] {
            push_pdf_xy(out, p);
            out.push(' ');
        }
        out.push_str("c\n");
    }
}

/// Append a filled circle to a PDF content stream.
pub(crate) fn push_pdf_circle(out: &mut String, (cx, cy): Point, r: f64) {
    let start = (cx + r, cy);
    push_pdf_xy(out, start);
    out.push_str(" m\n");
    push_pdf_arc(out, start, (cx - r, cy), r, false, true);
    push_pdf_arc(out, (cx - r, cy), start, r, false, true);
    out.push_str("f\n");
}

/// Flatten chains into polylines whose pieces are at most `step` long,
/// leaving out any that draw nothing.
pub(crate) fn flatten(chains: &[Chain], step: f64) -> Vec<Vec<Point>> {
    let mut lines = Vec::with_capacity(chains.len());
    for chain in chains {
        let mut line = vec![chain.start];
        for segment in &chain.segments {
            let from = *line.last().expect("lines start with a point");
            match *segment {
                Segment::Line(to) => line_points(&mut line, from, to, step),
                Segment::Arc { radius, large, sweep, to } => {
                    arc_points(&mut line, from, to, radius, large, sweep, step);
                }
            }
        }
        if line.len() > 1 {
            lines.push(line);
        }
    }
    lines
}

fn line_points(out: &mut Vec<Point>, from: Point, to: Point, step: f64) {
    let length = distance(from, to);
    if length < 1e-9 {
        return;
    }
    let n = (length / step).ceil().max(1.0) as usize;
    for k in 1..=n {
        let t = k as f64 / n as f64;
        out.push((from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t));
    }
}

fn arc_points(out: &mut Vec<Point>, from: Point, to: Point, radius: f64, large: bool, sweep: bool, step: f64) {
    let Some(CentreArc { cx, cy, r, theta, delta }) = centre_arc(from, to, radius, large, sweep) else {
        return;
    };
    let n = (delta.abs() * r / step).ceil().max(2.0) as usize;
    for k in 1..n {
        let angle = theta + delta * k as f64 / n as f64;
        out.push((cx + r * angle.cos(), cy + r * angle.sin()));
    }
    out.push(to);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arcs_end_where_they_should() {
        // a quarter circle of radius 10 from (10, 0) to (0, 10) about the origin
        let lines = flatten(&parse_path("M 10 0 A 10 10 0 0 1 0 10"), 1.0);
        let line = &lines[0];
        assert_eq!(*line.last().unwrap(), (0.0, 10.0));
        for &(x, y) in line {
            assert!((x.hypot(y) - 10.0).abs() < 1e-9, "({x}, {y}) is off the circle");
        }
    }

    #[test]
    fn written_paths_read_back() {
        let d = "M80 0A20 20 0 0 0 100 20M100 80L20 0";
        let mut again = String::new();
        for chain in parse_path(d) {
            chain.push_svg(&mut again);
        }
        assert_eq!(again, d);
    }
}
