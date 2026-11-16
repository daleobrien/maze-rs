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

use crate::maze::{Grid, CROSS_EW, CROSS_NS};
use crate::renderers::frame::{pdf_background, pdf_document, pdf_flip, pdf_frame, pdf_layout, svg_frame, svg_open};
use crate::renderers::path::{flatten, parse_path, Chain, Point, Segment};
use crate::renderers::pdf::PdfOptions;
use crate::renderers::style::{Colour, Pens, Style, CORRIDOR, RING};
use crate::renderers::svg::{dots, is_dot, plain_tiles, svg_scale, Traces};
use crate::renderers::{map_bands, push_fixed_trimmed, push_pair, used_tiles, PageSize, Solution, TILE_KINDS};

/// The ink and the pencil used for the solution.
const INK: Colour = Colour([0x2b, 0x21, 0x18]);
const PENCIL: Colour = Colour([0xc0, 0x39, 0x2b]);

/// How many pieces of a wall are drawn with one width before the pen changes.
const CHUNK: usize = 6;

/// The pen widths, as fractions of the middle one.
const WIDTHS: [f64; 3] = [0.8, 1.0, 1.22];

/// How strongly the passage that goes under a crossing shows through the one
/// on top, as in maze-hex.
const FAINT_OPACITY: &str = "0.2";

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

/// Append a wobbled wall to the path data for each pen width, a few pieces at a
/// time, each with the pen's width where it is.
fn push_wall(out: &mut [String; 3], pen: &Pen, wobbled: &[Point], format: Format) {
    // consecutive chunks share a point, so the pen never lifts
    let mut start = 0;
    while start + 1 < wobbled.len() {
        let end = (start + CHUNK).min(wobbled.len() - 1);
        let width = pen.width_at(wobbled[(start + end) / 2]);
        push_polyline(&mut out[width], &wobbled[start..=end], format);
        start = end;
    }
}

/// Points along the arc of radius `r` round `centre` from angle `from` to angle
/// `to` (radians, either way round), at most `step` apart, ends included.
fn arc(centre: Point, r: f64, from: f64, to: f64, step: f64) -> Vec<Point> {
    let n = ((to - from).abs() * r / step).ceil().max(1.0) as usize;
    (0..=n)
        .map(|k| {
            let angle = from + (to - from) * k as f64 / n as f64;
            (centre.0 + r * angle.cos(), centre.1 + r * angle.sin())
        })
        .collect()
}

/// `angle` moved by whole turns to within half a turn of `near`.
fn near_angle(angle: f64, near: f64) -> f64 {
    angle - (2.0 * PI) * ((angle - near) / (2.0 * PI)).round()
}

/// The walls of the start or end cell `(i, j)`, holding `cell`, drawn as Lubra
/// draws them: each corridor into the cell blends into a ring of radius `ring`
/// round its middle, so the outline has no corner. Where two corridors meet at a
/// right angle the ring gives way to a rounded inside corner. `hw` is half a
/// corridor's width and `s` the side of a cell. As polylines on the page.
fn ring_walls(cell: u8, (i, j): (usize, usize), s: f64, hw: f64, ring: f64, step: f64) -> Vec<Vec<Point>> {
    use crate::maze::{E, N, S, W};
    let centre = (i as f64 * s + s / 2.0, j as f64 * s + s / 2.0);
    let at = |base: Point, u: Point, a: f64, v: Point, b: f64| (base.0 + u.0 * a + v.0 * b, base.1 + u.1 * a + v.1 * b);
    // the ways out, as unit directions (y down), in order round the cell
    let mut ways: Vec<(f64, f64)> = [(N, (0.0, -1.0)), (S, (0.0, 1.0)), (E, (1.0, 0.0)), (W, (-1.0, 0.0))]
        .into_iter()
        .filter(|&(flag, _)| cell & flag != 0)
        .map(|(_, u)| u)
        .collect();
    if ways.is_empty() {
        return vec![arc(centre, ring, 0.0, 2.0 * PI, step)];
    }
    ways.sort_by(|a, b| a.1.atan2(a.0).total_cmp(&b.1.atan2(b.0)));

    // A wall blends into the ring on a small circle outside both, as in Lubra.
    let blend = s * 0.1;
    let reach = ring + blend;
    let rise = hw + blend;
    let run = (reach * reach - rise * rise).max(0.0).sqrt();
    let alpha = rise.atan2(run);

    (0..ways.len())
        .map(|k| {
            let (u, next) = (ways[k], ways[(k + 1) % ways.len()]);
            // across each corridor, a quarter turn on from it
            let v = (-u.1, u.0);
            let w = (-next.1, next.0);
            let theta = u.1.atan2(u.0);
            let mut gap = next.1.atan2(next.0) - theta;
            if gap <= 1e-9 {
                gap += 2.0 * PI;
            }
            // from the cell's side, along this corridor's wall on the `v` side ...
            let mut points = vec![at(centre, u, s / 2.0, v, hw)];
            if gap - 2.0 * alpha > 0.05 {
                // ... round the blend, the ring and the next corridor's blend
                let b1 = at(centre, u, run, v, rise);
                let start = (-v.1).atan2(-v.0);
                let end = near_angle(theta + alpha + PI, start);
                points.extend(arc(b1, blend, start, end, step));
                points.extend(arc(centre, ring, theta + alpha, theta + gap - alpha, step).into_iter().skip(1));
                let b2 = at(centre, next, run, w, -rise);
                let phi = next.1.atan2(next.0);
                let start = phi - alpha + PI;
                let end = near_angle(w.1.atan2(w.0), start);
                points.extend(arc(b2, blend, start, end, step).into_iter().skip(1));
            } else {
                // ... or, where the corridors meet at a right angle, a rounded inside corner
                let corner = at(centre, u, hw + blend, v, hw + blend);
                let start = (-v.1).atan2(-v.0);
                let end = near_angle((-u.1).atan2(-u.0), start);
                points.extend(arc(corner, blend, start, end, step));
            }
            // ... and out along the next corridor's wall on its far side
            points.push(at(centre, next, s / 2.0, w, -hw));
            points
        })
        .collect()
}

/// The walls, wobbled, as path data for each pen width.
fn walls(
    grid: &Grid,
    s: f64,
    pen: &Pen,
    tiles: &[Vec<Vec<Point>>],
    rings: &[Vec<Point>],
    format: Format,
) -> [String; 3] {
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
                    push_wall(&mut out, pen, &wobbled, format);
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
    // the start and end, when drawn as rings
    for line in rings {
        let wobbled: Vec<Point> = line.iter().map(|&p| pen.wobble(p, 0)).collect();
        push_wall(&mut out, pen, &wobbled, format);
    }
    out
}

/// The walls of the passage that goes under a crossing of kind `cell`, in the
/// tile's own coordinates: the stretch of each wall hidden inside the passage
/// on top, from one of its walls to the other. `custom` is whether the style
/// changes the corridor, which sets how wide the passages are.
fn hidden_walls(cell: u8, s: f64, corridor: Option<f64>) -> Vec<Chain> {
    // the passage's walls are at `a` and `b` across the cell, written exactly
    // as the tiles' own walls are so the two ends meet
    let (a, b) = match corridor {
        Some(corridor) => (s / 2.0 - corridor * s / 2.0, s / 2.0 + corridor * s / 2.0),
        None => (s * 0.2, s - s * 0.2),
    };
    // where the north-south passage is on top the east-west one runs under it
    // along `y = a` and `y = b`; where it is the other way round, the walls
    // are turned a quarter
    let across = |at: f64| -> ((f64, f64), (f64, f64)) {
        if cell == CROSS_NS {
            ((a, at), (b, at))
        } else {
            ((at, a), (at, b))
        }
    };
    [a, b]
        .into_iter()
        .map(|at| {
            let (start, end) = across(at);
            Chain { start, segments: vec![Segment::Line(end)] }.svg_rounded()
        })
        .collect()
}

/// The hidden walls of each kind of crossing the maze has, flattened, in the
/// tile's own coordinates; empty for every other kind of tile.
fn faint_tiles(s: f64, step: f64, corridor: Option<f64>, used: &[bool; TILE_KINDS]) -> Vec<Vec<Vec<Point>>> {
    (0..TILE_KINDS as u8)
        .map(|cell| {
            if (cell == CROSS_NS || cell == CROSS_EW) && used[usize::from(cell)] {
                flatten(&hidden_walls(cell, s, corridor), step)
            } else {
                Vec::new()
            }
        })
        .collect()
}

/// The walls hidden under the crossings, wobbled with the same pen as the
/// visible walls so they join up with them, as path data for each pen width.
/// Empty when the maze has no crossings.
fn faint_walls(grid: &Grid, s: f64, pen: &Pen, faint: &[Vec<Vec<Point>>]) -> [String; 3] {
    let mut out: [String; 3] = Default::default();
    let mut wobbled = Vec::new();
    for j in 0..grid.height() {
        for (i, &cell) in grid.row(j).iter().enumerate() {
            let (x0, y0) = (i as f64 * s, j as f64 * s);
            for line in &faint[usize::from(cell)] {
                wobbled.clear();
                wobbled.extend(line.iter().map(|&(x, y)| pen.wobble((x0 + x, y0 + y), 0)));
                push_wall(&mut out, pen, &wobbled, Format::Svg);
            }
        }
    }
    out
}

/// Append a path for each pen width that has any walls, `base` being the
/// width of the middle pen.
fn push_wall_paths(out: &mut String, base: f64, paths: [String; 3]) {
    for (factor, d) in WIDTHS.iter().zip(paths) {
        if !d.is_empty() {
            out.push_str(r#"<path stroke-width=""#);
            push_fixed_trimmed(out, base * factor, 2);
            w!(out, r#"" d="{d}"/>"#);
        }
    }
}

/// Everything an inked maze is drawn from, for cells of side `s`.
struct Ink {
    pen: Pen,
    /// Each kind of tile's walls, flattened, in the tile's own coordinates.
    tiles: Vec<Vec<Vec<Point>>>,
    /// The walls hidden under each kind of crossing, flattened, in the tile's
    /// own coordinates.
    faint: Vec<Vec<Vec<Point>>>,
    /// The solution, flattened, where it is on top and where it is under a
    /// crossing.
    over: Vec<Vec<Point>>,
    under: Vec<Vec<Point>>,
    /// The width of the middle pen, and the colours.
    pens: Pens,
    /// The radius of the start and end rings, when drawn as Lubra does.
    ring: Option<f64>,
    /// Their walls, then: polylines on the page, not yet wobbled.
    rings: Vec<Vec<Point>>,
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
        let ring = Some(style.ring.unwrap_or(RING) * s).filter(|&ring| ring > 0.0);
        let rings = ring.map_or_else(Vec::new, |ring| {
            let hw = style.corridor.unwrap_or(CORRIDOR) * s / 2.0;
            let mut cells = vec![(0, 0)];
            let last = (grid.width() - 1, grid.height() - 1);
            if last != (0, 0) {
                cells.push(last);
            }
            cells.into_iter().flat_map(|(i, j)| ring_walls(grid[(i, j)], (i, j), s, hw, ring, pen.step)).collect()
        });
        Ink {
            faint: faint_tiles(s, pen.step, style.custom_shape().then_some(style.corridor.unwrap_or(CORRIDOR)), &used),
            ring,
            rings,
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
                // in a ring the dot is half its size, as in Lubra
                let r = self.ring.map_or(r * 0.8, |ring| ring / 2.0);
                circle(cx, cy, r, self.pen.step).into_iter().map(|p| self.pen.wobble(p, 0)).collect()
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
    push_wall_paths(&mut out, base, walls(grid, s, &ink.pen, &ink.tiles, &ink.rings, Format::Svg));
    out.push_str("</g>");

    // where a passage goes under another, its walls show faintly through the
    // one on top. They are one group, so the opacity is applied once and where
    // pieces of a wall overlap they don't darken.
    let faint = faint_walls(grid, s, &ink.pen, &ink.faint);
    if faint.iter().any(|d| !d.is_empty()) {
        w!(
            out,
            r#"<g opacity="{FAINT_OPACITY}" fill="none" stroke="{ink_hex}" stroke-linecap="round" stroke-linejoin="round">"#
        );
        push_wall_paths(&mut out, base, faint);
        out.push_str("</g>");
    }

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

    for (factor, d) in WIDTHS.iter().zip(walls(grid, s, &ink.pen, &ink.tiles, &ink.rings, Format::Pdf)) {
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

    /// A 3 by 3 grid with straight passages through the middle, which is a
    /// crossing of kind `middle`, or just a passage when it isn't one.
    fn crossing_grid(middle: u8) -> Grid {
        use crate::maze::{E, N, S, W};
        let mut grid = Grid::new(3, 3);
        grid[(1, 0)] = S;
        grid[(1, 2)] = N;
        grid[(0, 1)] = E;
        grid[(2, 1)] = W;
        grid[(1, 1)] = middle;
        grid
    }

    #[test]
    fn the_passage_under_a_crossing_shows_faintly() {
        for middle in [CROSS_NS, CROSS_EW] {
            let svg = ink_svg_cells(&crossing_grid(middle), 40.0, true, None, 7, &Style::default());
            assert_eq!(svg.matches(r#"<g opacity="0.2""#).count(), 1, "one faint group for crossing {middle}");
            assert!(!svg.contains("NaN"));
        }
    }

    #[test]
    fn nothing_is_faint_without_a_crossing() {
        use crate::maze::{E, W};
        let svg = ink_svg_cells(&crossing_grid(E | W), 40.0, true, None, 7, &Style::default());
        assert!(!svg.contains("opacity=\"0.2\""));
    }

    #[test]
    fn the_hidden_walls_run_between_the_walls_on_top() {
        // north-south on top: the east-west walls cross from x = 8 to x = 32
        let chains = hidden_walls(CROSS_NS, 40.0, None);
        let lines = flatten(&chains, 4.0);
        assert_eq!(lines.len(), 2);
        assert_eq!((lines[0][0], *lines[0].last().unwrap()), ((8.0, 8.0), (32.0, 8.0)));
        assert_eq!((lines[1][0], *lines[1].last().unwrap()), ((8.0, 32.0), (32.0, 32.0)));
        // east-west on top: the same, turned a quarter
        let lines = flatten(&hidden_walls(CROSS_EW, 40.0, None), 4.0);
        assert_eq!((lines[0][0], *lines[0].last().unwrap()), ((8.0, 8.0), (8.0, 32.0)));
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
