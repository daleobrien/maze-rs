//! Locks the generated output for a set of fixed maze ids, so performance
//! work can't change a single byte of what the generator produces without
//! the hashes here being updated on purpose. The plain renderers match the
//! Python version; the styled and ink ones are locked too, so refactoring
//! them can't change their output by accident.

use maze::random::Rng;
use maze::renderers::ink::{ink_pdf_content, ink_svg, seed_from_id};
use maze::renderers::pdf::{pdf_content, PdfOptions};
use maze::renderers::style::{styled_pdf, styled_svg, Colour, Corners, DeadEnds, Style};
use maze::renderers::svg::svg_render;
use maze::renderers::text::text_render;
use maze::renderers::PageSize;
use maze::{create_maze, find_solution};
use sha2::{Digest, Sha256};

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Hash of every output for one maze: text, SVG (curved, straight and
/// solved) and the PDF content stream and tile forms.
fn outputs(id: &str, width: usize, height: usize, density: i64, add_a_loop: bool) -> String {
    let mut rng = Rng::from_str_seed(id);
    let grid = create_maze(width, height, density, add_a_loop, &mut rng);
    let solution = find_solution(&grid);

    let mut all = Vec::new();
    all.extend_from_slice(text_render(&grid).as_bytes());
    for curves in [true, false] {
        all.extend_from_slice(svg_render(&grid, PageSize::A4, curves, None).as_bytes());
        all.extend_from_slice(svg_render(&grid, PageSize::A4, curves, solution.as_deref()).as_bytes());
        for landscape in [false, true] {
            let options = PdfOptions { draw_with_curves: curves, page: PageSize::A4, landscape };
            let content = pdf_content(&grid, &options);
            all.extend_from_slice(content.stream.as_bytes());
            for form in &content.forms {
                all.push(form.cell);
                all.extend_from_slice(form.stream.as_bytes());
            }
        }
    }
    sha(&all)
}

#[test]
fn outputs_are_unchanged() {
    for &(id, w, h, d, l, expected) in CASES {
        assert_eq!(outputs(id, w, h, d, l), expected, "maze {id} {w}x{h} density={d} loop={l}");
    }
}

const CASES: &[(&str, usize, usize, i64, bool, &str)] = &[
    ("abc", 10, 8, 50, false, "fca756b914f3f24cc246c5e092166c3449865d74b04b71a94bf330629f23cdaf"),
    ("my-maze", 23, 30, 50, false, "18626630643c63a051b6996e3a6b968c9d1a1aa635208d52b5a8b09bbbc23dd7"),
    ("loop", 23, 30, 50, true, "47523bc175763c0c9f6fb6d7537217fad7e66f12d803d4cc3f683f77cd81fef9"),
    ("dense", 40, 40, 10, false, "08b78950c6bf2ba4b6bbc5b2f48c76283ddc4d2f289d95323364c7973c02af52"),
    ("sparse", 40, 40, 90, true, "1f953872ed15f8963d8f292ecbe7a4a90b6383612bec6fef88e6cd6605c5dfbe"),
    ("big", 120, 84, 50, false, "9272808b67b455ca51906e7ffc9546f371540ab3bd33a5edbe86dad75328c894"),
    ("tall", 3, 60, 50, false, "3e799563853004c87b715a370aca8b161bb6fbf0a3d0c2e8ab482c294deb1cc8"),
    ("odd-bands", 97, 61, 30, true, "f8f34d472cafff8625b71dd7702a2c46fefd3cdd96c2f7808d6987fcc4e28454"),
    ("wide", 250, 20, 50, false, "66b62a31beb7a2058334edcbf851273f984a08dc0c05c475d1197b96b3089e9b"),
    ("readme-big", 168, 240, 50, true, "f4204bcc5c396fb768fd8dfa6977875c578caaa6f3ed99a010b8906824a5cd0c"),
    ("one", 1, 1, 50, false, "ca45f9dda459f8fe26575271a93592bcbc5576bf25a2337feea2be7cf4aaecf5"),
];

/// The style options worth telling apart: none, the frame alone, and a few
/// mixes of every other option.
fn styles(id: &str) -> Vec<Style> {
    vec![
        Style::default(),
        Style { frame: Some(id.to_owned()), ..Style::default() },
        Style {
            corners: Some(Corners::Bevel),
            dead_ends: Some(DeadEnds::Flat),
            corridor: Some(0.4),
            bridge_gap: Some(0.1),
            line: Some(1.5),
            wall_colour: Colour::parse("#123"),
            solution_colour: Colour::parse("#00aa33"),
            background: Colour::parse("#fffaf0"),
            frame: None,
        },
        Style {
            corners: Some(Corners::Square),
            dead_ends: Some(DeadEnds::Round),
            corridor: Some(0.8),
            frame: Some(id.to_owned()),
            ..Style::default()
        },
        Style { corners: Some(Corners::Round), corridor: Some(0.3), bridge_gap: Some(0.15), ..Style::default() },
    ]
}

/// A PDF with its creation and modification dates blanked out.
fn undated(mut pdf: Vec<u8>) -> Vec<u8> {
    for key in [&b"/CreationDate ("[..], b"/ModDate ("] {
        let at = pdf.windows(key.len()).position(|w| w == key).expect("the PDF has dates") + key.len();
        pdf[at..at + 23].fill(b'X');
    }
    pdf
}

/// Hash of every styled and ink output for one maze: SVG and PDF, curved
/// and straight, with and without the solution, in each of [`styles`].
fn styled_outputs(id: &str, width: usize, height: usize, density: i64, add_a_loop: bool) -> String {
    let mut rng = Rng::from_str_seed(id);
    let grid = create_maze(width, height, density, add_a_loop, &mut rng);
    let solution = find_solution(&grid);
    let seed = seed_from_id(id);

    let mut all = Vec::new();
    for style in styles(id) {
        for curves in [true, false] {
            let options = PdfOptions { draw_with_curves: curves, page: PageSize::A4, landscape: false };
            for solved in [None, solution.as_deref()] {
                all.extend_from_slice(styled_svg(&grid, PageSize::A4, curves, solved, &style).as_bytes());
                all.extend_from_slice(&undated(styled_pdf(&grid, &options, solved, &style)));
                all.extend_from_slice(ink_svg(&grid, PageSize::A4, curves, solved, seed, &style).as_bytes());
                all.extend_from_slice(ink_pdf_content(&grid, &options, solved, seed, &style).as_bytes());
            }
        }
    }
    sha(&all)
}

#[test]
fn styled_outputs_are_unchanged() {
    for &(id, w, h, d, l, expected) in STYLED_CASES {
        assert_eq!(styled_outputs(id, w, h, d, l), expected, "maze {id} {w}x{h} density={d} loop={l}");
    }
}

const STYLED_CASES: &[(&str, usize, usize, i64, bool, &str)] = &[
    ("abc", 10, 8, 50, false, "a57b24bab08f677caf7aae7f03bb098f345a081a3c9149bec9104e91d46714d6"),
    ("my-maze", 23, 30, 50, false, "eeaf4f5f4fe899b7e02d1c65f05da6bf7a033fb7895d66dd8759561e3a68300e"),
    ("loop", 23, 30, 50, true, "e81053e03a93ea6d09cbcb83d9f755cf717170d950fcf41883258f5862d5ad37"),
    ("dense", 40, 40, 10, false, "7f32e819ce7c84da735a7793243acc16dd5425cf78831853dcdd55c06d1e77d7"),
    ("tall", 3, 60, 50, false, "4e39d4204ef7420c2fdc6705b3706b8398b29b14b066a46208f7e98a394584b3"),
    ("one", 1, 1, 50, false, "0dc0dd9772ae3bb09b6f7b6575396edb93a77cf6fe081045f82c06673bed94e3"),
];
