//! A hand-drawn "ink" style for the SVG and PDF output.
//!
//! The maze has the same geometry as the plain renderers (it is taken from
//! [`path_data`]), but every wall is flattened to short pieces, nudged by a
//! smooth noise field and drawn with a pen whose width wanders, in brown ink
//! on paper. The noise is seeded from the maze id, so a maze always comes out
//! the same, and depends only on where a point is on the page, so walls that
//! meet at the edge of a cell still meet after the wobble.
//!
//! Because every cell wobbles differently, the tiles can't be drawn once and
//! reused, so this style is bigger and slower than the plain one. It is only
//! used when asked for, and leaves the plain output untouched.

use std::f64::consts::PI;
use std::fmt::Write as _;

use crate::maze::Grid;
use crate::renderers::pdf::{pdf_date, zlib_compress, PageLayout, PdfOptions};
use crate::renderers::svg::{is_dot, path_data, svg_scale};
use crate::renderers::{map_bands, push_fixed_trimmed, py_float_repr, PageSize};

type Point = (f64, f64);

/// The paper, the ink and the pencil used for the solution.
const PAPER: &str = "#faf6ec";
const INK: &str = "#2b2118";
const PENCIL: &str = "#c0392b";
const PAPER_RGB: [f64; 3] = [0.98, 0.965, 0.925];
const INK_RGB: [f64; 3] = [0.169, 0.129, 0.094];
const DOT_RGB: [f64; 3] = [0.753, 0.224, 0.169];

/// How many pieces of a wall are drawn with one width before the pen changes.
const CHUNK: usize = 6;

/// The pen widths, as fractions of the middle one.
const WIDTHS: [f64; 3] = [0.8, 1.0, 1.22];

/// Different noise fields from the one seed.
const SALT_X: u64 = 0x5851_F42D_4C95_7F2D;
const SALT_Y: u64 = 0x1405_7B7E_F767_814F;
const SALT_WIDTH: u64 = 0x2545_F491_4F6C_DD1D;
const SALT_PENCIL: [u64; 3] = [0x9E37_79B9_7F4A_7C15, 0xBF58_476D_1CE4_E5B9, 0x94D0_49BB_1331_11EB];

/// A seed for the wobble from a maze id (64-bit FNV-1a).
#[must_use]
pub fn seed_from_id(id: &str) -> u64 {
    id.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

// ---- noise ----

/// A value in [-1, 1) for each lattice point.
fn lattice(ix: i64, iy: i64, seed: u64) -> f64 {
    // splitmix64's finaliser over the mixed coordinates
    let mut z =
        seed ^ (ix as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (iy as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 52) as f64 - 1.0
}

/// Smooth value noise in [-1, 1].
fn noise(x: f64, y: f64, seed: u64) -> f64 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (ix, iy) = (x0 as i64, y0 as i64);
    let (u, v) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let top = lattice(ix, iy, seed) * (1.0 - u) + lattice(ix + 1, iy, seed) * u;
    let bottom = lattice(ix, iy + 1, seed) * (1.0 - u) + lattice(ix + 1, iy + 1, seed) * u;
    top * (1.0 - v) + bottom * v
}

/// How the walls are drawn, scaled to cells of side `s`.
struct Pen {
    seed: u64,
    /// How far a point may be nudged.
    amplitude: f64,
    /// The size of a wobble.
    feature: f64,
    /// The longest piece a line or arc is flattened into.
    step: f64,
}

impl Pen {
    fn new(s: f64, seed: u64) -> Self {
        Pen { seed, amplitude: s * 0.035, feature: s * 0.45, step: s / 10.0 }
    }

    fn wobble(&self, (x, y): Point, salt: u64) -> Point {
        let (nx, ny) = (x / self.feature, y / self.feature);
        (
            x + self.amplitude * noise(nx, ny, self.seed ^ salt ^ SALT_X),
            y + self.amplitude * noise(nx, ny, self.seed ^ salt ^ SALT_Y),
        )
    }

    /// Which of the [`WIDTHS`] the pen has at `p`.
    fn width_at(&self, (x, y): Point) -> usize {
        let n = noise(x / (self.feature * 2.0), y / (self.feature * 2.0), self.seed ^ SALT_WIDTH);
        if n < -0.2 {
            0
        } else if n > 0.2 {
            2
        } else {
            1
        }
    }
}

// ---- reading SVG path data ----

/// Flatten SVG path data (only the `M`, `L` and `A` commands the plain
/// renderer writes) into polylines whose pieces are at most `step` long.
fn flatten(d: &str, step: f64) -> Vec<Vec<Point>> {
    let mut lines: Vec<Vec<Point>> = Vec::new();
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
            continue;
        }
        if c == b'-' || c == b'.' || c.is_ascii_digit() {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i] == b'.' || bytes[i].is_ascii_digit()) {
                i += 1;
            }
            numbers.push(d[start..i].parse().expect("the renderer writes plain numbers"));
            let needed = if command == b'A' { 7 } else { 2 };
            if numbers.len() == needed {
                let to = (numbers[needed - 2], numbers[needed - 1]);
                match (command, lines.last_mut()) {
                    (b'M', _) | (_, None) => lines.push(vec![to]),
                    (b'A', Some(line)) => {
                        let from = *line.last().expect("lines start with a point");
                        arc_points(line, from, to, numbers[0], numbers[3] != 0.0, numbers[4] != 0.0, step);
                    }
                    (_, Some(line)) => {
                        let from = *line.last().expect("lines start with a point");
                        line_points(line, from, to, step);
                    }
                }
                numbers.clear();
            }
            continue;
        }
        i += 1;
    }
    lines.retain(|line| line.len() > 1);
    lines
}

fn line_points(out: &mut Vec<Point>, from: Point, to: Point, step: f64) {
    let length = (to.0 - from.0).hypot(to.1 - from.1);
    if length < 1e-9 {
        return;
    }
    let n = (length / step).ceil().max(1.0) as usize;
    for k in 1..=n {
        let t = k as f64 / n as f64;
        out.push((from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t));
    }
}

/// An SVG circular arc from `from` to `to` (SVG 1.1 F.6.5, with rx = ry
/// and no rotation).
fn arc_points(out: &mut Vec<Point>, from: Point, to: Point, radius: f64, large: bool, sweep: bool, step: f64) {
    let (x1p, y1p) = ((from.0 - to.0) / 2.0, (from.1 - to.1) / 2.0);
    let d2 = x1p * x1p + y1p * y1p;
    if d2 < 1e-12 {
        return;
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
    let n = (delta.abs() * r / step).ceil().max(2.0) as usize;
    for k in 1..n {
        let angle = theta + delta * k as f64 / n as f64;
        out.push((cx + r * angle.cos(), cy + r * angle.sin()));
    }
    out.push(to);
}

/// A closed polyline around a circle.
fn circle(cx: f64, cy: f64, r: f64, step: f64) -> Vec<Point> {
    let n = (2.0 * PI * r / step).ceil().max(12.0) as usize;
    (0..=n)
        .map(|k| {
            let angle = 2.0 * PI * k as f64 / n as f64;
            (cx + r * angle.cos(), cy + r * angle.sin())
        })
        .collect()
}

// ---- writing ----

#[derive(Clone, Copy)]
enum Format {
    Svg,
    Pdf,
}

fn push_point(out: &mut String, (x, y): Point, format: Format, first: bool) {
    let digits = match format {
        Format::Svg => 1,
        Format::Pdf => 2,
    };
    if let (Format::Svg, true) = (format, first) {
        out.push('M');
    } else if let Format::Svg = format {
        out.push(' ');
    }
    push_fixed_trimmed(out, x, digits);
    out.push(' ');
    push_fixed_trimmed(out, y, digits);
    if let Format::Pdf = format {
        out.push_str(if first { " m\n" } else { " l\n" });
    } else if first {
        out.push('L');
    }
}

/// Append a polyline as path data.
fn push_polyline(out: &mut String, points: &[Point], format: Format) {
    for (k, &p) in points.iter().enumerate() {
        push_point(out, p, format, k == 0);
    }
}

/// The walls, wobbled, as path data for each pen width.
fn walls(grid: &Grid, s: f64, pen: &Pen, tiles: &[Vec<Vec<Point>>], format: Format) -> [String; 3] {
    let bands = map_bands(grid.height(), grid.width() * 8, |rows| {
        let mut out: [String; 3] = Default::default();
        let mut wobbled = Vec::new();
        for j in rows {
            for (i, &cell) in grid.row(j).iter().enumerate() {
                if is_dot(grid, i, j) {
                    continue;
                }
                let (x0, y0) = (i as f64 * s, j as f64 * s);
                for line in &tiles[usize::from(cell)] {
                    wobbled.clear();
                    wobbled.extend(line.iter().map(|&(x, y)| pen.wobble((x0 + x, y0 + y), 0)));
                    // consecutive chunks share a point, so the pen never lifts
                    let mut start = 0;
                    while start + 1 < wobbled.len() {
                        let end = (start + CHUNK).min(wobbled.len() - 1);
                        let width = pen.width_at(wobbled[(start + end) / 2]);
                        push_polyline(&mut out[width], &wobbled[start..=end], format);
                        start = end;
                    }
                }
            }
        }
        out
    });
    let mut out: [String; 3] = Default::default();
    for band in bands {
        for (all, part) in out.iter_mut().zip(band) {
            all.push_str(&part);
        }
    }
    out
}

/// Each kind of tile's walls, flattened, in the tile's own coordinates.
fn flat_tiles(tiles: &[String], step: f64) -> Vec<Vec<Vec<Point>>> {
    tiles.iter().map(|d| flatten(d, step)).collect()
}

fn base_width(s: f64) -> f64 {
    s * 0.075
}

/// Render the maze (and optionally its solution) as a hand-drawn SVG.
#[must_use]
pub fn ink_svg(
    grid: &Grid,
    page: PageSize,
    draw_with_curves: bool,
    solution: Option<&[(isize, isize)]>,
    seed: u64,
) -> String {
    let (width, s) = svg_scale(grid, page);
    let pen = Pen::new(s, seed);
    let data = path_data(grid, s, draw_with_curves, solution);
    let tiles = flat_tiles(&data.tiles, pen.step);

    let w = py_float_repr(width * s);
    let h = py_float_repr(grid.height() as f64 * s);
    let mut out = String::new();
    write!(
        out,
        r##"<svg id="maze" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}" width="{w}px" height="{h}px">"##
    )
    .expect("writing to a String cannot fail");
    // a faint grain over the paper
    write!(
        out,
        r##"<defs><filter id="grain" x="0" y="0" width="100%" height="100%"><feTurbulence type="fractalNoise" baseFrequency="0.85" numOctaves="2" seed="{}"/><feColorMatrix values="0 0 0 0 0.42 0 0 0 0 0.36 0 0 0 0 0.25 0 0 0 0.08 0"/></filter></defs><rect width="100%" height="100%" fill="{PAPER}"/><rect width="100%" height="100%" filter="url(#grain)"/>"##,
        seed % 10_000
    )
    .expect("writing to a String cannot fail");

    let base = base_width(s);
    write!(out, r#"<g fill="none" stroke="{INK}" stroke-linecap="round" stroke-linejoin="round">"#)
        .expect("writing to a String cannot fail");
    for (factor, d) in WIDTHS.iter().zip(walls(grid, s, &pen, &tiles, Format::Svg)) {
        if !d.is_empty() {
            out.push_str(r#"<path stroke-width=""#);
            push_fixed_trimmed(&mut out, base * factor, 2);
            write!(out, r#"" d="{d}"/>"#).expect("writing to a String cannot fail");
        }
    }
    out.push_str("</g>");

    // the start and end, inked in
    out.push_str(r#"<path fill=""#);
    out.push_str(PENCIL);
    out.push_str(r#"" fill-opacity="0.85" stroke=""#);
    out.push_str(INK);
    out.push_str(r#"" stroke-width=""#);
    push_fixed_trimmed(&mut out, base * 0.75, 2);
    out.push_str(r#"" d=""#);
    for &(cx, cy, r) in &data.dots {
        let points: Vec<Point> = circle(cx, cy, r * 0.8, pen.step).into_iter().map(|p| pen.wobble(p, 0)).collect();
        push_polyline(&mut out, &points, Format::Svg);
        out.push('Z');
    }
    out.push_str(r#""/>"#);

    // the solution, gone over twice in pencil, and once lightly where it's
    // under a crossing
    if !data.over.is_empty() {
        let over = flatten(&data.over, pen.step);
        let under = flatten(&data.under, pen.step);
        let passes = [
            (&over, SALT_PENCIL[0], 2.2, "0.45"),
            (&over, SALT_PENCIL[1], 1.5, "0.45"),
            (&under, SALT_PENCIL[2], 1.5, "0.18"),
        ];
        write!(out, r#"<g fill="none" stroke="{PENCIL}" stroke-linecap="round" stroke-linejoin="round">"#)
            .expect("writing to a String cannot fail");
        for (lines, salt, factor, opacity) in passes {
            write!(out, r#"<path stroke-opacity="{opacity}" stroke-width=""#).expect("writing to a String cannot fail");
            push_fixed_trimmed(&mut out, base * factor, 2);
            out.push_str(r#"" d=""#);
            for line in lines {
                let points: Vec<Point> = line.iter().map(|&p| pen.wobble(p, salt)).collect();
                push_polyline(&mut out, &points, Format::Svg);
            }
            out.push_str(r#""/>"#);
        }
        out.push_str("</g>");
    }
    out.push_str("</svg>");
    out
}

fn push_rgb(out: &mut String, rgb: [f64; 3], op: &str) {
    for c in rgb {
        push_fixed_trimmed(out, c, 3);
        out.push(' ');
    }
    out.push_str(op);
    out.push('\n');
}

/// The uncompressed page content stream of a hand-drawn PDF.
#[must_use]
pub fn ink_pdf_content(grid: &Grid, options: &PdfOptions, seed: u64) -> String {
    let PageLayout { page_width, page_height, s, left_margin, top_margin } = PageLayout::new(grid, options);
    let pen = Pen::new(s, seed);
    let data = path_data(grid, s, options.draw_with_curves, None);
    let tiles = flat_tiles(&data.tiles, pen.step);

    let mut out = String::new();
    push_rgb(&mut out, PAPER_RGB, "rg");
    out.push_str("0 0 ");
    push_fixed_trimmed(&mut out, page_width, 3);
    out.push(' ');
    push_fixed_trimmed(&mut out, page_height, 3);
    out.push_str(" re f\n1 J 1 j\n");
    // the maze is drawn as in the SVG, top down, so flip it onto the page
    out.push_str("1 0 0 -1 ");
    push_fixed_trimmed(&mut out, left_margin, 3);
    out.push(' ');
    push_fixed_trimmed(&mut out, top_margin + grid.height() as f64 * s, 3);
    out.push_str(" cm\n");
    push_rgb(&mut out, INK_RGB, "RG");

    let base = base_width(s);
    for (factor, d) in WIDTHS.iter().zip(walls(grid, s, &pen, &tiles, Format::Pdf)) {
        push_fixed_trimmed(&mut out, base * factor, 3);
        out.push_str(" w\n");
        out.push_str(&d);
        out.push_str("S\n");
    }

    push_rgb(&mut out, DOT_RGB, "rg");
    push_fixed_trimmed(&mut out, base * 0.75, 3);
    out.push_str(" w\n");
    for &(cx, cy, r) in &data.dots {
        let points: Vec<Point> = circle(cx, cy, r * 0.8, pen.step).into_iter().map(|p| pen.wobble(p, 0)).collect();
        push_polyline(&mut out, &points, Format::Pdf);
        out.push_str("b\n");
    }
    out
}

/// Render the maze as a complete, hand-drawn single-page PDF document.
#[must_use]
pub fn ink_pdf(grid: &Grid, options: &PdfOptions, seed: u64) -> Vec<u8> {
    let content = ink_pdf_content(grid, options, seed);
    let stream = zlib_compress(content.as_bytes());
    let layout = PageLayout::new(grid, options);

    let date = pdf_date();
    let mut media_box = String::from("0 0 ");
    push_fixed_trimmed(&mut media_box, layout.page_width, 3);
    media_box.push(' ');
    push_fixed_trimmed(&mut media_box, layout.page_height, 3);
    let page = format!("<<\n/Contents 5 0 R /MediaBox [ {media_box} ] /Parent 2 0 R /Resources <<\n>> /Type /Page\n>>");
    let info = format!(
        "<<\n/Author (Dale O'Brien) /CreationDate ({date}) /Creator (maze-rs) /ModDate ({date}) /Producer (maze-rs) /Title (Maze)\n>>"
    );
    let contents = [
        format!("<<\n/Filter /FlateDecode /Length {}\n>>\nstream\n", stream.len()).as_bytes(),
        &stream,
        b"\nendstream",
    ]
    .concat();
    let objects: [&[u8]; 5] = [
        b"<<\n/Pages 2 0 R /Type /Catalog\n>>",
        b"<<\n/Count 1 /Kids [ 3 0 R ] /Type /Pages\n>>",
        page.as_bytes(),
        info.as_bytes(),
        &contents,
    ];

    let mut out: Vec<u8> = b"%PDF-1.4\n%\x93\x8c\x8b\x9e maze-rs generated PDF document\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    let size = objects.len() + 1;
    let mut table = format!("xref\n0 {size}\n0000000000 65535 f \n");
    for offset in offsets {
        writeln!(table, "{offset:010} 00000 n ").expect("writing to a String cannot fail");
    }
    write!(table, "trailer\n<<\n/Info 4 0 R\n/Root 1 0 R\n/Size {size}\n>>\nstartxref\n{xref}\n%%EOF\n")
        .expect("writing to a String cannot fail");
    out.extend_from_slice(table.as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_smooth_and_bounded() {
        for k in 0..1000 {
            let (x, y) = (k as f64 * 0.137, k as f64 * 0.071);
            let n = noise(x, y, 7);
            assert!((-1.0..=1.0).contains(&n));
            assert!((noise(x + 1e-6, y, 7) - n).abs() < 1e-3);
        }
    }

    #[test]
    fn arcs_end_where_they_should() {
        // a quarter circle of radius 10 from (10, 0) to (0, 10) about the origin
        let lines = flatten("M 10 0 A 10 10 0 0 1 0 10", 1.0);
        let line = &lines[0];
        assert_eq!(*line.last().unwrap(), (0.0, 10.0));
        for &(x, y) in line {
            assert!((x.hypot(y) - 10.0).abs() < 1e-9, "({x}, {y}) is off the circle");
        }
    }

    #[test]
    fn same_seed_same_drawing() {
        let mut rng = crate::random::Rng::from_str_seed("ink");
        let grid = crate::maze::create_maze(10, 12, 50, false, &mut rng);
        let a = ink_svg(&grid, PageSize::A4, true, None, seed_from_id("ink"));
        assert_eq!(a, ink_svg(&grid, PageSize::A4, true, None, seed_from_id("ink")));
        assert_ne!(a, ink_svg(&grid, PageSize::A4, true, None, seed_from_id("other")));
    }
}
