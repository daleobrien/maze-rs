//! PDF output. The Python version draws with ReportLab; this module writes
//! the PDF directly but emits the same page drawing operators ReportLab
//! would (including its number formatting and Bezier arc approximation).

use std::f64::consts::PI;
use std::io::Write;
use std::ops::Range;
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::write::ZlibEncoder;
use flate2::{Compress, Compression, FlushCompress, Status};

use crate::maze::Grid;
use crate::renderers::{map_bands, page_size, push_fixed_trimmed, Geometry, PageSize};

/// ReportLab's `fp_str` for a single number, appended to `out`.
///
/// Returns the value of the written number, i.e. what parsing it back would
/// give.
fn push_fp_num(out: &mut String, x: f64) -> f64 {
    let sa = x.abs();
    if sa <= 1e-7 {
        out.push('0');
        return 0.0;
    }
    let l = if sa <= 1.0 { 6 } else { (6 - sa.log10() as i64).clamp(0, 6) as usize };
    let start = out.len();
    let value = push_fixed_trimmed(out, x, l);
    if out.as_bytes()[start] == b'0' && out.len() - start > 1 {
        out.remove(start);
    }
    value
}

/// ReportLab's `fp_str` for several numbers, appended to `out`.
fn push_fp_str(out: &mut String, xs: &[f64]) {
    for (i, &x) in xs.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        push_fp_num(out, x);
    }
}

/// ReportLab's `fp_str` for several numbers.
fn fp_str(xs: &[f64]) -> String {
    let mut out = String::new();
    push_fp_str(&mut out, xs);
    out
}

/// ReportLab's `pdfgeom.bezierArc`.
fn bezier_arc(x1: f64, y1: f64, x2: f64, y2: f64, start_ang: f64, extent: f64) -> Vec<[f64; 8]> {
    let (x1, y1, x2, y2) = (x1.min(x2), y1.max(y2), x1.max(x2), y1.min(y2));

    let (frag_angle, nfrag) = if extent.abs() <= 90.0 {
        (extent, 1)
    } else {
        let nfrag = (extent.abs() / 90.0).ceil() as usize;
        (extent / nfrag as f64, nfrag)
    };

    let x_cen = (x1 + x2) / 2.0;
    let y_cen = (y1 + y2) / 2.0;
    let rx = (x2 - x1) / 2.0;
    let ry = (y2 - y1) / 2.0;
    let half_ang = frag_angle * PI / 360.0;
    let kappa = (4.0 / 3.0 * (1.0 - half_ang.cos()) / half_ang.sin()).abs();

    let mut points = Vec::with_capacity(nfrag);
    for i in 0..nfrag {
        let theta0 = (start_ang + i as f64 * frag_angle) * PI / 180.0;
        let theta1 = (start_ang + (i + 1) as f64 * frag_angle) * PI / 180.0;
        let (c0, s0, c1, s1) = (theta0.cos(), theta0.sin(), theta1.cos(), theta1.sin());
        if frag_angle > 0.0 {
            points.push([
                x_cen + rx * c0,
                y_cen - ry * s0,
                x_cen + rx * (c0 - kappa * s0),
                y_cen - ry * (s0 + kappa * c0),
                x_cen + rx * (c1 + kappa * s1),
                y_cen - ry * (s1 - kappa * c1),
                x_cen + rx * c1,
                y_cen - ry * s1,
            ]);
        } else {
            points.push([
                x_cen + rx * c0,
                y_cen - ry * s0,
                x_cen + rx * (c0 + kappa * s0),
                y_cen - ry * (s0 - kappa * c0),
                x_cen + rx * (c1 - kappa * s1),
                y_cen - ry * (s1 + kappa * c1),
                x_cen + rx * c1,
                y_cen - ry * s1,
            ]);
        }
    }
    points
}

/// Equivalent of ReportLab's `PDFPathObject`.
struct Path {
    code: String,
}

impl Path {
    fn new() -> Self {
        Path { code: "n".to_string() }
    }

    /// Append one operator: its operands, then its name.
    fn op(&mut self, operands: &[f64], name: &str) {
        self.code.push(' ');
        push_fp_str(&mut self.code, operands);
        self.code.push(' ');
        self.code.push_str(name);
    }

    fn move_to(&mut self, x: f64, y: f64) {
        self.op(&[x, y], "m");
    }

    fn line_to(&mut self, x: f64, y: f64) {
        self.op(&[x, y], "l");
    }

    fn curves(&mut self, curves: &[[f64; 8]], initial_line: bool) {
        let [x, y, ..] = curves[0];
        if initial_line {
            self.line_to(x, y);
        } else {
            self.move_to(x, y);
        }
        for c in curves {
            self.op(&c[2..], "c");
        }
    }

    /// Like an arc, but draws a line from the current point to the start.
    fn arc_to(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, start_ang: f64, extent: f64) {
        self.curves(&bezier_arc(x1, y1, x2, y2, start_ang, extent), true);
    }

    fn circle(&mut self, x_cen: f64, y_cen: f64, r: f64) {
        let (x1, y1) = (x_cen - r, y_cen - r);
        let width = 2.0 * r;
        self.curves(&bezier_arc(x1, y1, x1 + width, y1 + width, 0.0, 360.0), false);
    }
}

/// The subset of ReportLab's `Canvas` drawing state the maze needs.
///
/// Operators are written straight into one buffer, one per line.
struct Canvas {
    code: String,
}

impl Canvas {
    fn new(preamble: &str) -> Self {
        Canvas { code: preamble.to_string() }
    }

    /// Start a new operator line.
    fn begin_op(&mut self) {
        self.code.push('\n');
    }

    fn push_op(&mut self, op: &str) {
        self.begin_op();
        self.code.push_str(op);
    }

    fn set_line_width(&mut self, width: f64) {
        self.begin_op();
        push_fp_num(&mut self.code, width);
        self.code.push_str(" w");
    }

    fn set_stroke_rgb(&mut self, r: f64, g: f64, b: f64) {
        self.begin_op();
        push_fp_str(&mut self.code, &[r, g, b]);
        self.code.push_str(" RG");
    }

    fn set_fill_rgb(&mut self, r: f64, g: f64, b: f64) {
        self.begin_op();
        push_fp_str(&mut self.code, &[r, g, b]);
        self.code.push_str(" rg");
    }

    /// Draw a path, given its already built operators.
    fn draw_path_code(&mut self, code: &str, fill: bool) {
        self.push_op(code);
        self.push_op(if fill { "B*" } else { "S" });
    }

    fn draw_path(&mut self, p: &Path, fill: bool) {
        self.draw_path_code(&p.code, fill);
    }
}

/// The walls of each kind of tile, in the tile's own coordinates. Angles
/// are in degrees, as ReportLab takes them.
struct Tiles {
    s: f64,
    g: f64,
    n: f64,
    r: f64,
    q: f64,
    v: f64,
    theta: f64,
    delta: f64,
    a: f64,
    b: f64,
    draw_with_curves: bool,
}

impl Tiles {
    fn new(s: f64, draw_with_curves: bool) -> Self {
        let Geometry { s, g, n, r, q, v, theta } = Geometry::new(s);
        let theta = theta * 180.0 / PI;
        Tiles { s, g, n, r, q, v, theta, delta: theta - 90.0, a: g, b: s - g, draw_with_curves }
    }

    fn s_shape_00(&self, p: &mut Path) {
        let Tiles { a, .. } = *self;
        p.move_to(a, 0.0);
        if self.draw_with_curves {
            p.arc_to(-a, -a, a, a, 0.0, 90.0);
        } else {
            p.line_to(a, a);
        }
        p.line_to(0.0, a);
    }

    fn s_shape_01(&self, p: &mut Path) {
        let Tiles { s, a, b, .. } = *self;
        p.move_to(0.0, b);
        if self.draw_with_curves {
            p.arc_to(-a, b, a, s + a, 270.0, 90.0);
        } else {
            p.line_to(a, b);
        }
        p.line_to(a, s);
    }

    fn s_shape_10(&self, p: &mut Path) {
        let Tiles { s, a, b, .. } = *self;
        p.move_to(s, a);
        if self.draw_with_curves {
            p.arc_to(b, -a, s + a, a, 90.0, 90.0);
        } else {
            p.line_to(b, a);
        }
        p.line_to(b, 0.0);
    }

    fn s_shape_11(&self, p: &mut Path) {
        let Tiles { s, a, b, .. } = *self;
        p.move_to(s, b);
        if self.draw_with_curves {
            p.arc_to(b, b, s + a, s + a, 270.0, -90.0);
        } else {
            p.line_to(b, b);
        }
        p.line_to(b, s);
    }

    /// The walls of a tile; the start and end tiles have a wider arc.
    fn path(&self, cell: u8, start: bool, end: bool) -> Path {
        let Tiles { s, g, n, r, q, v, theta, delta, a, b, draw_with_curves } = *self;
        let mut p = Path::new();
        match cell {
            3 => {
                // │ │
                // │ │
                p.move_to(a, s);
                p.line_to(a, 0.0);
                p.move_to(b, s);
                p.line_to(b, 0.0);
            }
            1 => {
                // │ │
                // └─┘
                p.move_to(b, 0.0);
                if draw_with_curves {
                    p.line_to(b, q);
                    let x = s - v - r;
                    let y = n;
                    p.arc_to(x, y, x + 2.0 * r, y + 2.0 * r, 180.0, delta);

                    p.arc_to(g / 2.0, g / 2.0, s - g / 2.0, s - g / 2.0, theta - 90.0, 360.0 - 2.0 * theta);

                    let x = v - r;
                    p.arc_to(x, y, x + 2.0 * r, y + 2.0 * r, 90.0 - theta, delta);
                } else {
                    p.line_to(b, b);
                    p.line_to(a, b);
                }
                p.line_to(g, 0.0);
            }
            2 => {
                // ┌─┐
                // │ │
                p.move_to(b, s);
                if draw_with_curves {
                    let x = s - v - r;
                    let y = s - n - 2.0 * r;
                    p.arc_to(x, y, x + 2.0 * r, y + 2.0 * r, 180.0, -delta);

                    p.arc_to(g / 2.0, g / 2.0, s - g / 2.0, s - g / 2.0, 90.0 - theta, -360.0 + 2.0 * theta);

                    let x = v - r;
                    p.arc_to(x, y, x + 2.0 * r, y + 2.0 * r, 270.0 + theta, -delta);
                } else {
                    p.line_to(b, a);
                    p.line_to(a, a);
                }
                p.line_to(a, s);
            }
            4 => {
                // ┌──
                // └──
                p.move_to(s, b);
                if draw_with_curves {
                    let x = s - n - 2.0 * r;
                    let y = s - v - r;
                    p.arc_to(x, y, x + 2.0 * r, y + 2.0 * r, 270.0, delta);

                    p.arc_to(g / 2.0, g / 2.0, s - g / 2.0, s - g / 2.0, 90.0 + delta, 360.0 - 2.0 * theta);

                    let y = v - r;
                    p.arc_to(x, y, x + 2.0 * r, y + 2.0 * r, 180.0 - theta, delta);
                } else {
                    p.line_to(g, b);
                    p.line_to(a, a);
                }
                p.line_to(s, a);
            }
            8 => {
                // ──┐
                // ──┘
                p.move_to(0.0, b);
                if draw_with_curves {
                    let x = n;
                    let y = s - v - r;

                    p.arc_to(x, y, x + 2.0 * r, y + 2.0 * r, 270.0, -delta);

                    p.arc_to(g / 2.0, g / 2.0, s - g / 2.0, s - g / 2.0, 90.0 - delta, -360.0 + 2.0 * theta);

                    let y = v - r;
                    p.arc_to(x, y, x + 2.0 * r, y + 2.0 * r, theta, -delta);
                } else {
                    p.line_to(b, b);
                    p.line_to(b, a);
                }
                p.line_to(0.0, a);
            }
            5 => {
                // │ └
                // └──
                self.s_shape_10(&mut p);

                p.move_to(s, b);
                if draw_with_curves {
                    if start {
                        p.arc_to(a, a, b, b, 90.0, 90.0);
                    } else {
                        p.arc_to(a, 2.0 * a - b, 2.0 * b - a, b, 90.0, 90.0);
                    }
                } else {
                    p.line_to(a, b);
                }
                p.line_to(a, 0.0);
            }
            6 => {
                // ┌──
                // │ ┌
                self.s_shape_11(&mut p);

                p.move_to(s, a);
                if draw_with_curves {
                    p.arc_to(a, a, 2.0 * b + a, 2.0 * b + a, 270.0, -90.0);
                } else {
                    p.line_to(a, a);
                }
                p.line_to(a, s);
            }
            7 => {
                // │ └
                // │ ┌
                p.move_to(a, s);
                p.line_to(a, 0.0);

                self.s_shape_10(&mut p);
                self.s_shape_11(&mut p);
            }
            9 => {
                // ┘ │
                // ──┘
                self.s_shape_00(&mut p);

                p.move_to(b, 0.0);
                if draw_with_curves {
                    p.arc_to(2.0 * a - b, 2.0 * a - b, b, b, 0.0, 90.0);
                } else {
                    p.line_to(b, b);
                }
                p.line_to(0.0, b);
            }
            10 => {
                // ──┐
                // ┐ │
                self.s_shape_01(&mut p);

                p.move_to(0.0, a);
                if draw_with_curves {
                    if end {
                        p.arc_to(a, a, b, b, 270.0, 90.0);
                    } else {
                        p.arc_to(2.0 * a - b, a, b, 2.0 * b + a, 270.0, 90.0);
                    }
                } else {
                    p.line_to(b, a);
                }
                p.line_to(b, s);
            }
            11 => {
                // ┘ │
                // ┐ │
                p.move_to(b, s);
                p.line_to(b, 0.0);

                self.s_shape_00(&mut p);
                self.s_shape_01(&mut p);
            }
            12 => {
                // ───
                // ───
                p.move_to(0.0, b);
                p.line_to(s, b);
                p.move_to(0.0, a);
                p.line_to(s, a);
            }
            13 => {
                // ┘ └
                // ───
                p.move_to(0.0, b);
                p.line_to(s, b);

                self.s_shape_00(&mut p);
                self.s_shape_10(&mut p);
            }
            14 => {
                // ───
                // ┐ ┌
                p.move_to(0.0, a);
                p.line_to(s, a);

                self.s_shape_01(&mut p);
                self.s_shape_11(&mut p);
            }
            15 => {
                // ┘ └
                // ┐ ┌
                self.s_shape_00(&mut p);
                self.s_shape_10(&mut p);
                self.s_shape_01(&mut p);
                self.s_shape_11(&mut p);
            }
            19 => {
                // ┤ ├
                // ┤ ├
                p.move_to(a, s);
                p.line_to(a, 0.0);
                p.move_to(b, s);
                p.line_to(b, 0.0);

                p.move_to(0.0, a);
                p.line_to(a, a);
                p.move_to(0.0, b);
                p.line_to(a, b);

                p.move_to(s, a);
                p.line_to(b, a);
                p.move_to(s, b);
                p.line_to(b, b);
            }
            28 => {
                // ┴─┴
                // ┬─┬
                p.move_to(0.0, b);
                p.line_to(s, b);
                p.move_to(0.0, a);
                p.line_to(s, a);

                p.move_to(a, a);
                p.line_to(a, 0.0);
                p.move_to(a, b);
                p.line_to(a, s);

                p.move_to(b, a);
                p.line_to(b, 0.0);
                p.move_to(b, b);
                p.line_to(b, s);
            }
            _ => {}
        }
        p
    }
}

/// The `cm` transforms that move to each cell, written once per column and
/// row rather than once per cell.
///
/// ReportLab moves to a cell with `translate(x, y)` and back with
/// `translate(-x, -y)`, and folds each move into the transform before it,
/// re-reading that one's (already formatted) values. So a cell's transform is
/// `1 0 0 1 x' y' cm`, where `x'` is its `x` plus the written `-x` of the cell
/// before, and likewise for `y'`: it depends only on the two cells' columns
/// and the two cells' rows.
struct Placement {
    /// The `x'` of each column; the first column's follows the last column
    /// of the row above.
    x: Vec<String>,
    /// The `y'` of each row when the cell before is in the same row...
    y_along: Vec<String>,
    /// ...and when it is at the end of the row above.
    y_down: Vec<String>,
    /// The move to the first cell, which follows no other move.
    first: String,
    /// The move back from the last cell, which nothing follows.
    last: String,
}

impl Placement {
    fn new(xs: &[f64], ys: &[f64]) -> Self {
        // the written value of each move back
        let back = |xs: &[f64]| -> Vec<f64> { xs.iter().map(|&x| push_fp_num(&mut String::new(), -x)).collect() };
        let (back_x, back_y) = (back(xs), back(ys));

        // ReportLab's `a0 * dx + c0 * dy + e0` (and likewise for y), where the
        // folded transform's `a0` is 1 and `c0` is 0; the maze sits inside the
        // margins, so the other offset is positive and `0 * dy` adds nothing
        let fold = |d: f64, e0: f64| {
            let mut out = String::new();
            push_fp_num(&mut out, d + e0);
            out
        };
        let columns = xs.len();
        let x = (0..columns).map(|i| fold(xs[i], back_x[(i + columns - 1) % columns])).collect();
        let y_along = (0..ys.len()).map(|j| fold(ys[j], back_y[j])).collect();
        let y_down = (0..ys.len()).map(|j| if j == 0 { String::new() } else { fold(ys[j], back_y[j - 1]) }).collect();

        let cm = |x: f64, y: f64| {
            let mut out = String::from("1 0 0 1 ");
            push_fp_num(&mut out, x);
            out.push(' ');
            push_fp_num(&mut out, y);
            out.push_str(" cm");
            out
        };
        let first = cm(xs[0], ys[0]);
        let last = cm(-xs[columns - 1], -ys[ys.len() - 1]);
        Placement { x, y_along, y_down, first, last }
    }

    /// Append the move to cell `(i, j)`, folded into the move back from the
    /// cell before it.
    fn push_move(&self, c: &mut Canvas, i: usize, j: usize) {
        if (i, j) == (0, 0) {
            c.push_op(&self.first);
            return;
        }
        let y = if i == 0 { &self.y_down[j] } else { &self.y_along[j] };
        c.push_op("1 0 0 1 ");
        c.code.push_str(&self.x[i]);
        c.code.push(' ');
        c.code.push_str(y);
        c.code.push_str(" cm");
    }
}

pub struct PdfOptions {
    pub draw_with_curves: bool,
    pub page: PageSize,
    pub landscape: bool,
}

/// A page's uncompressed content stream and its size in points.
pub struct PageContent {
    pub stream: String,
    pub width: f64,
    pub height: f64,
}

/// The uncompressed page content stream.
#[must_use]
pub fn pdf_content(grid: &Grid, options: &PdfOptions) -> PageContent {
    let (mut page_width, mut page_height) = page_size(options.page);
    if options.landscape {
        std::mem::swap(&mut page_width, &mut page_height);
    }

    let mut c = Canvas::new("1 0 0 1 0 0 cm  BT /F1 12 Tf 14.4 TL ET");

    // 0=butt,1=round,2=square
    c.push_op("1 J");

    let mut left_margin = 15.0;
    let mut top_margin = 15.0;

    // cells must be square, it's the math!, I'm not doing it again.
    // so scale the width if the height will go over the page
    let (org_width, height) = (grid.width(), grid.height());
    let mut width = org_width as f64;
    let ratio = (page_height - 2.0 * top_margin) / (page_width - 2.0 * left_margin);
    if height as f64 / width > ratio {
        width = (height as f64 / ratio).ceil();
    }

    let s = (page_width - 2.0 * left_margin) / width;

    // center the maze, looks better for mazes that don't fit the page nicely
    left_margin -= (org_width as f64 - width) * s / 2.0;
    top_margin -= (s * height as f64 - (page_height - 2.0 * top_margin)) / 2.0;

    c.set_line_width(s / 7.0);

    let tiles = Tiles::new(s, options.draw_with_curves);

    // a tile is drawn relative to its own cell, so every cell of the same
    // kind has the same path
    let tile_code: Vec<String> = (0..32).map(|cell| tiles.path(cell, false, false).code).collect();

    let xs: Vec<f64> = (0..org_width).map(|i| left_margin + i as f64 * s).collect();
    let ys: Vec<f64> = (0..height).map(|j| top_margin + j as f64 * s).collect();
    let placement = Placement::new(&xs, &ys);

    let draw_rows = |c: &mut Canvas, rows: Range<usize>| {
        for j in rows {
            // upper/lower rows
            for (i, &cell) in grid.row(j).iter().enumerate() {
                placement.push_move(c, i, j);
                c.push_op("0 0 0 RG");

                // mark start and end
                let start = i == 0 && j == height - 1;
                let end = i == org_width - 1 && j == 0;

                if start || end {
                    c.set_stroke_rgb(0.9, 0.1, 0.1);
                    c.set_fill_rgb(0.9, 0.1, 0.1);
                    let mut p = Path::new();
                    p.circle(s / 2.0, s / 2.0, tiles.g / 1.5);
                    c.draw_path(&p, true);

                    c.draw_path(&tiles.path(cell, start, end), false);
                } else {
                    c.draw_path_code(&tile_code[usize::from(cell)], false);
                }
            }
        }
    };

    // bands of rows are drawn in parallel; each cell's move back is folded
    // into the next cell's move, so only the last one is written
    let bands = map_bands(height, org_width, |rows| {
        // a cell takes its tile, the operators around it and its move
        let size = rows.clone().flat_map(|j| grid.row(j)).map(|&cell| tile_code[usize::from(cell)].len() + 64).sum();
        let mut c = Canvas { code: String::with_capacity(size) };
        draw_rows(&mut c, rows);
        c.code
    });
    c.code.reserve(bands.iter().map(String::len).sum::<usize>() + placement.last.len() + 4);
    for band in bands {
        c.code.push_str(&band);
    }
    c.push_op(&placement.last);

    let mut stream = c.code;
    stream.push_str("\n \n");
    PageContent { stream, width: page_width, height: page_height }
}

/// Below this a content stream is compressed on one thread.
const PARALLEL_COMPRESS_MIN_BYTES: usize = 1 << 20;

/// Compress `data` into a zlib stream.
///
/// Large inputs are split into pieces that are deflated in parallel, each
/// ending on a byte boundary (a sync flush), so the pieces join into one
/// ordinary zlib stream, as `pigz` does.
fn zlib_compress(data: &[u8]) -> Vec<u8> {
    if data.len() < PARALLEL_COMPRESS_MIN_BYTES {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data).expect("writing to a Vec cannot fail");
        return encoder.finish().expect("writing to a Vec cannot fail");
    }

    let pieces = map_bands(data.len(), 1, |range| {
        let last = range.end == data.len();
        let input = &data[range.clone()];
        let mut deflate = Compress::new(Compression::default(), false);
        let mut out = Vec::with_capacity(input.len() / 4 + 1024);
        let flush = if last { FlushCompress::Finish } else { FlushCompress::Sync };
        loop {
            let consumed = deflate.total_in() as usize;
            let status = deflate.compress_vec(&input[consumed..], &mut out, flush).expect("deflate accepts any input");
            let done = deflate.total_in() as usize == input.len() && out.len() < out.capacity();
            if status == Status::StreamEnd || (!last && done) {
                break;
            }
            out.reserve(out.capacity().max(1024));
        }
        (out, adler32(input), input.len())
    });

    // zlib header for the default compression level
    let mut out = vec![0x78, 0x9c];
    let mut checksum = 1;
    for (piece, piece_checksum, len) in pieces {
        out.extend_from_slice(&piece);
        checksum = adler32_combine(checksum, piece_checksum, len);
    }
    out.extend_from_slice(&checksum.to_be_bytes());
    out
}

const ADLER_BASE: u32 = 65_521;

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    // the most bytes that can be summed before the sums can overflow
    for chunk in data.chunks(5552) {
        for &byte in chunk {
            a += u32::from(byte);
            b += a;
        }
        a %= ADLER_BASE;
        b %= ADLER_BASE;
    }
    (b << 16) | a
}

/// The Adler-32 of two pieces joined, from the checksum of each and the
/// length of the second (zlib's `adler32_combine`).
fn adler32_combine(adler1: u32, adler2: u32, len2: usize) -> u32 {
    let rem = (len2 % ADLER_BASE as usize) as u32;
    let mut sum1 = adler1 & 0xffff;
    let mut sum2 = (rem * sum1) % ADLER_BASE;
    sum1 += (adler2 & 0xffff) + ADLER_BASE - 1;
    sum2 += (adler1 >> 16) + (adler2 >> 16) + ADLER_BASE - rem;
    if sum1 >= ADLER_BASE {
        sum1 -= ADLER_BASE;
    }
    if sum1 >= ADLER_BASE {
        sum1 -= ADLER_BASE;
    }
    if sum2 >= 2 * ADLER_BASE {
        sum2 -= 2 * ADLER_BASE;
    }
    if sum2 >= ADLER_BASE {
        sum2 -= ADLER_BASE;
    }
    (sum2 << 16) | sum1
}

/// Current UTC time in PDF date format.
fn pdf_date() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs().try_into().unwrap_or(i64::MAX));
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // civil-from-days (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("D:{year:04}{month:02}{day:02}{:02}{:02}{:02}+00'00'", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Render the maze as a complete single-page PDF document.
#[must_use]
pub fn pdf_render(grid: &Grid, options: &PdfOptions) -> Vec<u8> {
    let content = pdf_content(grid, options);

    let stream = zlib_compress(content.stream.as_bytes());

    let date = pdf_date();
    let media_box = fp_str(&[0.0, 0.0, content.width, content.height]);
    let page = format!(
        "<<\n/Contents 7 0 R /MediaBox [ {media_box} ] /Parent 6 0 R /Resources <<\n/Font 1 0 R /ProcSet [ /PDF /Text /ImageB /ImageC /ImageI ]\n>> /Rotate 0 /Type /Page\n>>"
    );
    let info = format!(
        "<<\n/Author (Dale O'Brien) /CreationDate ({date}) /Creator (maze-rs) /Keywords () /ModDate ({date}) /Producer (maze-rs) /Subject () /Title (Maze) /Trapped /False\n>>"
    );
    let contents = [
        format!("<<\n/Filter /FlateDecode /Length {}\n>>\nstream\n", stream.len()).as_bytes(),
        &stream,
        b"\nendstream",
    ]
    .concat();
    let objects: [&[u8]; 7] = [
        b"<<\n/F1 2 0 R\n>>",
        b"<<\n/BaseFont /Helvetica /Encoding /WinAnsiEncoding /Name /F1 /Subtype /Type1 /Type /Font\n>>",
        page.as_bytes(),
        b"<<\n/PageMode /UseNone /Pages 6 0 R /Type /Catalog\n>>",
        info.as_bytes(),
        b"<<\n/Count 1 /Kids [ 3 0 R ] /Type /Pages\n>>",
        &contents,
    ];

    let mut out: Vec<u8> = b"%PDF-1.3\n%\x93\x8c\x8b\x9e maze-rs generated PDF document\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        writeln!(out, "{} 0 obj", i + 1).expect("writing to a Vec cannot fail");
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }

    let xref = out.len();
    let size = objects.len() + 1;
    write!(out, "xref\n0 {size}\n0000000000 65535 f \n").expect("writing to a Vec cannot fail");
    for offset in offsets {
        writeln!(out, "{offset:010} 00000 n ").expect("writing to a Vec cannot fail");
    }
    write!(out, "trailer\n<<\n/Info 5 0 R\n/Root 4 0 R\n/Size {size}\n>>\nstartxref\n{xref}\n%%EOF\n")
        .expect("writing to a Vec cannot fail");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_compression_round_trips() {
        use std::io::Read;

        let data: Vec<u8> = (0..3_000_000u32).flat_map(|i| format!("{} {} l ", i % 997, i / 7).into_bytes()).collect();
        let compressed = zlib_compress(&data);
        let mut decoded = Vec::new();
        // the decoder checks the Adler-32 trailer too
        flate2::read::ZlibDecoder::new(&compressed[..]).read_to_end(&mut decoded).unwrap();
        assert!(decoded == data);

        let small = b"1 0 0 1 0 0 cm S";
        assert_eq!(adler32_combine(adler32(&small[..5]), adler32(&small[5..]), small.len() - 5), adler32(small));
    }

    #[test]
    fn fp_str_matches_reportlab() {
        assert_eq!(fp_str(&[0.0, 1.0, 0.5, -0.5, 27.028571428571, 597.6, 1e-9]), "0 1 .5 -0.5 27.02857 597.6 0");
        assert_eq!(fp_str(&[1234567.0, 0.9]), "1234567 .9");
    }
}
