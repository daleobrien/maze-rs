//! The page round a styled or inked maze: its size and background, the
//! frame with START and FINISH arrows and a footer, and the PDF document
//! the page goes into.

use std::fmt::Write as _;

use crate::maze::Grid;
use crate::renderers::path::{polyline_chain, push_pdf_xy, Point};
use crate::renderers::pdf::{flate_stream_object, pdf_date, write_pdf, PageLayout, PdfOptions};
use crate::renderers::style::{Colour, Style};
use crate::renderers::{push_fixed_trimmed, py_float_repr};

/// The least room, in points, a framed PDF leaves at the sides of the maze,
/// and at its top and bottom.
const PDF_FRAME_SIDE: f64 = 54.0;
const PDF_FRAME_TOP: f64 = 66.0;

/// The margin round an SVG maze with a frame, for cells of side `s`.
fn svg_margin(s: f64) -> f64 {
    (2.0 * s).max(48.0)
}

/// Open an SVG document for a maze `w` by `h` pixels, leaving room for the
/// frame if there is one and painting the background if there is one.
pub(crate) fn svg_open(out: &mut String, w: f64, h: f64, s: f64, style: &Style) {
    let m = if style.frame.is_some() { svg_margin(s) } else { 0.0 };
    let (vw, vh) = (py_float_repr(w + 2.0 * m), py_float_repr(h + 2.0 * m));
    let origin = if m == 0.0 { "0".to_owned() } else { py_float_repr(-m) };
    w!(
        out,
        r#"<svg id="maze" xmlns="http://www.w3.org/2000/svg" viewBox="{origin} {origin} {vw} {vh}" width="{vw}px" height="{vh}px">"#
    );
    if let Some(background) = style.background {
        w!(out, r#"<rect x="{origin}" y="{origin}" width="{vw}" height="{vh}" fill="{}"/>"#, background.hex());
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

/// Where a label's position is along it.
#[derive(Clone, Copy)]
enum Anchor {
    Start,
    Middle,
    End,
}

impl Anchor {
    fn svg(self) -> &'static str {
        match self {
            Anchor::Start => "start",
            Anchor::Middle => "middle",
            Anchor::End => "end",
        }
    }

    /// How much of the label's width is before its position.
    fn before(self) -> f64 {
        match self {
            Anchor::Start => 0.0,
            Anchor::Middle => 0.5,
            Anchor::End => 1.0,
        }
    }
}

/// One of the frame's labels, placed in maze coordinates by its baseline.
struct Label {
    text: String,
    at: Point,
    size: f64,
    anchor: Anchor,
}

/// The frame's labels.
fn frame_labels(grid: &Grid, id: &str, w: f64, h: f64, s: f64, m: f64) -> Vec<Label> {
    let small = (m * 0.22).clamp(7.0, 12.0);
    let footer = (m * 0.26).clamp(8.0, 14.0);
    let gap = small * 0.6;
    let label = |text: String, at: Point, size: f64, anchor: Anchor| Label { text, at, size, anchor };
    vec![
        label("START".to_owned(), (-m * 0.85, s / 2.0 - gap), small, Anchor::Start),
        label("FINISH".to_owned(), (w + m * 0.85, h - s / 2.0 - gap), small, Anchor::End),
        label("weave maze".to_owned(), (0.0, h + m * 0.55), footer, Anchor::Start),
        label(format!("No. {id}"), (w / 2.0, h + m * 0.55), footer, Anchor::Middle),
        label(format!("{} \u{d7} {}", grid.width(), grid.height()), (w, h + m * 0.55), footer, Anchor::End),
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
    w!(out, r#"<g fill="none" stroke="{colour}" stroke-linecap="round" stroke-linejoin="round"><path stroke-width=""#);
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
    w!(out, r#""/></g><g fill="{colour}" font-family="Helvetica, Arial, sans-serif">"#);
    for Label { text, at: (x, y), size, anchor } in frame_labels(grid, id, w, h, s, m) {
        out.push_str(r#"<text x=""#);
        push_fixed_trimmed(out, x, 2);
        out.push_str(r#"" y=""#);
        push_fixed_trimmed(out, y, 2);
        out.push_str(r#"" font-size=""#);
        push_fixed_trimmed(out, size, 2);
        w!(out, r#"" text-anchor="{}" letter-spacing="0.06em">{}</text>"#, anchor.svg(), xml_escape(&text));
    }
    out.push_str("</g>");
}

/// The page layout, with room for the frame if there is one.
pub(crate) fn pdf_layout(grid: &Grid, options: &PdfOptions, style: &Style) -> PageLayout {
    if style.frame.is_some() {
        PageLayout::with_margins(grid, options, PDF_FRAME_SIDE, PDF_FRAME_TOP)
    } else {
        PageLayout::new(grid, options)
    }
}

/// The transform that draws the maze top down, as in the SVG, on the page.
pub(crate) fn pdf_flip(out: &mut String, grid: &Grid, layout: &PageLayout) {
    out.push_str("1 0 0 -1 ");
    push_pdf_xy(out, (layout.left_margin, layout.top_margin + grid.height() as f64 * layout.s));
    out.push_str(" cm\n");
}

/// Paint the page's background, if the style has one.
pub(crate) fn pdf_background(out: &mut String, layout: &PageLayout, style: &Style) {
    if let Some(background) = style.background {
        background.push_pdf(out, "rg");
        out.push_str("0 0 ");
        push_pdf_xy(out, (layout.page_width, layout.page_height));
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
    let m = layout.left_margin.min(PDF_FRAME_SIDE) * 0.9;
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
    for Label { text, at: (x, y), size, anchor } in frame_labels(grid, id, w, h, s, m) {
        let width = helvetica_width(&text) * size;
        let x = layout.left_margin + x - width * anchor.before();
        out.push_str("BT /F1 ");
        push_fixed_trimmed(out, size, 2);
        out.push_str(" Tf ");
        push_pdf_xy(out, (x, top - y));
        w!(out, " Td {} Tj ET\n", pdf_string(&text));
    }
}

/// A complete single-page PDF from its content stream and its Form XObjects
/// (named `/T<cell>`, drawn in tile coordinates for cells of side `s`).
pub(crate) fn pdf_document(content: &str, layout: &PageLayout, forms: &[(u8, String)]) -> Vec<u8> {
    let date = pdf_date();
    let mut media_box = String::from("0 0 ");
    push_pdf_xy(&mut media_box, (layout.page_width, layout.page_height));

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
        w!(x_objects, " /T{cell} {} 0 R", k + 7);
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
    let contents = flate_stream_object(content);
    let mut objects: Vec<&[u8]> = vec![
        b"<<\n/Pages 2 0 R /Type /Catalog\n>>",
        b"<<\n/Count 1 /Kids [ 3 0 R ] /Type /Pages\n>>",
        page.as_bytes(),
        info.as_bytes(),
        &contents,
        b"<<\n/BaseFont /Helvetica /Encoding /WinAnsiEncoding /Name /F1 /Subtype /Type1 /Type /Font\n>>",
    ];
    objects.extend(form_objects.iter().map(String::as_bytes));
    write_pdf("1.4", &objects, 1, 4)
}
