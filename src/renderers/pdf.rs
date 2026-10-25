//! PDF output. The Python version draws with ReportLab; this module writes
//! the PDF directly but emits the same page drawing operators ReportLab
//! would (including its number formatting and Bezier arc approximation).

use std::f64::consts::PI;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::write::ZlibEncoder;
use flate2::Compression;

use crate::maze::Grid;
use crate::renderers::{page_size, PageSize};

/// ReportLab's `fp_str` for a single number.
fn fp_num(x: f64) -> String {
    let sa = x.abs();
    if sa <= 1e-7 {
        return "0".to_string();
    }
    let l = if sa <= 1.0 { 6 } else { (6 - sa.log10() as i64).clamp(0, 6) as usize };
    let mut n = format!("{x:.l$}");
    if l > 0 {
        let bytes = n.as_bytes();
        let mut j = bytes.len();
        while j > 0 {
            j -= 1;
            if bytes[j] != b'0' {
                if bytes[j] != b'.' {
                    j += 1;
                }
                break;
            }
        }
        n.truncate(j);
    }
    if n.starts_with('0') && n.len() > 1 {
        n.remove(0);
    }
    n
}

/// ReportLab's `fp_str` for several numbers.
fn fp_str(xs: &[f64]) -> String {
    xs.iter().map(|&x| fp_num(x)).collect::<Vec<_>>().join(" ")
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
    code: Vec<String>,
}

impl Path {
    fn new() -> Self {
        Path { code: vec!["n".to_string()] }
    }

    fn move_to(&mut self, x: f64, y: f64) {
        self.code.push(format!("{} m", fp_str(&[x, y])));
    }

    fn line_to(&mut self, x: f64, y: f64) {
        self.code.push(format!("{} l", fp_str(&[x, y])));
    }

    fn curves(&mut self, curves: &[[f64; 8]], initial_line: bool) {
        let [x, y, ..] = curves[0];
        if initial_line {
            self.line_to(x, y);
        } else {
            self.move_to(x, y);
        }
        for c in curves {
            self.code.push(format!("{} c", fp_str(&c[2..])));
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

    fn code(&self) -> String {
        self.code.join(" ")
    }
}

/// The subset of ReportLab's `Canvas` drawing state the maze needs.
struct Canvas {
    code: Vec<String>,
}

impl Canvas {
    fn translate(&mut self, dx: f64, dy: f64) {
        // ReportLab folds consecutive transforms into one `cm` operator,
        // re-reading the previous (already formatted) values.
        if let Some(last) = self.code.last_mut() {
            if last.ends_with(" cm") {
                let v: Vec<f64> = last.split_whitespace().take(6).map(|t| t.parse().unwrap()).collect();
                let (a0, b0, c0, d0, e0, f0) = (v[0], v[1], v[2], v[3], v[4], v[5]);
                let (a, b, c, d) = (1.0, 0.0, 0.0, 1.0);
                *last = format!(
                    "{} cm",
                    fp_str(&[
                        a0 * a + c0 * b,
                        b0 * a + d0 * b,
                        a0 * c + c0 * d,
                        b0 * c + d0 * d,
                        a0 * dx + c0 * dy + e0,
                        b0 * dx + d0 * dy + f0,
                    ])
                );
                return;
            }
        }
        self.code.push(format!("{} cm", fp_str(&[1.0, 0.0, 0.0, 1.0, dx, dy])));
    }

    fn set_stroke_rgb(&mut self, r: f64, g: f64, b: f64) {
        self.code.push(format!("{} RG", fp_str(&[r, g, b])));
    }

    fn set_fill_rgb(&mut self, r: f64, g: f64, b: f64) {
        self.code.push(format!("{} rg", fp_str(&[r, g, b])));
    }

    fn draw_path(&mut self, p: &Path, fill: bool) {
        self.code.push(p.code());
        self.code.push(if fill { "B*" } else { "S" }.to_string());
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

    let mut c = Canvas { code: Vec::new() };

    // 0=butt,1=round,2=square
    c.code.push("1 J".to_string());

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
    c.code.push(format!("{} w", fp_str(&[stroke])));

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

    for (j, row) in grid.iter().enumerate() {
        // upper/lower rows
        for (i, &cell) in row.iter().enumerate() {
            let x_offset = left_margin + i as f64 * s;
            let y_offset = top_margin + j as f64 * s;

            c.translate(x_offset, y_offset);
            c.set_stroke_rgb(0.0, 0.0, 0.0);
            let mut p = Path::new();

            // mark start and end
            let start = i == 0 && j == height - 1;
            let end = i == org_width - 1 && j == 0;

            if start || end {
                c.set_stroke_rgb(0.9, 0.1, 0.1);
                c.set_fill_rgb(0.9, 0.1, 0.1);
                p.circle(s / 2.0, s / 2.0, g / 1.5);
                c.draw_path(&p, true);

                p = Path::new();
            }

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

            c.draw_path(&p, false);
            c.translate(-x_offset, -y_offset);
        }
    }

    let preamble = "1 0 0 1 0 0 cm  BT /F1 12 Tf 14.4 TL ET";
    let content = format!("{preamble}\n{}\n \n", c.code.join("\n"));
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
