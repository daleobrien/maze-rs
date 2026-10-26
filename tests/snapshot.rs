//! Locks the generated output for a set of fixed maze ids, so performance
//! work can't change a single byte of what the generator produces without
//! the hashes here being updated on purpose.

use maze::random::Rng;
use maze::renderers::pdf::{pdf_content, PdfOptions};
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
