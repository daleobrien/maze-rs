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
//! A tile's walls are found by walking clockwise around the corridor's
//! inner box: a closed side adds an edge of the box, an open side ends one
//! wall and starts another at the cell's edge. Each wall is then a polyline
//! of right-angled corners, which are left square, cut off (bevel) or
//! rounded, as far as the neighbouring corners allow. Rounding as far as
//! possible gives the same curves as the plain renderer.

use std::f64::consts::PI;
use std::fmt::Write as _;

use crate::maze::{Grid, CROSS_EW, CROSS_NS, E, N, S, W};
use crate::renderers::pdf::{pdf_date, zlib_compress, PageLayout, PdfOptions};
use crate::renderers::svg::{is_dot, path_data, svg_scale};
use crate::renderers::{push_fixed_trimmed, py_float_repr, PageSize};

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

    /// The tile shape, with defaults filled in; plain `-S` means square corners.
    fn shape(&self, draw_with_curves: bool) -> Shape {
        Shape {
            corners: self.corners.unwrap_or(if draw_with_curves { Corners::Round } else { Corners::Square }),
            dead_ends: self.dead_ends.unwrap_or(DeadEnds::Bulb),
            corridor: self.corridor.unwrap_or(0.6),
            bridge_gap: self.bridge_gap.unwrap_or(0.0),
        }
    }

    /// Each kind of tile's walls as SVG path data, in the tile's own
    /// coordinates, for cells of side `s` (empty for kinds that aren't
    /// tiles).
    pub(crate) fn tiles(&self, s: f64, draw_with_curves: bool) -> Vec<String> {
        let shape = self.shape(draw_with_curves);
        (0..32u8)
            .map(|cell| {
                let mut d = String::new();
                if cell < 16 || cell == CROSS_NS || cell == CROSS_EW {
                    for chain in shape.chains(cell, s) {
                        chain.push_svg(&mut d);
                    }
                }
                d
            })
            .collect()
    }
}

#[derive(Clone, Copy)]
struct Shape {
    corners: Corners,
    dead_ends: DeadEnds,
    corridor: f64,
    bridge_gap: f64,
}

type Point = (f64, f64);

enum Segment {
    Line(Point),
    Arc { radius: f64, large: bool, sweep: bool, to: Point },
}

/// One continuous wall.
struct Chain {
    start: Point,
    segments: Vec<Segment>,
}

fn push_xy(out: &mut String, (x, y): Point) {
    push_fixed_trimmed(out, x, 2);
    out.push(' ');
    push_fixed_trimmed(out, y, 2);
}

fn distance(p: Point, q: Point) -> f64 {
    (q.0 - p.0).hypot(q.1 - p.1)
}

/// The point `r` along the way from `from` towards `to`.
fn towards(from: Point, to: Point, r: f64) -> Point {
    let d = distance(from, to);
    (from.0 + (to.0 - from.0) * r / d, from.1 + (to.1 - from.1) * r / d)
}

impl Chain {
    fn push_svg(&self, out: &mut String) {
        out.push('M');
        push_xy(out, self.start);
        for segment in &self.segments {
            match *segment {
                Segment::Line(to) => {
                    out.push('L');
                    push_xy(out, to);
                }
                Segment::Arc { radius, large, sweep, to } => {
                    out.push('A');
                    push_fixed_trimmed(out, radius, 2);
                    out.push(' ');
                    push_fixed_trimmed(out, radius, 2);
                    write!(out, " 0 {} {} ", u8::from(large), u8::from(sweep))
                        .expect("writing to a String cannot fail");
                    push_xy(out, to);
                }
            }
        }
    }

    fn push_pdf(&self, out: &mut String) {
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

fn push_pdf_xy(out: &mut String, (x, y): Point) {
    push_fixed_trimmed(out, x, 3);
    out.push(' ');
    push_fixed_trimmed(out, y, 3);
}

/// An SVG circular arc as PDF Bezier curves (SVG 1.1 F.6.5, rx = ry).
fn push_pdf_arc(out: &mut String, from: Point, to: Point, radius: f64, large: bool, sweep: bool) {
    let (x1p, y1p) = ((from.0 - to.0) / 2.0, (from.1 - to.1) / 2.0);
    let d2 = x1p * x1p + y1p * y1p;
    if d2 < 1e-12 {
        return;
    }
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

impl Shape {
    /// The walls of a tile of kind `cell`, with cells of side `s`.
    fn chains(self, cell: u8, s: f64) -> Vec<Chain> {
        let hw = self.corridor * s / 2.0;
        let (a, b) = (s / 2.0 - hw, s / 2.0 + hw);
        if cell == CROSS_NS || cell == CROSS_EW {
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
                    return self.walk(open, s, a, b, Corners::Round);
                }
                DeadEnds::Round => return self.walk(open, s, a, b, Corners::Round),
                DeadEnds::Flat => return self.walk(open, s, a, b, Corners::Square),
            }
        }
        self.walk(open, s, a, b, self.corners)
    }

    /// The walls found by walking clockwise around the inner box.
    #[allow(clippy::unused_self)]
    fn walk(self, open: [bool; 4], s: f64, a: f64, b: f64, corners: Corners) -> Vec<Chain> {
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

fn polyline_chain(points: &[Point]) -> Chain {
    Chain { start: points[0], segments: points[1..].iter().map(|&p| Segment::Line(p)).collect() }
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
    let turn = |(x, y): Point| {
        let mut p = (x - c, y - c);
        for _ in 0..side {
            p = (-p.1, p.0);
        }
        (p.0 + c, p.1 + c)
    };
    Chain {
        start: turn(chain.start),
        segments: chain
            .segments
            .into_iter()
            .map(|segment| match segment {
                Segment::Line(p) => Segment::Line(turn(p)),
                Segment::Arc { radius, large, sweep, to } => Segment::Arc { radius, large, sweep, to: turn(to) },
            })
            .collect(),
    }
}

// ---- the frame, arrows and footer ----

/// The margin round an SVG maze with a frame, for cells of side `s`.
fn svg_margin(s: f64) -> f64 {
    (2.0 * s).max(48.0)
}

/// Open an SVG document for a maze `w` by `h` pixels, leaving room for the
/// frame if there is one and painting the background if there is one.
pub(crate) fn svg_open(out: &mut String, w: f64, h: f64, s: f64, style: &Style) {
    let m = if style.frame.is_some() { svg_margin(s) } else { 0.0 };
    let (vw, vh) = (py_float_repr(w + 2.0 * m), py_float_repr(h + 2.0 * m));
    let origin = py_float_repr(-m);
    let origin = if m == 0.0 { "0".to_owned() } else { origin };
    write!(
        out,
        r#"<svg id="maze" xmlns="http://www.w3.org/2000/svg" viewBox="{origin} {origin} {vw} {vh}" width="{vw}px" height="{vh}px">"#
    )
    .expect("writing to a String cannot fail");
    if let Some(background) = style.background {
        write!(out, r#"<rect x="{origin}" y="{origin}" width="{vw}" height="{vh}" fill="{}"/>"#, background.hex())
            .expect("writing to a String cannot fail");
    }
}

/// The frame's lines and arrows in maze coordinates, as polylines, for a
/// maze `w` by `h` with cells of side `s` and a margin `m` round it.
fn frame_lines(w: f64, h: f64, s: f64, m: f64) -> Vec<Vec<Point>> {
    let arrow = |from: Point, to: Point| {
        let head = (m * 0.12).min(s * 0.4).max(4.0);
        vec![vec![from, to], vec![(to.0 - head, to.1 - head * 0.7), to, (to.0 - head, to.1 + head * 0.7)]]
    };
    let mut lines = vec![
        // the gaps are the way in, top left, and the way out, bottom right
        vec![(0.0, s), (0.0, h), (w, h)],
        vec![(w, h - s), (w, 0.0), (0.0, 0.0)],
    ];
    lines.extend(arrow((-m * 0.85, s / 2.0), (-m * 0.15, s / 2.0)));
    lines.extend(arrow((w + m * 0.15, h - s / 2.0), (w + m * 0.85, h - s / 2.0)));
    lines
}

/// The frame's labels: text, position (maze coordinates, baseline), size
/// and anchor (0 start, 1 middle, 2 end).
fn frame_labels(grid: &Grid, id: &str, w: f64, h: f64, s: f64, m: f64) -> Vec<(String, Point, f64, u8)> {
    let small = (m * 0.22).clamp(7.0, 12.0);
    let footer = (m * 0.26).clamp(8.0, 14.0);
    let gap = small * 0.6;
    vec![
        ("START".to_owned(), (-m * 0.85, s / 2.0 - gap), small, 0),
        ("FINISH".to_owned(), (w + m * 0.85, h - s / 2.0 - gap), small, 2),
        ("weave maze".to_owned(), (0.0, h + m * 0.55), footer, 0),
        (format!("No. {id}"), (w / 2.0, h + m * 0.55), footer, 1),
        (format!("{} \u{d7} {}", grid.width(), grid.height()), (w, h + m * 0.55), footer, 2),
    ]
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The frame's line: heavier than the walls, but never much more than a
/// tenth of a cell.
fn frame_width(line: f64, s: f64) -> f64 {
    (line * 2.5).min(s * 0.1).max(line)
}

fn arrow_width(line: f64, s: f64) -> f64 {
    (line * 1.2).min(s * 0.06).max(line * 0.6)
}

/// Draw the frame, arrows and footer, if the style has them.
pub(crate) fn svg_frame(out: &mut String, grid: &Grid, s: f64, line: f64, colour: Colour, style: &Style) {
    let Some(id) = &style.frame else { return };
    let (w, h) = (grid.width() as f64 * s, grid.height() as f64 * s);
    let m = svg_margin(s);
    let colour = colour.hex();
    write!(
        out,
        r#"<g fill="none" stroke="{colour}" stroke-linecap="round" stroke-linejoin="round"><path stroke-width=""#
    )
    .expect("writing to a String cannot fail");
    push_fixed_trimmed(out, frame_width(line, s), 2);
    out.push_str(r#"" d=""#);
    let lines = frame_lines(w, h, s, m);
    for line in &lines[..2] {
        polyline_chain(line).push_svg(out);
    }
    out.push_str(r#""/><path stroke-width=""#);
    push_fixed_trimmed(out, arrow_width(line, s), 2);
    out.push_str(r#"" d=""#);
    for line in &lines[2..] {
        polyline_chain(line).push_svg(out);
    }
    write!(out, r#""/></g><g fill="{colour}" font-family="Helvetica, Arial, sans-serif">"#)
        .expect("writing to a String cannot fail");
    for (text, (x, y), size, anchor) in frame_labels(grid, id, w, h, s, m) {
        let anchor = ["start", "middle", "end"][usize::from(anchor)];
        out.push_str(r#"<text x=""#);
        push_fixed_trimmed(out, x, 2);
        out.push_str(r#"" y=""#);
        push_fixed_trimmed(out, y, 2);
        out.push_str(r#"" font-size=""#);
        push_fixed_trimmed(out, size, 2);
        write!(out, r#"" text-anchor="{anchor}" letter-spacing="0.06em">{}</text>"#, xml_escape(&text))
            .expect("writing to a String cannot fail");
    }
    out.push_str("</g>");
}

/// The page layout, with room for the frame if there is one.
pub(crate) fn pdf_layout(grid: &Grid, options: &PdfOptions, style: &Style) -> PageLayout {
    if style.frame.is_some() {
        PageLayout::with_margins(grid, options, 54.0, 66.0)
    } else {
        PageLayout::new(grid, options)
    }
}

/// The transform that draws the maze top down, as in the SVG, on the page.
pub(crate) fn pdf_flip(out: &mut String, grid: &Grid, layout: &PageLayout) {
    out.push_str("1 0 0 -1 ");
    push_fixed_trimmed(out, layout.left_margin, 3);
    out.push(' ');
    push_fixed_trimmed(out, layout.top_margin + grid.height() as f64 * layout.s, 3);
    out.push_str(" cm\n");
}

/// Paint the page's background, if the style has one.
pub(crate) fn pdf_background(out: &mut String, layout: &PageLayout, style: &Style) {
    if let Some(background) = style.background {
        background.push_pdf(out, "rg");
        out.push_str("0 0 ");
        push_fixed_trimmed(out, layout.page_width, 3);
        out.push(' ');
        push_fixed_trimmed(out, layout.page_height, 3);
        out.push_str(" re f\n");
    }
}

/// A rough width of `text` in Helvetica, in units of the font size.
fn helvetica_width(text: &str) -> f64 {
    text.chars()
        .map(|c| match c {
            ' ' | '.' | ',' | 'i' | 'l' | 'j' | 't' | 'f' | 'I' => 0.28,
            'm' | 'w' | 'M' | 'W' => 0.83,
            c if c.is_ascii_uppercase() => 0.67,
            _ => 0.556,
        })
        .sum()
}

/// A string in PDF syntax, in WinAnsi (× is octal 327).
fn pdf_string(text: &str) -> String {
    let mut out = String::from("(");
    for c in text.chars() {
        match c {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\u{d7}' => out.push_str("\\327"),
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            _ => out.push('?'),
        }
    }
    out.push(')');
    out
}

/// Draw the frame, arrows and footer, if the style has them. Must be
/// called outside the flipped maze transform.
pub(crate) fn pdf_frame(out: &mut String, grid: &Grid, layout: &PageLayout, line: f64, colour: Colour, style: &Style) {
    let Some(id) = &style.frame else { return };
    let s = layout.s;
    let (w, h) = (grid.width() as f64 * s, grid.height() as f64 * s);
    let m = layout.left_margin.min(54.0) * 0.9;
    out.push_str("q\n");
    pdf_flip(out, grid, layout);
    colour.push_pdf(out, "RG");
    out.push_str("1 J 1 j\n");
    let lines = frame_lines(w, h, s, m);
    for (group, width) in [(&lines[..2], frame_width(line, s)), (&lines[2..], arrow_width(line, s))] {
        push_fixed_trimmed(out, width, 3);
        out.push_str(" w\n");
        for line in group {
            polyline_chain(line).push_pdf(out);
        }
        out.push_str("S\n");
    }
    out.push_str("Q\n");
    colour.push_pdf(out, "rg");
    let top = layout.top_margin + h;
    for (text, (x, y), size, anchor) in frame_labels(grid, id, w, h, s, m) {
        let width = helvetica_width(&text) * size;
        let x = layout.left_margin + x - width * f64::from(anchor) / 2.0;
        out.push_str("BT /F1 ");
        push_fixed_trimmed(out, size, 2);
        out.push_str(" Tf ");
        push_fixed_trimmed(out, x, 3);
        out.push(' ');
        push_fixed_trimmed(out, top - y, 3);
        writeln!(out, " Td {} Tj ET", pdf_string(&text)).expect("writing to a String cannot fail");
    }
}

/// A complete single-page PDF from its content stream and its Form XObjects
/// (named `/T<cell>`, drawn in tile coordinates for cells of side `s`).
pub(crate) fn pdf_document(content: &str, layout: &PageLayout, forms: &[(u8, String)]) -> Vec<u8> {
    let stream = zlib_compress(content.as_bytes());
    let date = pdf_date();
    let mut media_box = String::from("0 0 ");
    push_fixed_trimmed(&mut media_box, layout.page_width, 3);
    media_box.push(' ');
    push_fixed_trimmed(&mut media_box, layout.page_height, 3);

    // the forms follow the page's own objects, from object 7
    let s = layout.s;
    let mut bbox = String::new();
    for v in [-s, -s, 2.0 * s, 2.0 * s] {
        push_fixed_trimmed(&mut bbox, v, 3);
        bbox.push(' ');
    }
    let mut x_objects = String::new();
    let mut form_objects = Vec::with_capacity(forms.len());
    for (k, (cell, stream)) in forms.iter().enumerate() {
        write!(x_objects, " /T{cell} {} 0 R", k + 7).expect("writing to a String cannot fail");
        form_objects.push(format!(
            "<<\n/BBox [ {bbox}] /FormType 1 /Length {} /Resources <<\n>> /Subtype /Form /Type /XObject\n>>\nstream\n{stream}\nendstream",
            stream.len()
        ));
    }
    let page = format!(
        "<<\n/Contents 5 0 R /MediaBox [ {media_box} ] /Parent 2 0 R /Resources <<\n/Font << /F1 6 0 R >> /XObject <<{x_objects} >>\n>> /Type /Page\n>>"
    );
    let info = format!(
        "<<\n/Author (Dale O'Brien) /CreationDate ({date}) /Creator (maze-rs) /ModDate ({date}) /Producer (maze-rs) /Title (Maze)\n>>"
    );
    let contents = [
        format!("<<\n/Filter /FlateDecode /Length {}\n>>\nstream\n", stream.len()).as_bytes(),
        &stream,
        b"\nendstream",
    ]
    .concat();
    let mut objects: Vec<&[u8]> = vec![
        b"<<\n/Pages 2 0 R /Type /Catalog\n>>",
        b"<<\n/Count 1 /Kids [ 3 0 R ] /Type /Pages\n>>",
        page.as_bytes(),
        info.as_bytes(),
        &contents,
        b"<<\n/BaseFont /Helvetica /Encoding /WinAnsiEncoding /Name /F1 /Subtype /Type1 /Type /Font\n>>",
    ];
    objects.extend(form_objects.iter().map(String::as_bytes));

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

// ---- the styled renderers ----

const WALL: Colour = Colour([0, 0, 0]);
const SOLUTION: Colour = Colour([0xE5, 0x19, 0x19]);

/// Parse SVG path data written by this crate (`M`, `L` and `A` commands)
/// back into chains.
fn parse_path(d: &str) -> Vec<Chain> {
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

/// How wide the solution is drawn, for walls `line` wide and cells of side `s`.
fn solution_width(style: &Style, line: f64, s: f64) -> f64 {
    (line * 2.5).min(style.corridor.unwrap_or(0.6) * s * 0.5)
}

/// Render the maze (and optionally its solution) as an SVG in a style.
#[must_use]
pub fn styled_svg(
    grid: &Grid,
    page: PageSize,
    draw_with_curves: bool,
    solution: Option<&[(isize, isize)]>,
    style: &Style,
) -> String {
    let (width, s) = svg_scale(grid, page);
    let (w, h) = (width * s, grid.height() as f64 * s);
    let line = style.line.unwrap_or(2.0);
    let wall = style.wall_colour.unwrap_or(WALL);
    let path_colour = style.solution_colour.unwrap_or(SOLUTION).hex();
    let curves = style.corners.map_or(draw_with_curves, |c| c == Corners::Round);
    let data = path_data(grid, s, curves, solution);

    let mut out = String::new();
    svg_open(&mut out, w, h, s, style);
    let tiles = style.tiles(s, draw_with_curves);
    let mut used = [false; 32];
    for j in 0..grid.height() {
        for (i, &cell) in grid.row(j).iter().enumerate() {
            used[usize::from(cell)] |= !is_dot(grid, i, j);
        }
    }
    out.push_str("<defs>");
    for cell in (0..32).filter(|&c| used[c]) {
        write!(out, r#"<path id="t{cell}" d="{}"/>"#, tiles[cell]).expect("writing to a String cannot fail");
    }
    write!(out, r#"</defs><g fill="none" stroke="{}" stroke-width=""#, wall.hex())
        .expect("writing to a String cannot fail");
    push_fixed_trimmed(&mut out, line, 2);
    out.push_str(r#"" stroke-linecap="round" stroke-linejoin="round">"#);
    for j in 0..grid.height() {
        for (i, &cell) in grid.row(j).iter().enumerate() {
            if !is_dot(grid, i, j) {
                write!(out, r##"<use href="#t{cell}" x=""##).expect("writing to a String cannot fail");
                push_fixed_trimmed(&mut out, i as f64 * s, 2);
                out.push_str(r#"" y=""#);
                push_fixed_trimmed(&mut out, j as f64 * s, 2);
                out.push_str(r#""/>"#);
            }
        }
    }
    out.push_str("</g>");
    for &(cx, cy, r) in &data.dots {
        out.push_str(r#"<circle cx=""#);
        push_fixed_trimmed(&mut out, cx, 2);
        out.push_str(r#"" cy=""#);
        push_fixed_trimmed(&mut out, cy, 2);
        out.push_str(r#"" r=""#);
        push_fixed_trimmed(&mut out, r, 2);
        write!(out, r#"" fill="{path_colour}"/>"#).expect("writing to a String cannot fail");
    }
    if !data.over.is_empty() {
        let width = solution_width(style, line, s);
        write!(
            out,
            r#"<g fill="none" stroke="{path_colour}" stroke-linecap="round" stroke-linejoin="round" stroke-width=""#
        )
        .expect("writing to a String cannot fail");
        push_fixed_trimmed(&mut out, width, 2);
        write!(
            out,
            r#""><path d="{}"/><path stroke-opacity="0.3" d="{}"/></g>"#,
            data.over.trim_end(),
            data.under.trim_end()
        )
        .expect("writing to a String cannot fail");
    }
    svg_frame(&mut out, grid, s, line, wall, style);
    out.push_str("</svg>");
    out
}

/// Append a filled circle to a PDF content stream.
fn push_pdf_circle(out: &mut String, (cx, cy): Point, r: f64) {
    let start = (cx + r, cy);
    push_pdf_xy(out, start);
    out.push_str(" m\n");
    push_pdf_arc(out, start, (cx - r, cy), r, false, true);
    push_pdf_arc(out, (cx - r, cy), start, r, false, true);
    out.push_str("f\n");
}

/// Render the maze (and optionally its solution) as a PDF in a style. The
/// maze is drawn top down, as in the SVG, with the start at the top left.
#[must_use]
pub fn styled_pdf(grid: &Grid, options: &PdfOptions, solution: Option<&[(isize, isize)]>, style: &Style) -> Vec<u8> {
    let layout = pdf_layout(grid, options, style);
    let s = layout.s;
    let line = style.line.unwrap_or(s / 7.0);
    let wall = style.wall_colour.unwrap_or(WALL);
    let path_colour = style.solution_colour.unwrap_or(SOLUTION);
    let curves = style.corners.map_or(options.draw_with_curves, |c| c == Corners::Round);
    let data = path_data(grid, s, curves, solution);

    let tiles = style.tiles(s, options.draw_with_curves);
    let mut used = [false; 32];
    for j in 0..grid.height() {
        for (i, &cell) in grid.row(j).iter().enumerate() {
            used[usize::from(cell)] |= !is_dot(grid, i, j);
        }
    }
    let forms: Vec<(u8, String)> = (0..32u8)
        .filter(|&c| used[usize::from(c)])
        .map(|cell| {
            let mut stream = String::new();
            for chain in parse_path(&tiles[usize::from(cell)]) {
                chain.push_pdf(&mut stream);
            }
            stream.push('S');
            (cell, stream)
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
    for j in 0..grid.height() {
        for (i, &cell) in grid.row(j).iter().enumerate() {
            if !is_dot(grid, i, j) {
                out.push_str("q 1 0 0 1 ");
                push_pdf_xy(&mut out, (i as f64 * s, j as f64 * s));
                writeln!(out, " cm /T{cell} Do Q").expect("writing to a String cannot fail");
            }
        }
    }
    path_colour.push_pdf(&mut out, "rg");
    path_colour.push_pdf(&mut out, "RG");
    for &(cx, cy, r) in &data.dots {
        push_pdf_circle(&mut out, (cx, cy), r);
    }
    if !data.over.is_empty() {
        let width = solution_width(style, line, s);
        push_fixed_trimmed(&mut out, width, 3);
        out.push_str(" w\n");
        for chain in parse_path(&data.over) {
            chain.push_pdf(&mut out);
        }
        out.push_str("S\n");
        // PDF has no simple transparency, so the parts under bridges are dashed
        out.push('[');
        push_fixed_trimmed(&mut out, width * 0.2, 3);
        out.push(' ');
        push_fixed_trimmed(&mut out, width * 1.6, 3);
        out.push_str("] 0 d\n");
        for chain in parse_path(&data.under) {
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
                let tiles = style.tiles(30.0, true);
                for (cell, d) in tiles.iter().enumerate().take(16).skip(1) {
                    assert!(!d.is_empty(), "{corners:?} {dead_ends:?} {cell}");
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
