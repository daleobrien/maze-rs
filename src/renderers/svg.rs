//! SVG output. Each kind of tile is drawn once in `<defs>` and placed in its
//! cells with `<use>`; the drawing itself follows the Python version.

use std::f64::consts::PI;
use std::fmt::Write as _;

use crate::maze::{Grid, CROSS_EW, CROSS_NS};
use crate::renderers::{
    map_bands, page_size, push_fixed_trimmed, py_float_repr, used_tiles, Geometry, PageSize, Piece, Solution,
    TILE_KINDS,
};

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
        w!(middle, " 0 {large} {dir} ");

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

/// Path data being drawn: a tile's walls, or the solution.
struct SvgPath {
    svg: String,
    /// Whether this is the faint part of the solution, under crossings.
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
    fn new(s: f64, draw_with_curves: bool, faint: bool) -> Self {
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

    /// Draw `trail`, a path of cells as in a [`Solution`], from the middle of its
    /// first cell to the middle of its last. A solution starts top left and ends
    /// bottom right, but any trail of two or more cells can be drawn.
    fn render_trace(&mut self, grid: &Grid, trail: Solution) {
        let s = self.s;
        let to_page = |(x, y): (isize, isize)| (x as f64 * s, y as f64 * s);
        if let &[first, last] = trail {
            // too short for a turn: just the two half cells, and any crossing
            let (x, y) = to_page(first);
            self.trace_start(Self::side(last.0 - first.0, last.1 - first.1), x, y);
            let (x, y) = to_page(last);
            self.trace_end(Self::side(first.0 - last.0, first.1 - last.1), x, y);
            self.trace_crossing(grid, first, last);
            return;
        }
        let count = trail.len().saturating_sub(2);
        for (i, w) in trail.windows(3).enumerate() {
            let &[prev, curr, head] = w else { unreachable!("windows of 3") };

            let from = Self::side(curr.0 - prev.0, curr.1 - prev.1);
            let to = Self::side(curr.0 - head.0, curr.1 - head.1);
            let (cx, cy) = to_page(curr);

            if i == 0 {
                let (x, y) = to_page(prev);
                self.trace_start(from, x, y);
            }
            if i + 1 == count {
                let (x, y) = to_page(head);
                self.trace_end(to, x, y);
            }

            if !self.faint {
                self.trace_tile(from, to, cx, cy);
            }

            self.trace_crossing(grid, prev, curr);
        }
        // the crossing (if any) before the last cell; a solution never has one
        // there, since crossings are never on the edge of the maze
        if let [.., prev, last] = trail {
            if trail.len() > 2 {
                self.trace_crossing(grid, *prev, *last);
            }
        }
    }

    /// The half of the first cell, at `(x, y)`, on the way out of it: the trail
    /// enters the next cell from its side `from`.
    fn trace_start(&mut self, from: Side, x: f64, y: f64) {
        match from {
            Side::Top => self.t2(x, y),
            Side::Left => self.t4(x, y),
            // only a trail that doesn't start top left goes up or left
            Side::Bottom => self.t1(x, y),
            Side::Right => self.t8(x, y),
        }
    }

    /// The half of the last cell, at `(x, y)`, on the way into it: the trail
    /// leaves the cell before from its side `to`.
    fn trace_end(&mut self, to: Side, x: f64, y: f64) {
        match to {
            Side::Right => self.t8(x, y),
            Side::Bottom => self.t1(x, y),
            // only a trail that doesn't end bottom right comes from below or the right
            Side::Left => self.t4(x, y),
            Side::Top => self.t2(x, y),
        }
    }

    /// The crossing the trail passes over or under between `prev` and `curr`, if
    /// they are two apart.
    fn trace_crossing(&mut self, grid: &Grid, prev: (isize, isize), curr: (isize, isize)) {
        let s = self.s;
        // Python floor division
        let dx = (prev.0 - curr.0).div_euclid(2);
        let dy = (prev.1 - curr.1).div_euclid(2);
        let between = (curr.0 + dx, curr.1 + dy);
        let (px, py) = (between.0 as f64 * s, between.1 as f64 * s);
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
            CROSS_NS => self.cg(x, y),
            CROSS_EW => self.ch(x, y),
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

/// The width of the SVG drawing in cells (at least the grid's width, more
/// when the maze is too tall for the page) and the side of a cell.
pub(crate) fn svg_scale(grid: &Grid, page: PageSize) -> (f64, f64) {
    let (page_width, page_height) = page_size(page);
    let height = grid.height() as f64;

    // cells must be square, it's the math!, I'm not doing it again.
    // so scale the width if the height will go over the page
    let mut width = grid.width() as f64;
    if (height / width) > (page_height / page_width) {
        width = (page_width / page_height * height).ceil();
    }
    (width, page_width / width)
}

/// Whether cell `(i, j)` is the start or end, which get a dot instead of walls.
pub(crate) fn is_dot(grid: &Grid, i: usize, j: usize) -> bool {
    (i, j) == (0, 0) || (i, j) == (grid.width() - 1, grid.height() - 1)
}

/// The start and end dots (centre and radius), in the order the cells come
/// row by row, for cells of side `s`.
pub(crate) fn dots(grid: &Grid, s: f64) -> Vec<(f64, f64, f64)> {
    let mut cells = vec![(0, 0)];
    let last = (grid.width() - 1, grid.height() - 1);
    if last != (0, 0) {
        cells.push(last);
    }
    cells.into_iter().map(|(i, j)| (i as f64 * s + s / 2.0, j as f64 * s + s / 2.0, s / 3.0)).collect()
}

/// The walls of each kind of tile marked in `used`, as SVG path data in the
/// tile's own coordinates (each ending in a space), with cells of side `s`;
/// empty for the kinds not used.
pub(crate) fn plain_tiles(s: f64, draw_with_curves: bool, used: &[bool; TILE_KINDS]) -> Vec<String> {
    let mut walls = SvgPath::new(s, draw_with_curves, false);
    (0..TILE_KINDS as u8)
        .map(|cell| {
            walls.svg.clear();
            if used[usize::from(cell)] {
                walls.draw_tile(cell, 0.0, 0.0);
            }
            walls.svg.clone()
        })
        .collect()
}

/// The solution as SVG path data, in page coordinates, as [`svg_render`]
/// draws it.
pub(crate) struct Traces {
    /// Whether there is a solution to draw (though one of fewer than three
    /// cells draws nothing).
    pub solved: bool,
    /// Where the solution is on top...
    pub over: String,
    /// ...and where it passes under a crossing.
    pub under: String,
}

impl Traces {
    /// The traces of `solution` (empty without one) for cells of side `s`.
    pub(crate) fn new(grid: &Grid, s: f64, draw_with_curves: bool, solution: Option<Solution>) -> Self {
        let mut over = SvgPath::new(s, draw_with_curves, false);
        let mut under = SvgPath::new(s, draw_with_curves, true);
        let solution = solution.filter(|s| !s.is_empty());
        if let Some(trail) = solution {
            over.render_trace(grid, trail);
            under.render_trace(grid, trail);
        }
        Traces { solved: solution.is_some(), over: over.svg, under: under.svg }
    }
}

/// Append a `<use>` of its tile for every cell but the start and end, with
/// cells of side `s`.
pub(crate) fn push_uses(out: &mut String, grid: &Grid, s: f64) {
    // the start of a cell's `<use>` for each kind of tile, its x for each
    // column and the rest of it for each row
    let use_head: Vec<Piece> =
        (0..TILE_KINDS).map(|cell| Piece::new(&format!(r##"<use href="#t{cell}" x=""##))).collect();
    let column_x: Vec<Piece> = (0..grid.width())
        .map(|i| {
            let mut x = String::new();
            push_round(&mut x, i as f64 * s);
            Piece::new(&x)
        })
        .collect();
    let row_tail: Vec<Piece> = (0..grid.height())
        .map(|j| {
            let mut tail = String::from(r#"" y=""#);
            push_round(&mut tail, j as f64 * s);
            tail.push_str(r#""/>"#);
            Piece::new(&tail)
        })
        .collect();

    // rows are independent, so bands of them are drawn in parallel and joined
    let bands = map_bands(grid.height(), grid.width(), |rows| {
        let mut uses = String::with_capacity(rows.len() * grid.width() * 40);
        for j in rows {
            for (i, &cell) in grid.row(j).iter().enumerate() {
                if !is_dot(grid, i, j) {
                    use_head[usize::from(cell)].push_to(&mut uses);
                    column_x[i].push_to(&mut uses);
                    row_tail[j].push_to(&mut uses);
                }
            }
        }
        uses
    });
    out.reserve(bands.iter().map(String::len).sum());
    for band in bands {
        out.push_str(&band);
    }
}

/// Render the maze (and optionally its solution) as an SVG document.
#[must_use]
pub fn svg_render(grid: &Grid, page: PageSize, draw_with_curves: bool, solution: Option<Solution>) -> String {
    let (width, s) = svg_scale(grid, page);
    let height = grid.height() as f64;
    let stroke_width = 2;

    // each kind of tile is drawn once, in its own coordinates, and every cell
    // of that kind refers to it with a `<use>`
    let used = used_tiles(grid, |i, j| is_dot(grid, i, j));
    let tiles = plain_tiles(s, draw_with_curves, &used);
    let mut defs = String::from("<defs>");
    for (cell, d) in tiles.iter().enumerate().filter(|&(cell, _)| used[cell]) {
        w!(defs, r#"<path id="t{cell}" d="{}"/>"#, d.trim_end());
    }
    defs.push_str("</defs>");

    let mut uses = String::new();
    push_uses(&mut uses, grid, s);

    let mut dot_circles = String::new();
    for (cx, cy, r) in dots(grid, s) {
        dot_circles.push_str(r#"<circle cx=""#);
        push_round(&mut dot_circles, cx);
        dot_circles.push_str(r#"" cy=""#);
        push_round(&mut dot_circles, cy);
        dot_circles.push_str(r#"" r=""#);
        push_round(&mut dot_circles, r);
        dot_circles.push_str(r##"" fill-opacity="1.0" stroke-opacity="0" fill="#E51919" />"##);
    }

    let traces = Traces::new(grid, s, draw_with_curves, solution);

    let w = py_float_repr(width * s);
    let h = py_float_repr(height * s);
    let view_box = format!(r#"viewBox="0 0 {w} {h}" "#);
    let head = format!(
        r#"<svg id="maze" xmlns="http://www.w3.org/2000/svg" {view_box} width="{w}px" height="{h}px" stroke-width="{stroke_width}" fill-opacity="0.0" stroke="black">"#
    );

    let mut out = String::with_capacity(
        head.len() + defs.len() + uses.len() + dot_circles.len() + traces.over.len() + traces.under.len() + 400,
    );
    out.push_str(&head);
    out.push(' ');
    out.push_str(&defs);
    w!(
        out,
        r##" <g stroke-opacity="1.0" stroke-width="{stroke_width}" stroke="#000000" stroke-linecap="square">{uses}</g> {dot_circles}"##
    );
    if traces.solved {
        // the solution, and fainter where it passes under a crossing
        for (opacity, d) in [("1.0", &traces.over), ("0.2", &traces.under)] {
            w!(out, r##" <path stroke-opacity="{opacity}" stroke-width="5" stroke="#E51919" d="{d}" /> "##);
        }
    }
    // the Python version always appends an (empty) set of extra dots
    out.push_str("  </svg>");
    out
}
