//! Benchmarks for each stage of making a maze: generation, solving and the
//! renderers, plain, styled and inked. Run with `cargo bench`.

use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use maze::random::Rng;
use maze::renderers::ink::{ink_pdf, ink_svg, seed_from_id};
use maze::renderers::pdf::{pdf_render, PdfOptions};
use maze::renderers::style::{styled_pdf, styled_svg, Corners, Style};
use maze::renderers::svg::svg_render;
use maze::renderers::text::text_render;
use maze::renderers::PageSize;
use maze::{create_maze, find_solution, Grid};

// (width, height): the default size, a large A0 maze and the README's big one
const SIZES: &[(usize, usize)] = &[(23, 30), (120, 84), (168, 240)];

fn grid(width: usize, height: usize) -> Grid {
    create_maze(width, height, 50, false, &mut Rng::from_str_seed("bench"))
}

fn label(width: usize, height: usize) -> String {
    format!("{width}x{height}")
}

fn bench_seed(c: &mut Criterion) {
    c.bench_function("seed", |b| b.iter(|| Rng::from_str_seed(black_box("6f1c2a2e-bench"))));
}

fn bench_create(c: &mut Criterion) {
    let mut group = c.benchmark_group("create_maze");
    for &(w, h) in SIZES {
        group.bench_with_input(BenchmarkId::from_parameter(label(w, h)), &(w, h), |b, &(w, h)| {
            b.iter(|| grid(w, h));
        });
    }
    group.finish();
}

fn bench_solve(c: &mut Criterion) {
    let mut group = c.benchmark_group("find_solution");
    for &(w, h) in SIZES {
        let g = grid(w, h);
        group.bench_function(label(w, h), |b| b.iter(|| find_solution(black_box(&g))));
    }
    group.finish();
}

fn bench_render(c: &mut Criterion) {
    let mut group = c.benchmark_group("render");
    group.sample_size(20);
    for &(w, h) in SIZES {
        let g = grid(w, h);
        let solution = find_solution(&g);
        group.bench_function(format!("text/{}", label(w, h)), |b| b.iter(|| text_render(black_box(&g))));
        group.bench_function(format!("svg/{}", label(w, h)), |b| {
            b.iter(|| svg_render(black_box(&g), PageSize::A4, true, None));
        });
        group.bench_function(format!("svg_solved/{}", label(w, h)), |b| {
            b.iter(|| svg_render(black_box(&g), PageSize::A4, true, solution.as_deref()));
        });
        let options = PdfOptions { draw_with_curves: true, page: PageSize::A4, landscape: false };
        group.bench_function(format!("pdf/{}", label(w, h)), |b| b.iter(|| pdf_render(black_box(&g), &options)));

        // a style that reshapes the walls and adds the frame
        let style = Style { corners: Some(Corners::Bevel), frame: Some("bench".to_owned()), ..Style::default() };
        group.bench_function(format!("styled_svg/{}", label(w, h)), |b| {
            b.iter(|| styled_svg(black_box(&g), PageSize::A4, true, solution.as_deref(), &style));
        });
        group.bench_function(format!("styled_pdf/{}", label(w, h)), |b| {
            b.iter(|| styled_pdf(black_box(&g), &options, solution.as_deref(), &style));
        });
        let seed = seed_from_id("bench");
        let plain = Style::default();
        group.bench_function(format!("ink_svg/{}", label(w, h)), |b| {
            b.iter(|| ink_svg(black_box(&g), PageSize::A4, true, solution.as_deref(), seed, &plain));
        });
        group.bench_function(format!("ink_pdf/{}", label(w, h)), |b| {
            b.iter(|| ink_pdf(black_box(&g), &options, solution.as_deref(), seed, &plain));
        });
    }
    group.finish();
}

// Criterion's 3s default warm-up per benchmark dominates the run time; 0.2s is
// plenty for these. `--warm-up-time <secs>` on the command line still overrides it.
fn config() -> Criterion {
    Criterion::default().warm_up_time(Duration::from_millis(200))
}

criterion_group! {
    name = benches;
    config = config();
    targets = bench_seed, bench_create, bench_solve, bench_render
}
criterion_main!(benches);
