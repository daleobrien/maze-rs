//! PDF output. The Python version draws with ReportLab; this module writes
//! the PDF directly but emits the same page drawing operators ReportLab
//! would (including its number formatting and Bezier arc approximation).

use std::f64::consts::PI;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::write::ZlibEncoder;
use flate2::Compression;

use crate::maze::Grid;
use crate::renderers::{page_size, push_fixed, PageSize};

/// ReportLab's `fp_str` for a single number, appended to `out`.
fn push_fp_num(out: &mut String, x: f64) {
    let sa = x.abs();
    if sa <= 1e-7 {
        out.push('0');
        return;
    }
    let l = if sa <= 1.0 { 6 } else { (6 - sa.log10() as i64).clamp(0, 6) as usize };
    let start = out.len();
    push_fixed(out, x, l);
    if l > 0 {
        let bytes = out.as_bytes();
        let mut j = bytes.len();
        while j > start {
            j -= 1;
            if bytes[j] != b'0' {
                if bytes[j] != b'.' {
                    j += 1;
                }
                break;
            }
        }
        out.truncate(j);
    }
    if out.as_bytes()[start] == b'0' && out.len() - start > 1 {
        out.remove(start);
    }
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
    /// Where the last operator starts, if it is a `cm` transform.
    last_cm: Option<usize>,
}

impl Canvas {
    fn new(preamble: &str) -> Self {
        Canvas { code: preamble.to_string(), last_cm: None }
    }

    /// Start a new operator line.
    fn begin_op(&mut self) {
        self.code.push('\n');
        self.last_cm = None;
    }

    fn push_op(&mut self, op: &str) {
        self.begin_op();
        self.code.push_str(op);
    }

    fn push_cm(&mut self, values: &[f64; 6]) {
        self.begin_op();
        self.last_cm = Some(self.code.len());
        push_fp_str(&mut self.code, values);
        self.code.push_str(" cm");
    }

    fn translate(&mut self, dx: f64, dy: f64) {
        // ReportLab folds consecutive transforms into one `cm` operator,
        // re-reading the previous (already formatted) values.
        if let Some(start) = self.last_cm {
            let mut v = [0.0; 6];
            for (slot, t) in v.iter_mut().zip(self.code[start..].split_whitespace()) {
                *slot = t.parse().unwrap();
            }
            let [a0, b0, c0, d0, e0, f0] = v;
            let (a, b, c, d) = (1.0, 0.0, 0.0, 1.0);
            // drop the old operator and its line break; push_cm adds them back
            self.code.truncate(start - 1);
            self.push_cm(&[
                a0 * a + c0 * b,
                b0 * a + d0 * b,
                a0 * c + c0 * d,
                b0 * c + d0 * d,
                a0 * dx + c0 * dy + e0,
                b0 * dx + d0 * dy + f0,
            ]);
            return;
        }
        self.push_cm(&[1.0, 0.0, 0.0, 1.0, dx, dy]);
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

pub struct PdfOptions {
    pub draw_with_curves: bool,
    pub page: PageSize,
    pub landscape: bool,
    pub width: usize,
    pub height: usize,
}

/// The uncompressed page content stream.
pub fn pdf_content(grid: &Grid, options: &PdfOptions) -> (String, f64, f64) {
    let draw_with_curves = options.draw_with_curves;
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
    let height = options.height;
    let org_width = options.width;
    let mut width = options.width as f64;
    let ratio = (page_height - 2.0 * top_margin) / (page_width - 2.0 * left_margin);
    if height as f64 / width > ratio {
        width = (height as f64 / ratio).ceil();
    }

    let s = (page_width - 2.0 * left_margin) / width;

    // center the maze, looks better for mazes that don't fit the page nicely
    left_margin -= (org_width as f64 - width) * s / 2.0;
    top_margin -= (s * height as f64 - (page_height - 2.0 * top_margin)) / 2.0;

    let g = s * 0.2;
    let stroke = s / 7.0;
    c.push_op(&format!("{} w", fp_str(&[stroke])));

    let k = 0.5;

    let n = -(g / k) + 0.5 * (s - ((g * (4.0 * g - 3.0 * g * k + 2.0 * k * s)) / k).sqrt());

    let r = g / k;
    let q = n + r;
    let v = (g * (-1.0 + k)) / k;

    let theta = ((2.0 * g - 2.0 * g * k + k * s) / (2.0 * g - g * k + k * s)).asin() * 180.0 / PI;

    let delta = theta - 90.0;

    let a = g;
    let b = s - g;

    let s_shape_00 = |p: &mut Path| {
        p.move_to(a, 0.0);
        if draw_with_curves {
            p.arc_to(-a, -a, a, a, 0.0, 90.0);
        } else {
            p.line_to(a, a);
        }
        p.line_to(0.0, a);
    };
    let s_shape_01 = |p: &mut Path| {
        p.move_to(0.0, b);
        if draw_with_curves {
            p.arc_to(-a, b, a, s + a, 270.0, 90.0);
        } else {
            p.line_to(a, b);
        }
        p.line_to(a, s);
    };
    let s_shape_10 = |p: &mut Path| {
        p.move_to(s, a);
        if draw_with_curves {
            p.arc_to(b, -a, s + a, a, 90.0, 90.0);
        } else {
            p.line_to(b, a);
        }
        p.line_to(b, 0.0);
    };
    let s_shape_11 = |p: &mut Path| {
        p.move_to(s, b);
        if draw_with_curves {
            p.arc_to(b, b, s + a, s + a, 270.0, -90.0);
        } else {
            p.line_to(b, b);
        }
        p.line_to(b, s);
    };

    // the walls of one tile, in the tile's own coordinates
    let tile_path = |cell: u8, start: bool, end: bool| {
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
                s_shape_10(&mut p);

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
                s_shape_11(&mut p);

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

                s_shape_10(&mut p);
                s_shape_11(&mut p);
            }
            9 => {
                // ┘ │
                // ──┘
                s_shape_00(&mut p);

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
                s_shape_01(&mut p);

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

                s_shape_00(&mut p);
                s_shape_01(&mut p);
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

                s_shape_00(&mut p);
                s_shape_10(&mut p);
            }
            14 => {
                // ───
                // ┐ ┌
                p.move_to(0.0, a);
                p.line_to(s, a);

                s_shape_01(&mut p);
                s_shape_11(&mut p);
            }
            15 => {
                // ┘ └
                // ┐ ┌
                s_shape_00(&mut p);
                s_shape_10(&mut p);
                s_shape_01(&mut p);
                s_shape_11(&mut p);
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
    };
    let mut tile_cache: [Option<String>; 32] = Default::default();

    for (j, row) in grid.iter().enumerate() {
        // upper/lower rows
        for (i, &cell) in row.iter().enumerate() {
            let x_offset = left_margin + i as f64 * s;
            let y_offset = top_margin + j as f64 * s;

            c.translate(x_offset, y_offset);
            c.set_stroke_rgb(0.0, 0.0, 0.0);

            // mark start and end
            let start = i == 0 && j == height - 1;
            let end = i == org_width - 1 && j == 0;

            if start || end {
                c.set_stroke_rgb(0.9, 0.1, 0.1);
                c.set_fill_rgb(0.9, 0.1, 0.1);
                let mut p = Path::new();
                p.circle(s / 2.0, s / 2.0, g / 1.5);
                c.draw_path(&p, true);

                c.draw_path(&tile_path(cell, start, end), false);
            } else {
                // a tile is drawn relative to its own cell, so every cell of
                // the same kind has the same path
                let code = tile_cache[usize::from(cell)].get_or_insert_with(|| tile_path(cell, false, false).code);
                c.draw_path_code(code, false);
            }
            c.translate(-x_offset, -y_offset);
        }
    }

    let mut content = c.code;
    content.push_str("\n \n");
    (content, page_width, page_height)
}

/// Current UTC time in PDF date format.
fn pdf_date() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
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
    format!(
        "D:{year:04}{month:02}{day:02}{:02}{:02}{:02}+00'00'",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Render the maze as a complete single-page PDF document.
pub fn pdf_render(grid: &Grid, options: &PdfOptions) -> Vec<u8> {
    let (content, page_width, page_height) = pdf_content(grid, options);

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(content.as_bytes()).unwrap();
    let stream = encoder.finish().unwrap();

    let date = pdf_date();
    let media_box = fp_str(&[0.0, 0.0, page_width, page_height]);
    let objects: Vec<Vec<u8>> = vec![
        b"<<\n/F1 2 0 R\n>>".to_vec(),
        b"<<\n/BaseFont /Helvetica /Encoding /WinAnsiEncoding /Name /F1 /Subtype /Type1 /Type /Font\n>>".to_vec(),
        format!(
            "<<\n/Contents 7 0 R /MediaBox [ {media_box} ] /Parent 6 0 R /Resources <<\n/Font 1 0 R /ProcSet [ /PDF /Text /ImageB /ImageC /ImageI ]\n>> /Rotate 0 /Type /Page\n>>"
        )
        .into_bytes(),
        b"<<\n/PageMode /UseNone /Pages 6 0 R /Type /Catalog\n>>".to_vec(),
        format!(
            "<<\n/Author (Dale O'Brien) /CreationDate ({date}) /Creator (maze-rs) /Keywords () /ModDate ({date}) /Producer (maze-rs) /Subject () /Title (Maze) /Trapped /False\n>>"
        )
        .into_bytes(),
        b"<<\n/Count 1 /Kids [ 3 0 R ] /Type /Pages\n>>".to_vec(),
        {
            let mut o = format!("<<\n/Filter /FlateDecode /Length {}\n>>\nstream\n", stream.len()).into_bytes();
            o.extend_from_slice(&stream);
            o.extend_from_slice(b"\nendstream");
            o
        },
    ];

    let mut out: Vec<u8> = b"%PDF-1.3\n%\x93\x8c\x8b\x9e maze-rs generated PDF document\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }

    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<<\n/Info 5 0 R\n/Root 4 0 R\n/Size {}\n>>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fp_str_matches_reportlab() {
        assert_eq!(fp_str(&[0.0, 1.0, 0.5, -0.5, 27.028571428571, 597.6, 1e-9]), "0 1 .5 -0.5 27.02857 597.6 0");
        assert_eq!(fp_str(&[1234567.0, 0.9]), "1234567 .9");
    }
}
