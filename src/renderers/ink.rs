//! A hand-drawn "ink" style for the SVG and PDF output.
//!
//! The maze has the same geometry as the plain SVG renderer (or the
//! style's, when it reshapes the walls), but every wall is flattened to
//! short pieces, nudged by a smooth noise field and drawn with a pen whose
//! width wanders, in brown ink on a transparent background. The noise is
//! seeded from the maze id, so a maze always comes out the same, and depends
//! only on where a point is on the page, so walls that meet at the edge of a
//! cell still meet after the wobble.
//!
//! Because every cell wobbles differently, the tiles can't be drawn once and
//! reused, so this style is bigger and slower than the plain one. It is only
//! used when asked for, and leaves the plain output untouched.

use std::f64::consts::PI;
use std::fmt::Write as _;

use crate::maze::Grid;
use crate::renderers::frame::{pdf_background, pdf_document, pdf_flip, pdf_frame, pdf_layout, svg_frame, svg_open};
use crate::renderers::path::{flatten, parse_path, Chain, Point};
use crate::renderers::pdf::PdfOptions;
use crate::renderers::style::{Colour, Pens, Style};
use crate::renderers::svg::{dots, is_dot, plain_tiles, svg_scale, Traces};
use crate::renderers::{map_bands, push_fixed_trimmed, push_pair, used_tiles, PageSize, Solution};

/// The ink and the pencil used for the solution.
const INK: Colour = Colour([0x2b, 0x21, 0x18]);
const PENCIL: Colour = Colour([0xc0, 0x39, 0x2b]);

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

/// Append a polyline as path data.
fn push_polyline(out: &mut String, points: &[Point], format: Format) {
    for (k, &p) in points.iter().enumerate() {
        match format {
            Format::Svg => {
                out.push(if k == 0 { 'M' } else { ' ' });
                push_pair(out, p, 1);
                if k == 0 {
                    out.push('L');
                }
            }
            Format::Pdf => {
                push_pair(out, p, 2);
                out.push_str(if k == 0 { " m\n" } else { " l\n" });
            }
        }
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

/// Everything an inked maze is drawn from, for cells of side `s`.
struct Ink {
    pen: Pen,
    /// Each kind of tile's walls, flattened, in the tile's own coordinates.
    tiles: Vec<Vec<Vec<Point>>>,
    /// The solution, flattened, where it is on top and where it is under a
    /// crossing.
    over: Vec<Vec<Point>>,
    under: Vec<Vec<Point>>,
    /// The width of the middle pen, and the colours.
    pens: Pens,
}

impl Ink {
    fn new(grid: &Grid, s: f64, draw_with_curves: bool, solution: Option<Solution>, seed: u64, style: &Style) -> Self {
        let pen = Pen::new(s, seed);
        let curves = style.curves(draw_with_curves);
        // the plain renderer's walls, unless the style changes their shape
        let used = used_tiles(grid, |i, j| is_dot(grid, i, j));
        let chains: Vec<Vec<Chain>> = if style.custom_shape() {
            style.tiles(s, draw_with_curves, &used)
        } else {
            plain_tiles(s, curves, &used).iter().map(|d| parse_path(d)).collect()
        };
        let traces = Traces::new(grid, s, curves, solution);
        Ink {
            tiles: chains.iter().map(|chains| flatten(chains, pen.step)).collect(),
            over: flatten(&parse_path(&traces.over), pen.step),
            under: flatten(&parse_path(&traces.under), pen.step),
            // the middle pen is 7.5% of a cell wide
            pens: style.pens(Pens { line: s * 0.075, wall: INK, solution: PENCIL }),
            pen,
        }
    }

    /// The start and end dots, as wobbled closed polylines.
    fn dots(&self, grid: &Grid, s: f64) -> Vec<Vec<Point>> {
        dots(grid, s)
            .into_iter()
            .map(|(cx, cy, r)| {
                circle(cx, cy, r * 0.8, self.pen.step).into_iter().map(|p| self.pen.wobble(p, 0)).collect()
            })
            .collect()
    }

    /// A pencil pass over the solution, `lines`, wobbled with `salt`.
    fn push_pass(&self, out: &mut String, lines: &[Vec<Point>], salt: u64, format: Format) {
        push_pass(out, &self.pen, lines, salt, format);
    }
}

/// A pencil pass over `lines`, wobbled with `salt`.
fn push_pass(out: &mut String, pen: &Pen, lines: &[Vec<Point>], salt: u64, format: Format) {
    for line in lines {
        let points: Vec<Point> = line.iter().map(|&p| pen.wobble(p, salt)).collect();
        push_polyline(out, &points, format);
    }
}

/// Append a trail in pencil, as an SVG group: gone over twice where it is on
/// top, and once lightly where it passes under a crossing. `base` is the width
/// of the middle pen and `colour` a `#rrggbb`.
fn push_pencil(out: &mut String, pen: &Pen, over: &[Vec<Point>], under: &[Vec<Point>], base: f64, colour: &str) {
    let passes = [
        (over, SALT_PENCIL[0], 2.2, "0.45"),
        (over, SALT_PENCIL[1], 1.5, "0.45"),
        (under, SALT_PENCIL[2], 1.5, "0.18"),
    ];
    w!(out, r#"<g fill="none" stroke="{colour}" stroke-linecap="round" stroke-linejoin="round">"#);
    for (lines, salt, factor, opacity) in passes {
        w!(out, r#"<path stroke-opacity="{opacity}" stroke-width=""#);
        push_fixed_trimmed(out, base * factor, 2);
        out.push_str(r#"" d=""#);
        push_pass(out, pen, lines, salt, Format::Svg);
        out.push_str(r#""/>"#);
    }
    out.push_str("</g>");
}

/// Render the maze (and optionally its solution) as a hand-drawn SVG.
#[must_use]
pub fn ink_svg(
    grid: &Grid,
    page: PageSize,
    draw_with_curves: bool,
    solution: Option<Solution>,
    seed: u64,
    style: &Style,
) -> String {
    let (width, s) = svg_scale(grid, page);
    ink_svg_sized(grid, width * s, s, draw_with_curves, solution, seed, style)
}

/// Render the maze (and optionally its solution) as a hand-drawn SVG with
/// cells of side `s`, exactly as wide as the maze rather than fitted to a page:
/// cell `(x, y)` covers `x * s` to `(x + 1) * s` across and `y * s` to
/// `(y + 1) * s` down. Draw a trail over it with [`pencil_svg`].
#[must_use]
pub fn ink_svg_cells(
    grid: &Grid,
    s: f64,
    draw_with_curves: bool,
    solution: Option<Solution>,
    seed: u64,
    style: &Style,
) -> String {
    ink_svg_sized(grid, grid.width() as f64 * s, s, draw_with_curves, solution, seed, style)
}

/// An inked maze `w` wide, with cells of side `s`.
fn ink_svg_sized(
    grid: &Grid,
    w: f64,
    s: f64,
    draw_with_curves: bool,
    solution: Option<Solution>,
    seed: u64,
    style: &Style,
) -> String {
    let h = grid.height() as f64 * s;
    let ink = Ink::new(grid, s, draw_with_curves, solution, seed, style);
    let Pens { line: base, wall, solution: pencil } = ink.pens;
    let (ink_hex, pencil) = (wall.hex(), pencil.hex());

    let mut out = String::new();
    svg_open(&mut out, w, h, s, style);

    w!(out, r#"<g fill="none" stroke="{ink_hex}" stroke-linecap="round" stroke-linejoin="round">"#);
    for (factor, d) in WIDTHS.iter().zip(walls(grid, s, &ink.pen, &ink.tiles, Format::Svg)) {
        if !d.is_empty() {
            out.push_str(r#"<path stroke-width=""#);
            push_fixed_trimmed(&mut out, base * factor, 2);
            w!(out, r#"" d="{d}"/>"#);
        }
    }
    out.push_str("</g>");

    // the start and end, inked in
    w!(out, r#"<path fill="{pencil}" fill-opacity="0.85" stroke="{ink_hex}" stroke-width=""#);
    push_fixed_trimmed(&mut out, base * 0.75, 2);
    out.push_str(r#"" d=""#);
    for points in ink.dots(grid, s) {
        push_polyline(&mut out, &points, Format::Svg);
        out.push('Z');
    }
    out.push_str(r#""/>"#);

    // the solution, gone over twice in pencil, and once lightly where it's
    // under a crossing
    if !ink.over.is_empty() {
        push_pencil(&mut out, &ink.pen, &ink.over, &ink.under, base, &pencil);
    }
    svg_frame(&mut out, grid, s, base, wall, style);
    out.push_str("</svg>");
    out
}

/// `trail` (cells as in a [`Solution`], from where it starts to where it ends)
/// drawn in pencil of `colour`, as an SVG group to lay over [`ink_svg_cells`]
/// with the same `grid`, `s`, `draw_with_curves` and `seed`: it is drawn just as
/// the solution is, gone over twice, and lightly where it passes under a
/// crossing. Empty for a trail of fewer than two cells.
#[must_use]
pub fn pencil_svg(grid: &Grid, s: f64, draw_with_curves: bool, trail: Solution, seed: u64, colour: Colour) -> String {
    if trail.len() < 2 {
        return String::new();
    }
    let pen = Pen::new(s, seed);
    let traces = Traces::new(grid, s, draw_with_curves, Some(trail));
    let over = flatten(&parse_path(&traces.over), pen.step);
    let under = flatten(&parse_path(&traces.under), pen.step);
    // the middle pen is 7.5% of a cell wide, as in the maze
    let mut out = String::new();
    push_pencil(&mut out, &pen, &over, &under, s * 0.075, &colour.hex());
    out
}

/// `colour` as if drawn at `opacity` on white, since the PDF has no
/// transparency.
fn faded(colour: Colour, opacity: f64) -> Colour {
    Colour(colour.0.map(|c| (f64::from(c) * opacity + 255.0 * (1.0 - opacity)).round() as u8))
}

/// The uncompressed page content stream of a hand-drawn PDF.
#[must_use]
pub fn ink_pdf_content(
    grid: &Grid,
    options: &PdfOptions,
    solution: Option<Solution>,
    seed: u64,
    style: &Style,
) -> String {
    let layout = pdf_layout(grid, options, style);
    let s = layout.s;
    let ink = Ink::new(grid, s, options.draw_with_curves, solution, seed, style);
    let Pens { line: base, wall, solution: pencil } = ink.pens;

    // no background unless asked for, so the page shows through
    let mut out = String::new();
    pdf_background(&mut out, &layout, style);
    out.push_str("q\n1 J 1 j\n");
    // the maze is drawn as in the SVG, top down, so flip it onto the page
    pdf_flip(&mut out, grid, &layout);
    wall.push_pdf(&mut out, "RG");

    for (factor, d) in WIDTHS.iter().zip(walls(grid, s, &ink.pen, &ink.tiles, Format::Pdf)) {
        push_fixed_trimmed(&mut out, base * factor, 3);
        out.push_str(" w\n");
        out.push_str(&d);
        out.push_str("S\n");
    }

    pencil.push_pdf(&mut out, "rg");
    push_fixed_trimmed(&mut out, base * 0.75, 3);
    out.push_str(" w\n");
    for points in ink.dots(grid, s) {
        push_polyline(&mut out, &points, Format::Pdf);
        out.push_str("b\n");
    }

    // the solution in pencil, faded rather than see-through
    if !ink.over.is_empty() {
        let passes = [
            (&ink.over, SALT_PENCIL[0], 2.2, 0.55),
            (&ink.over, SALT_PENCIL[1], 1.5, 0.75),
            (&ink.under, SALT_PENCIL[2], 1.5, 0.3),
        ];
        for (lines, salt, factor, opacity) in passes {
            faded(pencil, opacity).push_pdf(&mut out, "RG");
            push_fixed_trimmed(&mut out, base * factor, 3);
            out.push_str(" w\n");
            ink.push_pass(&mut out, lines, salt, Format::Pdf);
            out.push_str("S\n");
        }
    }
    out.push_str("Q\n");
    pdf_frame(&mut out, grid, &layout, base, wall, style);
    out
}

/// Render the maze (and optionally its solution) as a complete, hand-drawn
/// single-page PDF document.
#[must_use]
pub fn ink_pdf(grid: &Grid, options: &PdfOptions, solution: Option<Solution>, seed: u64, style: &Style) -> Vec<u8> {
    let content = ink_pdf_content(grid, options, solution, seed, style);
    pdf_document(&content, &pdf_layout(grid, options, style), &[])
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

    fn pencil_maze() -> (Grid, Vec<(isize, isize)>) {
        let mut rng = crate::random::Rng::from_str_seed("pencil");
        let grid = crate::maze::create_maze(12, 9, 50, false, &mut rng);
        let solution = crate::maze::find_solution(&grid).expect("a maze has a solution");
        (grid, solution)
    }

    #[test]
    fn a_trail_in_pencil_is_drawn_as_the_solution_is() {
        let (grid, solution) = pencil_maze();
        let solved = ink_svg_cells(&grid, 40.0, true, Some(&solution), 7, &Style::default());
        let pencil = pencil_svg(&grid, 40.0, true, &solution, 7, PENCIL);
        assert!(!pencil.is_empty());
        assert!(solved.contains(&pencil), "the solution is the same pencil marks");
        assert!(solved.starts_with(r#"<svg id="maze" xmlns="http://www.w3.org/2000/svg" viewBox="0 0 480.0 360.0""#));
    }

    #[test]
    fn any_stretch_of_a_trail_can_be_pencilled() {
        let (grid, solution) = pencil_maze();
        assert!(pencil_svg(&grid, 40.0, true, &solution[..1], 7, PENCIL).is_empty());
        for start in 0..solution.len() - 1 {
            for end in start + 2..=solution.len() {
                let mut trail = solution[start..end].to_vec();
                for _ in 0..2 {
                    let pencil = pencil_svg(&grid, 40.0, true, &trail, 7, PENCIL);
                    assert!(pencil.contains(" d=\"M"), "{trail:?} draws something");
                    assert!(!pencil.contains("NaN"));
                    // and the same stretch walked the other way
                    trail.reverse();
                }
            }
        }
    }

    #[test]
    fn same_seed_same_drawing() {
        let mut rng = crate::random::Rng::from_str_seed("ink");
        let grid = crate::maze::create_maze(10, 12, 50, false, &mut rng);
        let a = ink_svg(&grid, PageSize::A4, true, None, seed_from_id("ink"), &Style::default());
        assert_eq!(a, ink_svg(&grid, PageSize::A4, true, None, seed_from_id("ink"), &Style::default()));
        assert_ne!(a, ink_svg(&grid, PageSize::A4, true, None, seed_from_id("other"), &Style::default()));
    }
}
