//! SVG output. Each kind of tile is drawn once in `<defs>` and placed in its
//! cells with `<use>`; the drawing itself follows the Python version.

use std::f64::consts::PI;
use std::fmt::Write as _;

use crate::maze::{Grid, CROSS_EW, CROSS_NS};
use crate::renderers::{map_bands, page_size, push_fixed_trimmed, py_float_repr, Geometry, PageSize};

const QUARTER: f64 = PI / 2.0;
const HALF: f64 = PI;
const THREE_QUARTER: f64 = 3.0 * PI / 2.0;
const FULL: f64 = 2.0 * PI;

/// Round to two decimals and strip trailing zeros after the decimal point,
/// appending the result to `out`.
fn push_round(out: &mut String, x: f64) {
    push_fixed_trimmed(out, x, 2);
}

/// The side of a cell the solution enters or leaves it by.
#[derive(Clone, Copy, PartialEq)]
enum Side {
    Top,
    Left,
    Right,
    Bottom,
}

fn pos_angle(angle: f64) -> f64 {
    if angle < 0.0 {
        angle + FULL
    } else {
        angle
    }
}

/// One arc of a tile. Everything about it that doesn't depend on where the
/// tile is (where its ends are relative to its centre, and its radius and
/// flags as written) is worked out once per render, not once per cell.
struct ArcShape {
    start: (f64, f64),
    end: (f64, f64),
    /// ` A rx ry 0 large sweep `
    middle: String,
}

impl ArcShape {
    fn new(radius: f64, start_angle: f64, end_angle: f64, dir: i32) -> Self {
        let s = pos_angle(start_angle);
        let e = pos_angle(end_angle);

        let diff = e - s;
        let large = if 0.0 < diff && diff <= PI { 1 - dir } else { dir };

        let mut middle = String::from(" A ");
        push_round(&mut middle, radius);
        middle.push(' ');
        push_round(&mut middle, radius);
        write!(middle, " 0 {large} {dir} ").expect("writing to a String cannot fail");

        ArcShape { start: (radius * s.cos(), radius * s.sin()), end: (radius * e.cos(), radius * e.sin()), middle }
    }

    /// Append the arc centred on `(x, y)`, with a line to its start.
    fn draw(&self, out: &mut String, x: f64, y: f64) {
        out.push('L');
        push_round(out, x + self.start.0);
        out.push(' ');
        push_round(out, y + self.start.1);
        out.push_str(&self.middle);
        push_round(out, x + self.end.0);
        out.push(' ');
        push_round(out, y + self.end.1);
        out.push(' ');
    }
}

/// Every arc a tile can have, named after the method that draws it.
struct Arcs {
    s0: ArcShape,
    s1: ArcShape,
    s2: ArcShape,
    s3: ArcShape,
    t5: ArcShape,
    t6: ArcShape,
    t9: ArcShape,
    ta: ArcShape,
    c1: [ArcShape; 3],
    c2: [ArcShape; 3],
    c4: [ArcShape; 3],
    c8: [ArcShape; 3],
    c5: ArcShape,
    c6: ArcShape,
    c9: ArcShape,
    ca: ArcShape,
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
    r4: f64,
    g: f64,
    n: f64,
    r: f64,
    q: f64,
    v: f64,
    arcs: Arcs,
}

impl SvgPath {
    fn new(
        colour: &'static str,
        stroke_width: u32,
        opacity: &'static str,
        s: f64,
        draw_with_curves: bool,
        faint: bool,
    ) -> Self {
        let Geometry { s, g, n, r, q, v, theta } = Geometry::new(s);
        let delta = theta - PI / 2.0;
        // the radii of the small, medium and big quarter circles
        let (a, r3, r2) = (g, s / 2.0 - g / 2.0, s - g - g);
        let arc = ArcShape::new;
        let arcs = Arcs {
            s0: arc(a, 0.0, QUARTER, 1),
            s1: arc(a, THREE_QUARTER, FULL, 1),
            s2: arc(a, QUARTER, HALF, 1),
            s3: arc(a, THREE_QUARTER, HALF, 0),
            t5: arc(r3, QUARTER, HALF, 1),
            t6: arc(r3, THREE_QUARTER, HALF, 0),
            t9: arc(r3, 0.0, QUARTER, 1),
            ta: arc(r3, THREE_QUARTER, FULL, 1),
            c1: [
                arc(r, HALF, HALF + delta, 0),
                arc(r3, theta - QUARTER, THREE_QUARTER - theta, 1),
                arc(r, QUARTER - theta, THREE_QUARTER + theta - delta, 0),
            ],
            c2: [
                arc(r, HALF, HALF - delta, 1),
                arc(r3, QUARTER - theta, theta - THREE_QUARTER, 0),
                arc(r, THREE_QUARTER + theta, THREE_QUARTER + theta - delta, 1),
            ],
            c4: [
                arc(r, THREE_QUARTER, THREE_QUARTER + delta, 0),
                arc(r3, QUARTER + delta, QUARTER + delta - 2.0 * theta, 1),
                arc(r, HALF - theta, HALF - theta + delta, 0),
            ],
            c8: [
                arc(r, THREE_QUARTER, THREE_QUARTER - delta, 1),
                arc(r3, QUARTER - delta, QUARTER - delta + 2.0 * theta, 0),
                arc(r, theta, theta - delta, 1),
            ],
            c5: arc(r2, QUARTER, HALF, 1),
            c6: arc(r2, THREE_QUARTER, HALF, 0),
            c9: arc(r2, 0.0, QUARTER, 1),
            ca: arc(r2, THREE_QUARTER, FULL, 1),
        };

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
            r4: s / 2.0 - g,
            g,
            n,
            r,
            s,
            s2: s / 2.0,
            q,
            v,
            arcs,
        }
    }

    /// Append the `<path>` element (and any dots) to `out`.
    fn write_path(&self, out: &mut String) {
        write!(
            out,
            r#"<path stroke-opacity="{}" stroke-width="{}" stroke="{}" d="{}" /> {}"#,
            self.opacity, self.stroke_width, self.colour, self.svg, self.dots
        )
        .expect("writing to a String cannot fail");
    }

    fn dot(&mut self, x: f64, y: f64) {
        let out = &mut self.dots;
        out.push_str(r#"<circle cx=""#);
        push_round(out, x + self.s2);
        out.push_str(r#"" cy=""#);
        push_round(out, y + self.s2);
        out.push_str(r#"" r=""#);
        push_round(out, self.s / 3.0);
        out.push_str(r##"" fill-opacity="1.0" stroke-opacity="0" fill="#E51919" />"##);
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

    /// The side of a cell that a step of `(dx, dy)` into it comes in by.
    fn side(dx: isize, dy: isize) -> Side {
        match (dx, dy) {
            (0, dy) if dy < 0 => Side::Bottom,
            (0, _) => Side::Top,
            (dx, _) if dx < 0 => Side::Right,
            _ => Side::Left,
        }
    }

    fn trace_tile(&mut self, from: Side, to: Side, x: f64, y: f64) {
        use Side::{Bottom, Left, Right, Top};
        match (from, to) {
            (Top, Left) | (Left, Top) => self.t9(x, y),
            (Top, Right) | (Right, Top) => self.t5(x, y),
            (Top, Bottom) | (Bottom, Top) => self.t3(x, y),
            (Left, Right) | (Right, Left) => self.tc(x, y),
            (Left, Bottom) | (Bottom, Left) => self.ta(x, y),
            (Right, Bottom) | (Bottom, Right) => self.t6(x, y),
            _ => panic!("solution doubles back on itself"),
        }
    }

    fn render_trace(&mut self, grid: &Grid, trail: &[(isize, isize)]) {
        let s = self.s;
        let to_page = |(x, y): (isize, isize)| (x as f64 * s, y as f64 * s);
        let count = trail.len().saturating_sub(2);
        for (i, w) in trail.windows(3).enumerate() {
            let &[prev, curr, head] = w else { unreachable!("windows of 3") };

            let from = Self::side(curr.0 - prev.0, curr.1 - prev.1);
            let to = Self::side(curr.0 - head.0, curr.1 - head.1);
            let (cx, cy) = to_page(curr);

            if i == 0 {
                let (x, y) = to_page(prev);
                match from {
                    Side::Top => self.t2(x, y),
                    Side::Left => self.t4(x, y),
                    _ => {}
                }
            }
            if i + 1 == count {
                let (x, y) = to_page(head);
                match to {
                    Side::Right => self.t8(x, y),
                    Side::Bottom => self.t1(x, y),
                    _ => {}
                }
            }

            if !self.faint {
                self.trace_tile(from, to, cx, cy);
            }

            // Python floor division
            let dx = (prev.0 - curr.0).div_euclid(2);
            let dy = (prev.1 - curr.1).div_euclid(2);
            let between = (curr.0 + dx, curr.1 + dy);
            let (px, py) = to_page(between);
            let prev_tile = grid[(between.0 as usize, between.1 as usize)];

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
            self.arcs.t5.draw(&mut self.svg, x + self.b2, y + self.a2);
        } else {
            self.l(x + self.s2, y + self.s2);
        }
        self.l(x + self.s2, y);
    }

    fn t6(&mut self, x: f64, y: f64) {
        // ┌─
        self.m(x + self.s, y + self.s2);
        if self.draw_with_curves {
            self.arcs.t6.draw(&mut self.svg, x + self.b2, y + self.b2);
        } else {
            self.l(x + self.s2, y + self.s2);
        }
        self.l(x + self.s2, y + self.s);
    }

    fn t9(&mut self, x: f64, y: f64) {
        // ─┘
        self.m(x + self.s2, y);
        if self.draw_with_curves {
            self.arcs.t9.draw(&mut self.svg, x + self.a2, y + self.a2);
        } else {
            self.l(x + self.s2, y + self.s2);
        }
        self.l(x, y + self.s2);
    }

    fn ta(&mut self, x: f64, y: f64) {
        // ─┐
        self.m(x, y + self.s2);
        if self.draw_with_curves {
            self.arcs.ta.draw(&mut self.svg, x + self.a2, y + self.b2);
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

    /// Draw the walls of a tile of kind `cell` with its corner at `(x, y)`.
    fn draw_tile(&mut self, cell: u8, x: f64, y: f64) {
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

    fn s0(&mut self, x: f64, y: f64) {
        // ┘
        self.m(x + self.a, y);
        if self.draw_with_curves {
            self.arcs.s0.draw(&mut self.svg, x, y);
        } else {
            self.l(x + self.a, y + self.a);
        }
        self.l(x, y + self.a);
    }

    fn s1(&mut self, x: f64, y: f64) {
        // ┐
        self.m(x, y + self.b);
        if self.draw_with_curves {
            self.arcs.s1.draw(&mut self.svg, x, y + self.s);
        } else {
            self.l(x + self.a, y + self.b);
        }
        self.l(x + self.a, y + self.s);
    }

    fn s2(&mut self, x: f64, y: f64) {
        // └
        self.m(x + self.s, y + self.a);
        if self.draw_with_curves {
            self.arcs.s2.draw(&mut self.svg, x + self.s, y);
        } else {
            self.l(x + self.b, y + self.a);
        }
        self.l(x + self.b, y);
    }

    fn s3(&mut self, x: f64, y: f64) {
        // ┌
        self.m(x + self.s, y + self.b);
        if self.draw_with_curves {
            self.arcs.s3.draw(&mut self.svg, x + self.s, y + self.s);
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
            self.arcs.c1[0].draw(&mut self.svg, x + self.s - self.v, y + self.n + self.r);
            self.arcs.c1[1].draw(&mut self.svg, x + self.s2, y + self.s2);
            self.arcs.c1[2].draw(&mut self.svg, x + self.v, y + self.n + self.r);
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
            self.arcs.c2[0].draw(&mut self.svg, x + self.s - self.v, y + self.s - self.n - self.r);
            self.arcs.c2[1].draw(&mut self.svg, x + self.s2, y + self.s2);
            self.arcs.c2[2].draw(&mut self.svg, x + self.v, y + self.s - self.n - self.r);
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
            self.arcs.c4[0].draw(&mut self.svg, x + self.s - self.n - self.r, y + self.s - self.v);
            self.arcs.c4[1].draw(&mut self.svg, x + self.s2, y + self.s2);
            self.arcs.c4[2].draw(&mut self.svg, x + self.s - self.n - self.r, y + self.v);
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
            self.arcs.c8[0].draw(&mut self.svg, x + self.n + self.r, y + self.s - self.v);
            self.arcs.c8[1].draw(&mut self.svg, x + self.s2, y + self.s2);
            self.arcs.c8[2].draw(&mut self.svg, x + self.n + self.r, y + self.v);
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
            self.arcs.c5.draw(&mut self.svg, x + self.b, y + self.a);
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
            self.arcs.c6.draw(&mut self.svg, x + self.b, y + self.b);
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
            self.arcs.c9.draw(&mut self.svg, x + self.a, y + self.a);
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
            self.arcs.ca.draw(&mut self.svg, x + self.a, y + self.b);
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

/// Render the maze (and optionally its solution) as an SVG document.
#[must_use]
pub fn svg_render(grid: &Grid, page: PageSize, draw_with_curves: bool, solution: Option<&[(isize, isize)]>) -> String {
    let (page_width, page_height) = page_size(page);
    let height = grid.height() as f64;

    // cells must be square, it's the math!, I'm not doing it again.
    // so scale the width if the height will go over the page
    let mut width = grid.width() as f64;
    if (height / width) > (page_height / page_width) {
        width = (page_width / page_height * height).ceil();
    }

    let s = page_width / width;
    let stroke_width = 2;

    let mut walls = SvgPath::new("#000000", stroke_width, "1.0", s, draw_with_curves, false); // Maze walls
    let mut over_trace = SvgPath::new("#E51919", 5, "1.0", s, draw_with_curves, false); // solution
    let mut under_trace = SvgPath::new("#E51919", 5, "0.2", s, draw_with_curves, true); // faint solution lines

    // the start and end cells get a dot instead of walls
    let (last_x, last_y) = (grid.width() - 1, grid.height() - 1);
    let is_dot = |i: usize, j: usize| (i, j) == (0, 0) || (i, j) == (last_x, last_y);

    // each kind of tile is drawn once, in its own coordinates, and every cell
    // of that kind refers to it with a `<use>`
    let mut used = [false; 32];
    for j in 0..grid.height() {
        for (i, &cell) in grid.row(j).iter().enumerate() {
            used[usize::from(cell)] |= !is_dot(i, j);
        }
    }
    let mut defs = String::from("<defs>");
    for cell in (0..32u8).filter(|&cell| used[usize::from(cell)]) {
        walls.svg.clear();
        walls.draw_tile(cell, 0.0, 0.0);
        write!(defs, r#"<path id="t{cell}" d="{}"/>"#, walls.svg.trim_end()).expect("writing to a String cannot fail");
    }
    defs.push_str("</defs>");

    // the start of a cell's `<use>` for each kind of tile, its x for each
    // column and the rest of it for each row
    let use_head: Vec<String> = (0..32).map(|cell| format!(r##"<use href="#t{cell}" x=""##)).collect();
    let column_x: Vec<String> = (0..grid.width())
        .map(|i| {
            let mut x = String::new();
            push_round(&mut x, i as f64 * s);
            x
        })
        .collect();
    let row_tail: Vec<String> = (0..grid.height())
        .map(|j| {
            let mut tail = String::from(r#"" y=""#);
            push_round(&mut tail, j as f64 * s);
            tail.push_str(r#""/>"#);
            tail
        })
        .collect();

    // rows are independent, so bands of them are drawn in parallel and joined
    let bands = map_bands(grid.height(), grid.width(), |rows| {
        let mut band = SvgPath::new("#000000", stroke_width, "1.0", s, false, false);
        let mut uses = String::with_capacity(rows.len() * grid.width() * 40);
        for j in rows {
            for (i, &cell) in grid.row(j).iter().enumerate() {
                if is_dot(i, j) {
                    band.dot(i as f64 * s, j as f64 * s);
                } else {
                    uses.push_str(&use_head[usize::from(cell)]);
                    uses.push_str(&column_x[i]);
                    uses.push_str(&row_tail[j]);
                }
            }
        }
        (uses, band.dots)
    });
    let mut uses = String::with_capacity(bands.iter().map(|b| b.0.len()).sum());
    for (band_uses, dots) in bands {
        uses.push_str(&band_uses);
        walls.dots.push_str(&dots);
    }
    let solution = solution.filter(|s| !s.is_empty());
    if let Some(trail) = solution {
        over_trace.render_trace(grid, trail);
        under_trace.render_trace(grid, trail);
    }

    let w = py_float_repr(width * s);
    let h = py_float_repr(height * s);
    let view_box = format!(r#"viewBox="0 0 {w} {h}" "#);
    let head = format!(
        r#"<svg id="maze" xmlns="http://www.w3.org/2000/svg" {view_box} width="{w}px" height="{h}px" stroke-width="{stroke_width}" fill-opacity="0.0" stroke="black">"#
    );

    // the Python version always appends an (empty) set of extra dots
    let dots = "";
    let mut out = String::with_capacity(
        head.len() + defs.len() + uses.len() + walls.dots.len() + over_trace.svg.len() + under_trace.svg.len() + 400,
    );
    out.push_str(&head);
    out.push(' ');
    out.push_str(&defs);
    write!(
        out,
        r#" <g stroke-opacity="{}" stroke-width="{}" stroke="{}" stroke-linecap="square">{uses}</g> {}"#,
        walls.opacity, walls.stroke_width, walls.colour, walls.dots
    )
    .expect("writing to a String cannot fail");
    if solution.is_some() {
        for p in [&over_trace, &under_trace] {
            out.push(' ');
            p.write_path(&mut out);
        }
    }
    write!(out, " {dots} </svg>").expect("writing to a String cannot fail");
    out
}
