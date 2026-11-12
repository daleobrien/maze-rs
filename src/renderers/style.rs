//! Optional drawing styles for the SVG and PDF output: the shape of corners
//! and dead ends, the width of the corridors, a gap where a passage goes
//! under a bridge, the line weight, colours, and a printed frame with START
//! and FINISH arrows and a footer.
//!
//! None of this touches the plain renderers: when no style option is given
//! the original `svg_render` and `pdf_render` run and their output is
//! unchanged. When one is given, the walls are rebuilt here from scratch for
//! each kind of tile, so every option works with every other.
//!
//! The page round the maze (background, frame and footer) is drawn by
//! `frame`, and the walls are written out by `path`.
//!
//! A tile's walls are found by walking clockwise around the corridor's
//! inner box: a closed side adds an edge of the box, an open side ends one
//! wall and starts another at the cell's edge. Each wall is then a polyline
//! of right-angled corners, which are left square, cut off (bevel) or
//! rounded, as far as the neighbouring corners allow. Rounding as far as
//! possible gives the same curves as the plain renderer.

use std::fmt::Write as _;

use crate::maze::{is_crossing, Grid, CROSS_EW, E, N, S, W};
use crate::renderers::frame::{pdf_background, pdf_document, pdf_flip, pdf_frame, pdf_layout, svg_frame, svg_open};
use crate::renderers::path::{
    distance, parse_path, polyline_chain, push_pdf_circle, push_pdf_xy, towards, Chain, Point, Segment,
};
use crate::renderers::pdf::PdfOptions;
use crate::renderers::svg::{dots, is_dot, push_uses, svg_scale, Traces};
use crate::renderers::{map_bands, push_fixed_trimmed, used_tiles, PageSize, Piece, Solution, TILE_KINDS};

/// How the corners of the corridors are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corners {
    Round,
    Bevel,
    Square,
}

/// How a corridor that goes nowhere ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeadEnds {
    /// A round bulb wider than the corridor, like the plain renderer's.
    Bulb,
    /// A semicircle as wide as the corridor.
    Round,
    /// Squared off.
    Flat,
}

/// A colour, from `#rrggbb` or `#rgb` (the `#` is optional).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colour(pub [u8; 3]);

impl Colour {
    #[must_use]
    pub fn parse(text: &str) -> Option<Colour> {
        let hex = text.strip_prefix('#').unwrap_or(text);
        let digit = |i: usize| u8::from_str_radix(hex.get(i..=i)?, 16).ok();
        match hex.len() {
            3 => Some(Colour([digit(0)? * 17, digit(1)? * 17, digit(2)? * 17])),
            6 => Some(Colour([digit(0)? * 16 + digit(1)?, digit(2)? * 16 + digit(3)?, digit(4)? * 16 + digit(5)?])),
            _ => None,
        }
    }

    #[must_use]
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0[0], self.0[1], self.0[2])
    }

    /// Append the colour as PDF operands and `op` (`rg` to fill, `RG` to stroke).
    pub(crate) fn push_pdf(self, out: &mut String, op: &str) {
        for c in self.0 {
            push_fixed_trimmed(out, f64::from(c) / 255.0, 3);
            out.push(' ');
        }
        out.push_str(op);
        out.push('\n');
    }
}

/// The style options; `None` means "as the renderer normally draws it".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Style {
    pub corners: Option<Corners>,
    pub dead_ends: Option<DeadEnds>,
    /// The width of a corridor as a fraction of a cell (normally 0.6).
    pub corridor: Option<f64>,
    /// How far, as a fraction of a cell, the walls of a passage stop short
    /// of the bridge it goes under (normally 0).
    pub bridge_gap: Option<f64>,
    /// The width of the walls, in pixels (SVG) or points (PDF).
    pub line: Option<f64>,
    pub wall_colour: Option<Colour>,
    pub solution_colour: Option<Colour>,
    pub background: Option<Colour>,
    /// Draw a frame, START and FINISH arrows and a footer naming this maze id.
    pub frame: Option<String>,
}

/// A corridor's width, as a fraction of a cell, when the style doesn't set one.
const CORRIDOR: f64 = 0.6;

impl Style {
    /// Whether no option is set, so the plain renderers can be used.
    #[must_use]
    pub fn is_plain(&self) -> bool {
        *self == Style::default()
    }

    /// Whether any option changes the shape of the walls.
    #[must_use]
    pub fn custom_shape(&self) -> bool {
        self.corners.is_some() || self.dead_ends.is_some() || self.corridor.is_some() || self.bridge_gap.is_some()
    }

    /// Whether the solution, and walls of the plain shape, are drawn with
    /// curves: as asked (`-S` or not), unless the corners say otherwise.
    pub(crate) fn curves(&self, draw_with_curves: bool) -> bool {
        self.corners.map_or(draw_with_curves, |c| c == Corners::Round)
    }

    /// The line width and colours, with a renderer's own defaults for those
    /// the style doesn't set.
    pub(crate) fn pens(&self, default: Pens) -> Pens {
        Pens {
            line: self.line.unwrap_or(default.line),
            wall: self.wall_colour.unwrap_or(default.wall),
            solution: self.solution_colour.unwrap_or(default.solution),
        }
    }

    /// The tile shape, with defaults filled in; plain `-S` means square corners.
    fn shape(&self, draw_with_curves: bool) -> Shape {
        Shape {
            corners: self.corners.unwrap_or(if draw_with_curves { Corners::Round } else { Corners::Square }),
            dead_ends: self.dead_ends.unwrap_or(DeadEnds::Bulb),
            corridor: self.corridor.unwrap_or(CORRIDOR),
            bridge_gap: self.bridge_gap.unwrap_or(0.0),
        }
    }

    /// The walls of each kind of tile marked in `used`, in the tile's own
    /// coordinates, for cells of side `s` (none for the other kinds). Their
    /// numbers are as written in SVG, so the SVG, PDF and ink drawings of a
    /// tile agree.
    pub(crate) fn tiles(&self, s: f64, draw_with_curves: bool, used: &[bool; TILE_KINDS]) -> Vec<Vec<Chain>> {
        let shape = self.shape(draw_with_curves);
        (0..TILE_KINDS as u8)
            .map(|cell| {
                if used[usize::from(cell)] && (cell < 16 || is_crossing(cell)) {
                    shape.chains(cell, s).into_iter().map(Chain::svg_rounded).collect()
                } else {
                    Vec::new()
                }
            })
            .collect()
    }
}

/// The line width and colours a maze is drawn with.
#[derive(Clone, Copy)]
pub(crate) struct Pens {
    /// The width of the walls.
    pub line: f64,
    pub wall: Colour,
    pub solution: Colour,
}

#[derive(Clone, Copy)]
struct Shape {
    corners: Corners,
    dead_ends: DeadEnds,
    corridor: f64,
    bridge_gap: f64,
}

impl Shape {
    /// The walls of a tile of kind `cell`, with cells of side `s`.
    fn chains(self, cell: u8, s: f64) -> Vec<Chain> {
        let hw = self.corridor * s / 2.0;
        let (a, b) = (s / 2.0 - hw, s / 2.0 + hw);
        if is_crossing(cell) {
            let g = (self.bridge_gap * s).min(a * 0.9);
            let mut lines = vec![((a, 0.0), (a, s)), ((b, 0.0), (b, s))];
            for y in [a, b] {
                if a - g > 1e-6 {
                    lines.push(((0.0, y), (a - g, y)));
                    lines.push(((b + g, y), (s, y)));
                }
            }
            let transpose = cell == CROSS_EW;
            return lines
                .into_iter()
                .map(|(p, q)| {
                    let (p, q) = if transpose { ((p.1, p.0), (q.1, q.0)) } else { (p, q) };
                    Chain { start: p, segments: vec![Segment::Line(q)] }
                })
                .collect();
        }

        let open = [N, E, S, W].map(|side| cell & side != 0);
        let open_count = open.iter().filter(|&&o| o).count();
        if open_count == 1 {
            let side = open.iter().position(|&o| o).expect("one side is open");
            match self.dead_ends {
                DeadEnds::Bulb => {
                    if let Some(chain) = bulb(s, hw) {
                        return vec![rotate(chain, side, s)];
                    }
                    return walk(open, s, a, b, Corners::Round);
                }
                DeadEnds::Round => return walk(open, s, a, b, Corners::Round),
                DeadEnds::Flat => return walk(open, s, a, b, Corners::Square),
            }
        }
        walk(open, s, a, b, self.corners)
    }
}

/// The walls of a tile open on the sides marked in `open` (north, east,
/// south, west), with the corridor's inner box from `a` to `b` across a
/// cell of side `s`, found by walking clockwise around the box.
fn walk(open: [bool; 4], s: f64, a: f64, b: f64, corners: Corners) -> Vec<Chain> {
    // the box's corners, and where each side's walls meet the cell edge
    // when it is open, both clockwise from the top left
    let corner = [(a, a), (b, a), (b, b), (a, b)];
    let edge = [[(a, 0.0), (b, 0.0)], [(s, a), (s, b)], [(b, s), (a, s)], [(0.0, b), (0.0, a)]];

    let Some(first) = open.iter().position(|&o| o) else {
        let mut points = corner.to_vec();
        points.push(corner[0]);
        return vec![polyline_chain(&points)];
    };
    let mut lines: Vec<Vec<Point>> = Vec::new();
    let mut current = vec![edge[first][1], corner[(first + 1) % 4]];
    for j in 1..4 {
        let k = (first + j) % 4;
        if open[k] {
            current.push(edge[k][0]);
            lines.push(std::mem::take(&mut current));
            current = vec![edge[k][1], corner[(k + 1) % 4]];
        } else {
            current.push(corner[(k + 1) % 4]);
        }
    }
    current.push(edge[first][0]);
    lines.push(current);

    lines.iter().map(|points| shape_corners(&drop_straight(points), corners)).collect()
}

/// The points of a polyline without those in the middle of a straight run.
fn drop_straight(points: &[Point]) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(points.len());
    for &p in points {
        if let Some(&last) = out.last() {
            if distance(last, p) < 1e-9 {
                continue;
            }
        }
        if out.len() >= 2 {
            let (o, m) = (out[out.len() - 2], out[out.len() - 1]);
            let cross = (m.0 - o.0) * (p.1 - m.1) - (m.1 - o.1) * (p.0 - m.0);
            if cross.abs() < 1e-9 {
                out.pop();
            }
        }
        out.push(p);
    }
    out
}

/// Square, cut off or round each corner of a polyline, by as much as its
/// neighbours allow (half the way to a neighbouring corner, or all the way
/// to the end of the wall).
fn shape_corners(points: &[Point], corners: Corners) -> Chain {
    if corners == Corners::Square || points.len() < 3 {
        return polyline_chain(points);
    }
    let last = points.len() - 1;
    let mut chain = Chain { start: points[0], segments: Vec::new() };
    let mut at = points[0];
    // a line to where the pen already is would draw nothing
    let line_to = |chain: &mut Chain, at: &mut Point, p: Point| {
        if distance(*at, p) > 1e-6 {
            chain.segments.push(Segment::Line(p));
        }
        *at = p;
    };
    for i in 1..last {
        let (prev, v, next) = (points[i - 1], points[i], points[i + 1]);
        let room = |q: Point, is_corner: bool| distance(v, q) / if is_corner { 2.0 } else { 1.0 };
        let r = room(prev, i - 1 > 0).min(room(next, i + 1 < last));
        let (p1, p2) = (towards(v, prev, r), towards(v, next, r));
        line_to(&mut chain, &mut at, p1);
        if corners == Corners::Bevel {
            line_to(&mut chain, &mut at, p2);
        } else {
            let cross = (v.0 - prev.0) * (next.1 - v.1) - (v.1 - prev.1) * (next.0 - v.0);
            chain.segments.push(Segment::Arc { radius: r, large: false, sweep: cross > 0.0, to: p2 });
            at = p2;
        }
    }
    line_to(&mut chain, &mut at, points[last]);
    chain
}

/// A dead end open to the north that swells into a round bulb, or `None`
/// when the corridor is too wide for one to fit.
fn bulb(s: f64, hw: f64) -> Option<Chain> {
    let radius = (hw * 1.45).min(0.46 * s);
    if radius <= hw * 1.05 {
        return None;
    }
    let neck = (radius * radius - hw * hw).sqrt();
    let cy = (0.96 * s - radius).max(neck + 0.02 * s);
    let yi = cy - neck;
    let (a, b) = (s / 2.0 - hw, s / 2.0 + hw);
    Some(Chain {
        start: (b, 0.0),
        segments: vec![
            Segment::Line((b, yi)),
            Segment::Arc { radius, large: true, sweep: true, to: (a, yi) },
            Segment::Line((a, 0.0)),
        ],
    })
}

/// Turn a chain drawn for the north side to face side `side` (0 north, 1
/// east, 2 south, 3 west), turning clockwise about the cell's centre.
fn rotate(chain: Chain, side: usize, s: f64) -> Chain {
    let c = s / 2.0;
    chain.map_points(|(x, y)| {
        let mut p = (x - c, y - c);
        for _ in 0..side {
            p = (-p.1, p.0);
        }
        (p.0 + c, p.1 + c)
    })
}
// ---- the styled renderers ----

const WALL: Colour = Colour([0, 0, 0]);
const SOLUTION: Colour = Colour([0xE5, 0x19, 0x19]);

/// How wide the solution is drawn, for walls `line` wide and cells of side `s`.
fn solution_width(style: &Style, line: f64, s: f64) -> f64 {
    (line * 2.5).min(style.corridor.unwrap_or(CORRIDOR) * s * 0.5)
}

/// Render the maze (and optionally its solution) as an SVG in a style.
#[must_use]
pub fn styled_svg(
    grid: &Grid,
    page: PageSize,
    draw_with_curves: bool,
    solution: Option<Solution>,
    style: &Style,
) -> String {
    let (width, s) = svg_scale(grid, page);
    let (w, h) = (width * s, grid.height() as f64 * s);
    let Pens { line, wall, solution: path_colour } = style.pens(Pens { line: 2.0, wall: WALL, solution: SOLUTION });
    let path_colour = path_colour.hex();
    let traces = Traces::new(grid, s, style.curves(draw_with_curves), solution);

    let mut out = String::new();
    svg_open(&mut out, w, h, s, style);
    let used = used_tiles(grid, |i, j| is_dot(grid, i, j));
    out.push_str("<defs>");
    for (cell, chains) in style.tiles(s, draw_with_curves, &used).iter().enumerate().filter(|&(c, _)| used[c]) {
        w!(out, r#"<path id="t{cell}" d=""#);
        for chain in chains {
            chain.push_svg(&mut out);
        }
        out.push_str(r#""/>"#);
    }
    w!(out, r#"</defs><g fill="none" stroke="{}" stroke-width=""#, wall.hex());
    push_fixed_trimmed(&mut out, line, 2);
    out.push_str(r#"" stroke-linecap="round" stroke-linejoin="round">"#);
    push_uses(&mut out, grid, s);
    out.push_str("</g>");
    for (cx, cy, r) in dots(grid, s) {
        out.push_str(r#"<circle cx=""#);
        push_fixed_trimmed(&mut out, cx, 2);
        out.push_str(r#"" cy=""#);
        push_fixed_trimmed(&mut out, cy, 2);
        out.push_str(r#"" r=""#);
        push_fixed_trimmed(&mut out, r, 2);
        w!(out, r#"" fill="{path_colour}"/>"#);
    }
    if !traces.over.is_empty() {
        let width = solution_width(style, line, s);
        w!(
            out,
            r#"<g fill="none" stroke="{path_colour}" stroke-linecap="round" stroke-linejoin="round" stroke-width=""#
        );
        push_fixed_trimmed(&mut out, width, 2);
        w!(
            out,
            r#""><path d="{}"/><path stroke-opacity="0.3" d="{}"/></g>"#,
            traces.over.trim_end(),
            traces.under.trim_end()
        );
    }
    svg_frame(&mut out, grid, s, line, wall, style);
    out.push_str("</svg>");
    out
}

/// Append a `q ... cm /T<cell> Do Q` line drawing its tile for every cell
/// but the start and end, with cells of side `s`.
fn push_form_cells(out: &mut String, grid: &Grid, s: f64) {
    // the start of each column's line, the y of each row, and the end of the
    // line for each kind of tile
    let column_head: Vec<Piece> = (0..grid.width())
        .map(|i| {
            let mut head = String::from("q 1 0 0 1 ");
            push_fixed_trimmed(&mut head, i as f64 * s, 3);
            head.push(' ');
            Piece::new(&head)
        })
        .collect();
    let row_y: Vec<Piece> = (0..grid.height())
        .map(|j| {
            let mut y = String::new();
            push_fixed_trimmed(&mut y, j as f64 * s, 3);
            Piece::new(&y)
        })
        .collect();
    let tail: Vec<Piece> = (0..TILE_KINDS).map(|cell| Piece::new(&format!(" cm /T{cell} Do Q\n"))).collect();

    // rows are independent, so bands of them are written in parallel
    let bands = map_bands(grid.height(), grid.width(), |rows| {
        let mut band = String::with_capacity(rows.len() * grid.width() * 36);
        for j in rows {
            for (i, &cell) in grid.row(j).iter().enumerate() {
                if !is_dot(grid, i, j) {
                    column_head[i].push_to(&mut band);
                    row_y[j].push_to(&mut band);
                    tail[usize::from(cell)].push_to(&mut band);
                }
            }
        }
        band
    });
    out.reserve(bands.iter().map(String::len).sum());
    for band in bands {
        out.push_str(&band);
    }
}

/// Render the maze (and optionally its solution) as a PDF in a style. The
/// maze is drawn top down, as in the SVG, with the start at the top left.
#[must_use]
pub fn styled_pdf(grid: &Grid, options: &PdfOptions, solution: Option<Solution>, style: &Style) -> Vec<u8> {
    let layout = pdf_layout(grid, options, style);
    let s = layout.s;
    let Pens { line, wall, solution: path_colour } = style.pens(Pens { line: s / 7.0, wall: WALL, solution: SOLUTION });
    let traces = Traces::new(grid, s, style.curves(options.draw_with_curves), solution);

    let used = used_tiles(grid, |i, j| is_dot(grid, i, j));
    let forms: Vec<(u8, String)> = style
        .tiles(s, options.draw_with_curves, &used)
        .iter()
        .enumerate()
        .filter(|&(cell, _)| used[cell])
        .map(|(cell, chains)| {
            let mut stream = String::new();
            for chain in chains {
                chain.push_pdf(&mut stream);
            }
            stream.push('S');
            (cell as u8, stream)
        })
        .collect();

    let mut out = String::new();
    pdf_background(&mut out, &layout, style);
    out.push_str("q\n");
    pdf_flip(&mut out, grid, &layout);
    out.push_str("1 J 1 j\n");
    wall.push_pdf(&mut out, "RG");
    push_fixed_trimmed(&mut out, line, 3);
    out.push_str(" w\n");
    push_form_cells(&mut out, grid, s);
    path_colour.push_pdf(&mut out, "rg");
    path_colour.push_pdf(&mut out, "RG");
    for (cx, cy, r) in dots(grid, s) {
        push_pdf_circle(&mut out, (cx, cy), r);
    }
    if !traces.over.is_empty() {
        let width = solution_width(style, line, s);
        push_fixed_trimmed(&mut out, width, 3);
        out.push_str(" w\n");
        for chain in parse_path(&traces.over) {
            chain.push_pdf(&mut out);
        }
        out.push_str("S\n");
        // PDF has no simple transparency, so the parts under bridges are dashed
        out.push('[');
        push_pdf_xy(&mut out, (width * 0.2, width * 1.6));
        out.push_str("] 0 d\n");
        for chain in parse_path(&traces.under) {
            chain.push_pdf(&mut out);
        }
        out.push_str("S\n[] 0 d\n");
    }
    out.push_str("Q\n");
    pdf_frame(&mut out, grid, &layout, line, wall, style);
    pdf_document(&out, &layout, &forms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_parse() {
        assert_eq!(Colour::parse("#E51919"), Some(Colour([0xE5, 0x19, 0x19])));
        assert_eq!(Colour::parse("fff"), Some(Colour([255, 255, 255])));
        assert_eq!(Colour::parse("#12345"), None);
        assert_eq!(Colour::parse("zzzzzz"), None);
    }

    #[test]
    fn round_corners_match_the_plain_curves() {
        // a turn from north to east: inner wall a quarter circle of radius
        // a about the cell's top right corner, outer one of radius b
        let shape = Style::default().shape(true);
        let mut d = String::new();
        for chain in shape.chains(N | E, 100.0) {
            chain.push_svg(&mut d);
        }
        assert_eq!(d, "M80 0A20 20 0 0 0 100 20M100 80A80 80 0 0 1 20 0");
    }

    #[test]
    fn every_tile_has_walls() {
        for corners in [Corners::Round, Corners::Bevel, Corners::Square] {
            for dead_ends in [DeadEnds::Bulb, DeadEnds::Round, DeadEnds::Flat] {
                let style = Style { corners: Some(corners), dead_ends: Some(dead_ends), ..Style::default() };
                let tiles = style.tiles(30.0, true, &[true; TILE_KINDS]);
                for (cell, chains) in tiles.iter().enumerate().take(16).skip(1) {
                    assert!(!chains.is_empty(), "{corners:?} {dead_ends:?} {cell}");
                }
            }
        }
    }

    #[test]
    fn styled_pdf_is_well_formed() {
        let mut rng = crate::random::Rng::from_str_seed("style");
        let grid = crate::maze::create_maze(8, 10, 50, false, &mut rng);
        let solution = crate::maze::find_solution(&grid);
        let style = Style { frame: Some("style".to_owned()), corners: Some(Corners::Bevel), ..Style::default() };
        let options = PdfOptions { draw_with_curves: true, page: PageSize::A4, landscape: false };
        let pdf = styled_pdf(&grid, &options, solution.as_deref(), &style);
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(pdf.ends_with(b"%%EOF\n"));
    }
}
