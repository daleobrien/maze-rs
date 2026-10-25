use std::f64::consts::PI;
use std::fmt::Write as _;

use crate::maze::{Grid, CROSS_EW, CROSS_NS};
use std::ops::Range;

use crate::renderers::{map_row_bands, page_size, push_fixed_trimmed, py_float_repr, PageSize};

const QUARTER: f64 = PI / 2.0;
const HALF: f64 = PI;
const THREE_QUARTER: f64 = 3.0 * PI / 2.0;
const FULL: f64 = 2.0 * PI;

/// Round to two decimals and strip trailing zeros after the decimal point,
/// appending the result to `out`.
fn push_round(out: &mut String, x: f64) {
    push_fixed_trimmed(out, x, 2);
}

/// Round to two decimals and strip trailing zeros after the decimal point.
fn round(x: f64) -> String {
    let mut s = String::new();
    push_round(&mut s, x);
    s
}

#[derive(Clone, Copy, PartialEq)]
enum Dir {
    T,
    L,
    R,
    B,
}

struct SvgPath {
    svg: String,
    dots: String,
    colour: &'static str,
    stroke_width: u32,
    opacity: &'static str,
    faint: bool,
    draw_with_curves: bool,

    s: f64,
    s2: f64,
    a: f64,
    a2: f64,
    b: f64,
    b2: f64,
    r3: f64,
    r4: f64,
    g: f64,
    n: f64,
    r: f64,
    r2: f64,
    q: f64,
    v: f64,
    delta: f64,
    theta: f64,
}

impl SvgPath {
    fn new(colour: &'static str, stroke_width: u32, opacity: &'static str, s: f64, draw_with_curves: bool, faint: bool) -> Self {
        let g = s * 0.2;
        let k = 0.5;

        let n = -(g / k) + 0.5 * (s - ((g * (4.0 * g - 3.0 * g * k + 2.0 * k * s)) / k).sqrt());

        let r = g / k;
        let q = n + r;
        let v = (g * (-1.0 + k)) / k;

        let theta = ((2.0 * g - 2.0 * g * k + k * s) / (2.0 * g - g * k + k * s)).asin();

        let delta = theta - PI / 2.0;

        SvgPath {
            svg: String::new(),
            dots: String::new(),
            colour,
            stroke_width,
            opacity,
            faint,
            draw_with_curves,
            // inside loop
            a: g,
            a2: g / 2.0,
            b: s - g,
            b2: s - g / 2.0,
            r3: s / 2.0 - g / 2.0,
            r4: s / 2.0 - g,
            g,
            n,
            r,
            r2: s - g - g,
            s,
            s2: s / 2.0,
            q,
            v,
            delta,
            theta,
        }
    }

    /// Append the `<path>` element (and any dots) to `out`.
    fn write_path(&self, out: &mut String) {
        write!(
            out,
            r#"<path stroke-opacity="{}" stroke-width="{}" stroke="{}" d="{}" /> {}"#,
            self.opacity, self.stroke_width, self.colour, self.svg, self.dots
        )
        .unwrap();
    }

    fn dot(&mut self, x: f64, y: f64) {
        let a = round(x + self.s2);
        let b = round(y + self.s2);
        let r = round(self.s / 3.0);

        self.dots += &format!(
            r##"<circle cx="{a}" cy="{b}" r="{r}" fill-opacity="1.0" stroke-opacity="0" fill="#E51919" />"##
        );
    }

    fn polar(x: f64, y: f64, radius: f64, angle: f64) -> (f64, f64) {
        (x + (radius * angle.cos()), y + (radius * angle.sin()))
    }

    fn pos_angle(angle: f64) -> f64 {
        if angle < 0.0 {
            angle + FULL
        } else {
            angle
        }
    }

    fn arc(&mut self, x: f64, y: f64, radius: f64, start_angle: f64, end_angle: f64, dir: i32) {
        let s = Self::pos_angle(start_angle);
        let e = Self::pos_angle(end_angle);
        let (a, b) = Self::polar(x, y, radius, s);
        let (c, d) = Self::polar(x, y, radius, e);

        let diff = e - s;
        let large = if 0.0 < diff && diff <= PI { 1 - dir } else { dir };

        let out = &mut self.svg;
        out.push('L');
        push_round(out, a);
        out.push(' ');
        push_round(out, b);
        out.push_str(" A ");
        let r_start = out.len();
        push_round(out, radius);
        let r_end = out.len();
        out.push(' ');
        out.extend_from_within(r_start..r_end);
        write!(out, " 0 {large} {dir} ").unwrap();
        push_round(out, c);
        out.push(' ');
        push_round(out, d);
        out.push(' ');
    }

    /// Append a command and its point, e.g. `M 1.5 2 `.
    fn command(&mut self, name: &str, x: f64, y: f64) {
        self.svg.push_str(name);
        push_round(&mut self.svg, x);
        self.svg.push(' ');
        push_round(&mut self.svg, y);
        self.svg.push(' ');
    }

    fn m(&mut self, x: f64, y: f64) {
        self.command("M ", x, y);
    }

    fn l(&mut self, x: f64, y: f64) {
        self.command("L ", x, y);
    }

    fn ml(&mut self, a: f64, b: f64, c: f64, d: f64) {
        self.m(a, b);
        self.l(c, d);
    }

    fn mh(&mut self, a: f64, b: f64, x: f64) {
        self.ml(a, b, a + x, b);
    }

    fn mv(&mut self, a: f64, b: f64, y: f64) {
        self.ml(a, b, a, b + y);
    }

    fn h(&mut self, x: f64, y: f64, v: f64) {
        self.ml(x, y + v, x + self.s, y + v);
    }

    fn v(&mut self, x: f64, y: f64, h: f64) {
        self.ml(x + h, y, x + h, y + self.s);
    }

    // ---- solution trace ----

    fn direction(dx: isize, dy: isize) -> Dir {
        if dx == 0 {
            return if dy < 0 { Dir::B } else { Dir::T };
        }
        if dx < 0 {
            Dir::R
        } else {
            Dir::L
        }
    }

    fn trace_tile(&mut self, from: Dir, to: Dir, x: f64, y: f64) {
        use Dir::*;
        match (from, to) {
            (T, L) | (L, T) => self.t9(x, y),
            (T, R) | (R, T) => self.t5(x, y),
            (T, B) | (B, T) => self.t3(x, y),
            (L, R) | (R, L) => self.tc(x, y),
            (L, B) | (B, L) => self.ta(x, y),
            (R, B) | (B, R) => self.t6(x, y),
            _ => panic!("solution doubles back on itself"),
        }
    }

    fn render_trace(&mut self, grid: &Grid, trail: &[(isize, isize)]) {
        let s = self.s;
        let count = trail.windows(3).count();
        for (i, w) in trail.windows(3).enumerate() {
            let (prev, curr, head) = (w[0], w[1], w[2]);

            let from_d = Self::direction(curr.0 - prev.0, curr.1 - prev.1);
            let to_d = Self::direction(curr.0 - head.0, curr.1 - head.1);
            let (cx, cy) = (curr.0 as f64 * s, curr.1 as f64 * s);

            if i == 0 {
                if from_d == Dir::T {
                    self.t2(prev.0 as f64 * s, prev.1 as f64 * s);
                } else if from_d == Dir::L {
                    self.t4(prev.0 as f64 * s, prev.1 as f64 * s);
                }
            }
            if i + 1 == count {
                if to_d == Dir::R {
                    self.t8(head.0 as f64 * s, head.1 as f64 * s);
                }
                if to_d == Dir::B {
                    self.t1(head.0 as f64 * s, head.1 as f64 * s);
                }
            }

            if !self.faint {
                self.trace_tile(from_d, to_d, cx, cy);
            }

            // Python floor division
            let dx = (prev.0 - curr.0).div_euclid(2);
            let dy = (prev.1 - curr.1).div_euclid(2);
            let (px, py) = ((dx + curr.0) as f64 * s, (dy + curr.1) as f64 * s);
            let prev_tile = grid[(dy + curr.1) as usize][(dx + curr.0) as usize];

            if dx == 1 || dx == -1 {
                if prev_tile == CROSS_NS {
                    self.tg(px, py);
                } else if prev_tile == CROSS_EW && !self.faint {
                    self.tc(px, py);
                }
            }

            if dy == 1 || dy == -1 {
                if prev_tile == CROSS_NS && !self.faint {
                    self.t3(px, py);
                } else if prev_tile == CROSS_EW {
                    self.th(px, py);
                }
            }
        }
    }

    fn t1(&mut self, x: f64, y: f64) {
        // │
        // .
        self.mv(x + self.s2, y, self.s2);
    }

    fn t2(&mut self, x: f64, y: f64) {
        // .
        // │
        self.mv(x + self.s2, y + self.s, -self.s2);
    }

    fn t4(&mut self, x: f64, y: f64) {
        // .-
        self.mh(x + self.s2, y + self.s2, self.s2);
    }

    fn t8(&mut self, x: f64, y: f64) {
        // -.
        self.mh(x + self.s2, y + self.s2, -self.s2);
    }

    fn t5(&mut self, x: f64, y: f64) {
        // Medium arc
        // └─
        self.m(x + self.s, y + self.s2);
        if self.draw_with_curves {
            self.arc(x + self.b2, y + self.a2, self.r3, QUARTER, HALF, 1);
        } else {
            self.l(x + self.s2, y + self.s2);
        }
        self.l(x + self.s2, y);
    }

    fn t6(&mut self, x: f64, y: f64) {
        // ┌─
        self.m(x + self.s, y + self.s2);
        if self.draw_with_curves {
            self.arc(x + self.b2, y + self.b2, self.r3, THREE_QUARTER, HALF, 0);
        } else {
            self.l(x + self.s2, y + self.s2);
        }
        self.l(x + self.s2, y + self.s);
    }

    fn t9(&mut self, x: f64, y: f64) {
        // ─┘
        self.m(x + self.s2, y);
        if self.draw_with_curves {
            self.arc(x + self.a2, y + self.a2, self.r3, 0.0, QUARTER, 1);
        } else {
            self.l(x + self.s2, y + self.s2);
        }
        self.l(x, y + self.s2);
    }

    fn ta(&mut self, x: f64, y: f64) {
        // ─┐
        self.m(x, y + self.s2);
        if self.draw_with_curves {
            self.arc(x + self.a2, y + self.b2, self.r3, THREE_QUARTER, FULL, 1);
        } else {
            self.l(x + self.s2, y + self.s2);
        }
        self.l(x + self.s2, y + self.s);
    }

    fn t3(&mut self, x: f64, y: f64) {
        // │
        // │
        self.v(x, y, self.s2);
    }

    fn tc(&mut self, x: f64, y: f64) {
        // ───
        self.h(x, y, self.s2);
    }

    fn tg(&mut self, x: f64, y: f64) {
        if self.faint {
            // Inside to A/B (faint)
            self.mh(x + self.a, y + self.s2, self.r4);
            self.mh(x + self.b, y + self.s2, -self.r4);
        } else {
            // Outside to A/B
            self.mh(x, y + self.s2, self.a);
            self.mh(x + self.s, y + self.s2, -self.a);
        }
    }

    fn th(&mut self, x: f64, y: f64) {
        if self.faint {
            // Inside to A/B (faint)
            self.mv(x + self.s2, y + self.a, self.r4);
            self.mv(x + self.s2, y + self.b, -self.r4);
        } else {
            // Outside to A/B
            self.mv(x + self.s2, y, self.a);
            self.mv(x + self.s2, y + self.s, -self.a);
        }
    }

    // ---- maze walls ----

    /// Draw the walls of the given rows of the maze.
    fn render_walls(&mut self, grid: &Grid, rows: Range<usize>) {
        let left_margin = 0.0;
        let top_margin = 0.0;

        for (j, row) in grid.iter().enumerate().take(rows.end).skip(rows.start) {
            let y = top_margin + j as f64 * self.s;

            for (i, &cell) in row.iter().enumerate() {
                let x = left_margin + i as f64 * self.s;

                if (i == 0 && j == 0) || (i == row.len() - 1 && j == grid.len() - 1) {
                    self.dot(x, y);
                } else {
                    match cell {
                        1 => self.c1(x, y),
                        2 => self.c2(x, y),
                        3 => self.c3(x, y),
                        4 => self.c4(x, y),
                        5 => self.c5(x, y),
                        6 => self.c6(x, y),
                        7 => self.c7(x, y),
                        8 => self.c8(x, y),
                        9 => self.c9(x, y),
                        10 => self.ca(x, y),
                        11 => self.cb(x, y),
                        12 => self.cc(x, y),
                        13 => self.cd(x, y),
                        14 => self.ce(x, y),
                        15 => self.cf(x, y),
                        19 => self.cg(x, y),
                        28 => self.ch(x, y),
                        _ => panic!("no svg tile for cell {cell}"),
                    }
                }
            }
        }
    }

    fn s0(&mut self, x: f64, y: f64) {
        // ┘
        self.m(x + self.a, y);
        if self.draw_with_curves {
            self.arc(x, y, self.a, 0.0, QUARTER, 1);
        } else {
            self.l(x + self.a, y + self.a);
        }
        self.l(x, y + self.a);
    }

    fn s1(&mut self, x: f64, y: f64) {
        // ┐
        self.m(x, y + self.b);
        if self.draw_with_curves {
            self.arc(x, y + self.s, self.a, THREE_QUARTER, FULL, 1);
        } else {
            self.l(x + self.a, y + self.b);
        }
        self.l(x + self.a, y + self.s);
    }

    fn s2(&mut self, x: f64, y: f64) {
        // └
        self.m(x + self.s, y + self.a);
        if self.draw_with_curves {
            self.arc(x + self.s, y, self.a, QUARTER, HALF, 1);
        } else {
            self.l(x + self.b, y + self.a);
        }
        self.l(x + self.b, y);
    }

    fn s3(&mut self, x: f64, y: f64) {
        // ┌
        self.m(x + self.s, y + self.b);
        if self.draw_with_curves {
            self.arc(x + self.s, y + self.s, self.a, THREE_QUARTER, HALF, 0);
        } else {
            self.l(x + self.b, y + self.b);
        }
        self.l(x + self.b, y + self.s);
    }

    fn c1(&mut self, x: f64, y: f64) {
        // │ │
        // └─┘
        self.mv(x + self.b, y, self.q);
        if self.draw_with_curves {
            self.arc(x + self.s - self.v, y + self.n + self.r, self.r, HALF, HALF + self.delta, 0);
            self.arc(
                x + self.s2,
                y + self.s2,
                self.s2 - self.g / 2.0,
                self.theta - QUARTER,
                THREE_QUARTER - self.theta,
                1,
            );
            self.arc(
                x + self.v,
                y + self.n + self.r,
                self.r,
                QUARTER - self.theta,
                THREE_QUARTER + self.theta - self.delta,
                0,
            );
        } else {
            self.l(x + self.b, y + self.b);
            self.l(x + self.a, y + self.b);
        }
        self.l(x + self.g, y);
    }

    fn c2(&mut self, x: f64, y: f64) {
        // ┌─┐
        // │ │
        self.m(x + self.b, y + self.s);
        if self.draw_with_curves {
            self.arc(x + self.s - self.v, y + self.s - self.n - self.r, self.r, HALF, HALF - self.delta, 1);
            self.arc(
                x + self.s2,
                y + self.s2,
                self.s2 - self.g / 2.0,
                QUARTER - self.theta,
                self.theta - THREE_QUARTER,
                0,
            );
            self.arc(
                x + self.v,
                y + self.s - self.n - self.r,
                self.r,
                THREE_QUARTER + self.theta,
                THREE_QUARTER + self.theta - self.delta,
                1,
            );
        } else {
            self.l(x + self.b, y + self.a);
            self.l(x + self.a, y + self.a);
        }
        self.l(x + self.a, y + self.s);
    }

    fn c4(&mut self, x: f64, y: f64) {
        // ┌──
        // └──
        self.m(x + self.s, y + self.b);
        if self.draw_with_curves {
            self.arc(
                x + self.s - self.n - self.r,
                y + self.s - self.v,
                self.r,
                THREE_QUARTER,
                THREE_QUARTER + self.delta,
                0,
            );
            self.arc(
                x + self.s2,
                y + self.s2,
                self.s2 - self.g / 2.0,
                QUARTER + self.delta,
                QUARTER + self.delta - 2.0 * self.theta,
                1,
            );
            self.arc(
                x + self.s - self.n - self.r,
                y + self.v,
                self.r,
                HALF - self.theta,
                HALF - self.theta + self.delta,
                0,
            );
        } else {
            self.l(x + self.g, y + self.b);
            self.l(x + self.a, y + self.a);
        }

        self.l(x + self.s, y + self.a);
    }

    fn c8(&mut self, x: f64, y: f64) {
        // ──┐
        // ──┘
        self.m(x, y + self.b);
        if self.draw_with_curves {
            self.arc(x + self.n + self.r, y + self.s - self.v, self.r, THREE_QUARTER, THREE_QUARTER - self.delta, 1);
            self.arc(
                x + self.s2,
                y + self.s2,
                self.s2 - self.g / 2.0,
                QUARTER - self.delta,
                QUARTER - self.delta + 2.0 * self.theta,
                0,
            );
            self.arc(x + self.n + self.r, y + self.v, self.r, self.theta, self.theta - self.delta, 1);
        } else {
            self.l(x + self.b, y + self.b);
            self.l(x + self.b, y + self.a);
        }
        self.l(x, y + self.a);
    }

    fn c5(&mut self, x: f64, y: f64) {
        // │ └
        // └──
        // Small arc
        self.s2(x, y);

        // Big arc
        self.m(x + self.s, y + self.b);
        if self.draw_with_curves {
            self.arc(x + self.b, y + self.a, self.r2, QUARTER, HALF, 1);
        } else {
            self.l(x + self.a, y + self.b);
        }
        self.l(x + self.a, y);
    }

    fn c6(&mut self, x: f64, y: f64) {
        // ┌──
        // │ ┌
        self.s3(x, y);
        self.m(x + self.s, y + self.a);
        if self.draw_with_curves {
            self.arc(x + self.b, y + self.b, self.r2, THREE_QUARTER, HALF, 0);
        } else {
            self.l(x + self.a, y + self.a);
        }
        self.l(x + self.a, y + self.s);
    }

    fn c9(&mut self, x: f64, y: f64) {
        // ┘ │
        // ──┘
        self.s0(x, y);
        self.m(x + self.b, y);
        if self.draw_with_curves {
            self.arc(x + self.a, y + self.a, self.r2, 0.0, QUARTER, 1);
        } else {
            self.l(x + self.b, y + self.b);
        }
        self.l(x, y + self.b);
    }

    fn ca(&mut self, x: f64, y: f64) {
        // ──┐
        // ┐ │
        self.s1(x, y);
        self.m(x, y + self.a);
        if self.draw_with_curves {
            self.arc(x + self.a, y + self.b, self.r2, THREE_QUARTER, FULL, 1);
        } else {
            self.l(x + self.b, y + self.a);
        }
        self.l(x + self.b, y + self.s);
    }

    fn c7(&mut self, x: f64, y: f64) {
        // │ └
        // │ ┌
        self.v(x, y, self.a);
        self.s2(x, y);
        self.s3(x, y);
    }

    fn cb(&mut self, x: f64, y: f64) {
        // ┘ │
        // ┐ │
        self.v(x, y, self.b);
        self.s0(x, y);
        self.s1(x, y);
    }

    fn c3(&mut self, x: f64, y: f64) {
        // │ │
        // │ │
        self.v(x, y, self.a);
        self.v(x, y, self.b);
    }

    fn cc(&mut self, x: f64, y: f64) {
        // ───
        // ───
        self.h(x, y, self.a);
        self.h(x, y, self.b);
    }

    fn cd(&mut self, x: f64, y: f64) {
        // ┘ └
        // ───
        self.h(x, y, self.b);
        self.s0(x, y);
        self.s2(x, y);
    }

    fn ce(&mut self, x: f64, y: f64) {
        // ───
        // ┐ ┌
        self.h(x, y, self.a);
        self.s1(x, y);
        self.s3(x, y);
    }

    fn cf(&mut self, x: f64, y: f64) {
        // ┘ └
        // ┐ ┌
        self.s0(x, y);
        self.s1(x, y);
        self.s2(x, y);
        self.s3(x, y);
    }

    fn cg(&mut self, x: f64, y: f64) {
        // ┤ ├
        // ┤ ├
        self.v(x, y, self.a);
        self.v(x, y, self.b);

        self.mh(x, y + self.a, self.a);
        self.mh(x, y + self.b, self.a);

        self.mh(x + self.s, y + self.a, self.b - self.s);
        self.mh(x + self.s, y + self.b, self.b - self.s);
    }

    fn ch(&mut self, x: f64, y: f64) {
        // ┴─┴
        // ┬─┬
        self.h(x, y, self.a);
        self.h(x, y, self.b);

        self.mv(x + self.a, y, self.a);
        self.mv(x + self.b, y, self.a);

        self.mv(x + self.a, y + self.b, self.s - self.b);
        self.mv(x + self.b, y + self.b, self.s - self.b);
    }
}

/// Render the maze (and optionally its solution) as an SVG document,
/// matching the Python output byte for byte.
pub fn svg_render(grid: &Grid, width: usize, height: usize, page: PageSize, draw_with_curves: bool, solution: Option<&[(isize, isize)]>) -> String {
    let (page_width, page_height) = page_size(page);
    let left_margin = 0.0;

    // cells must be square, it's the math!, I'm not doing it again.
    // so scale the width if the height will go over the page
    let mut width = width as f64;
    if (height as f64 / width) > (page_height / page_width) {
        width = (page_width / page_height * height as f64).ceil();
    }

    let s = (page_width - 2.0 * left_margin) / width;
    let stroke_width = 2;

    let mut walls = SvgPath::new("#000000", stroke_width, "1.0", s, draw_with_curves, false); // Maze walls
    let mut over_trace = SvgPath::new("#E51919", 5, "1.0", s, draw_with_curves, false); // solution
    let mut under_trace = SvgPath::new("#E51919", 5, "0.2", s, draw_with_curves, true); // faint solution lines

    // rows are independent, so bands of them are drawn in parallel and joined
    let bands = map_row_bands(grid.len(), grid.first().map_or(0, |r| r.len()), |rows| {
        let mut band = SvgPath::new("#000000", stroke_width, "1.0", s, draw_with_curves, false);
        band.render_walls(grid, rows);
        (band.svg, band.dots)
    });
    walls.svg.reserve(bands.iter().map(|b| b.0.len()).sum());
    for (svg, dots) in bands {
        walls.svg.push_str(&svg);
        walls.dots.push_str(&dots);
    }
    let solution = solution.filter(|s| !s.is_empty());
    if let Some(trail) = solution {
        over_trace.render_trace(grid, trail);
        under_trace.render_trace(grid, trail);
    }

    let w = py_float_repr(width * s);
    let h = py_float_repr(height as f64 * s);
    let view_box = format!(r#"viewBox="0 0 {w} {h}" "#);
    let head = format!(
        r#"<svg id="maze" xmlns="http://www.w3.org/2000/svg" {view_box} width="{w}px" height="{h}px" stroke-width="{stroke_width}" fill-opacity="0.0" stroke="black">"#
    );

    // the Python version always appends an (empty) set of extra dots
    let dots = "";
    let mut paths = vec![&walls];
    if solution.is_some() {
        paths.extend([&over_trace, &under_trace]);
    }
    let mut out = String::with_capacity(head.len() + paths.iter().map(|p| p.svg.len() + p.dots.len() + 100).sum::<usize>());
    out.push_str(&head);
    for p in paths {
        out.push(' ');
        p.write_path(&mut out);
    }
    write!(out, " {dots} </svg>").unwrap();
    out
}
