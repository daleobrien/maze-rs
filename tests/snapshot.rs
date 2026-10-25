//! Locks the generated output for a set of fixed maze ids, so performance
//! work can't change a single byte of what the generator produces.

use maze::maze::{create_maze, find_solution};
use maze::random::PyRandom;
use maze::renderers::pdf::{pdf_content, PdfOptions};
use maze::renderers::svg::svg_render;
use maze::renderers::text::text_render;
use maze::renderers::PageSize;
use sha2::{Digest, Sha256};

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Hash of every output for one maze: text, SVG (curved, straight and
/// solved) and the PDF content stream.
fn outputs(id: &str, width: usize, height: usize, density: i64, add_a_loop: bool) -> String {
    let mut rng = PyRandom::from_str_seed(id);
    let grid = create_maze(width, height, density, add_a_loop, &mut rng);
    let solution = find_solution(&grid);

    let mut all = Vec::new();
    all.extend_from_slice(text_render(&grid).as_bytes());
    for curves in [true, false] {
        all.extend_from_slice(svg_render(&grid, width, height, PageSize::A4, curves, None).as_bytes());
        all.extend_from_slice(svg_render(&grid, width, height, PageSize::A4, curves, solution.as_deref()).as_bytes());
        for landscape in [false, true] {
            let options = PdfOptions { draw_with_curves: curves, page: PageSize::A4, landscape, width, height };
            all.extend_from_slice(pdf_content(&grid, &options).0.as_bytes());
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
    ("abc", 10, 8, 50, false, "ee53e99f5d19a77b2ad6d71d0f2a61c491f5dcd22b1f1d508c5de4f845ea6de6"),
    ("my-maze", 23, 30, 50, false, "d3ba65ad9958820dbe13fbd9dd711af68381adc0a8e42e39ddaf84ee6a34abb7"),
    ("loop", 23, 30, 50, true, "fcc6db79b5f4d8d824a3c5b5518b858fed43ead55b5b2bf24284b6fed63c90c0"),
    ("dense", 40, 40, 10, false, "37bde527c6c4f0c32e2487be0bf161bd2d774e1326f5de65604ef92deabfdf6e"),
    ("sparse", 40, 40, 90, true, "9082904b3a826c17279591a1ca22bb830534b5b1b8ee24657c68bb12d58fedfa"),
    ("big", 120, 84, 50, false, "eedc26ec006c34bd647707d98ba6a01d2d6fbc57d1f071729935dd0695bb08f8"),
    ("tall", 3, 60, 50, false, "252d6b98c36c46b6294cfbb67e096435d6839adf5595d4f966f8b9b32c906a20"),
    ("odd-bands", 97, 61, 30, true, "112c84e25eacc7737d5d036f136fae2f7f6960259fcc097c9e9ab672be5c5722"),
    ("wide", 250, 20, 50, false, "ce29666534fe17f992da80b3df8c149f75b72ea7d5bb879f8e1bd4ceb92d8cd0"),
    ("readme-big", 168, 240, 50, true, "67d9431d28a6e4736b7270515777bd2ba8ec2cd339eeca41478501710bcafaab"),
    ("one", 1, 1, 50, false, "1820aed91eb3adad3668d55748660ad05e0884b354f07d3f48dbff5c81bc45fe"),
];
