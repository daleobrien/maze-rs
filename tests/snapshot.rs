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
    ("abc", 10, 8, 50, false, "545cc80144fce91de92456628dd8f95cae7adb5b2145f5a5ec01ca56f1371553"),
    ("my-maze", 23, 30, 50, false, "959c47ff7f7ef2d324d45cb284ceca57ae80d368eeb4ac4c31c1fae7b97a3244"),
    ("loop", 23, 30, 50, true, "8fe85c9e4cea9e395c89c87bc2a066b109237c2f53d53e932a3c7b12232b2258"),
    ("dense", 40, 40, 10, false, "daa13855fd16480043a49d4f8ba0fe7206f554ca7789538f0f43f6081b81cb98"),
    ("sparse", 40, 40, 90, true, "b9346214a99f65f9a8a5fe66a1f3b236595164d07746830e751fe91a5325681a"),
    ("big", 120, 84, 50, false, "e68e46a07ab79ec8cf3244a3fe0f667f58e59ed2ebb43aed9ed606124d204187"),
    ("tall", 3, 60, 50, false, "890562ab92edcb1a52f8bebe01f2a8ee0200da8d1379168f9e7906ec00056f84"),
    ("odd-bands", 97, 61, 30, true, "40cb0da9b6e8699e798f391b77d7120059b691f6d021c03fd0fd7bc4225c479d"),
    ("wide", 250, 20, 50, false, "234fc42c8cce1009bb565eccca21e03c500586a67523140f2794a52b750130a9"),
    ("readme-big", 168, 240, 50, true, "37bc4898c4efdca9cbffbb1cc5aa6c984f30487c0b0e83ffdbd845d7d3ffb916"),
    ("one", 1, 1, 50, false, "ca45f9dda459f8fe26575271a93592bcbc5576bf25a2337feea2be7cf4aaecf5"),
];
