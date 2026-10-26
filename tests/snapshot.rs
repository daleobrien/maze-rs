//! Locks the generated output for a set of fixed maze ids, so performance
//! work can't change a single byte of what the generator produces without
//! the hashes here being updated on purpose.

use maze::random::PyRandom;
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
    let mut rng = PyRandom::from_str_seed(id);
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
    ("abc", 10, 8, 50, false, "fd0c1a58abd3a04d46a4b132501ab394268c24b5f8c421452144e7805abf4c55"),
    ("my-maze", 23, 30, 50, false, "ec171a34472d92db4ea926c6128a44a92d66864e6d4eb41a04098d79d48c881e"),
    ("loop", 23, 30, 50, true, "596006e745e52d9dbf7483e456b792a106d9153522dcdbac9e321d10d8d78e69"),
    ("dense", 40, 40, 10, false, "b34126174a2305d7e95e22b4ed2e96c880237bda0287b6421620c3036896dc85"),
    ("sparse", 40, 40, 90, true, "5f75ff5cb7d44c5c0cb648bc5790bcb77636f0e0e50dd0271b2f0a21faf451d7"),
    ("big", 120, 84, 50, false, "e797e308bbd0b41ec523c3e99a22375a34fa17b78b4e2dedcbeaf7fd8be4dcad"),
    ("tall", 3, 60, 50, false, "97212ea2adbca7e102060191ef07c927d75dc1397d7e13cf8bdb9be6ab86d091"),
    ("odd-bands", 97, 61, 30, true, "4da4d703b872bc28a6a88c61d2e296940ed83e603bc736d9241363515d9bab07"),
    ("wide", 250, 20, 50, false, "58770d3a20e0d209165174710203b9e34cd3597933e58b6ffd4b5131c64ad80d"),
    ("readme-big", 168, 240, 50, true, "c75b6dbe4a87aec11d1e89212c9592d005502e31d80d20f0dcf7df15423115f1"),
    ("one", 1, 1, 50, false, "ca45f9dda459f8fe26575271a93592bcbc5576bf25a2337feea2be7cf4aaecf5"),
];
