//! Benchmarks for each stage of making a maze: generation, solving and the
//! three renderers. Run with `cargo bench`.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use maze::random::PyRandom;
use maze::renderers::pdf::{pdf_render, PdfOptions};
use maze::renderers::svg::svg_render;
use maze::renderers::text::text_render;
use maze::renderers::PageSize;
use maze::{create_maze, find_solution, Grid};

// (width, height): the default size, a large A0 maze and the README's big one
const SIZES: &[(usize, usize)] = &[(23, 30), (120, 84), (168, 240)];

fn grid(width: usize, height: usize) -> Grid {
    create_maze(width, height, 50, false, &mut PyRandom::from_str_seed("bench"))
}

fn label(width: usize, height: usize) -> String {
    format!("{width}x{height}")
}

fn bench_seed(c: &mut Criterion) {
    c.bench_function("seed", |b| b.iter(|| PyRandom::from_str_seed(black_box("6f1c2a2e-bench"))));
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
    }
    group.finish();
}

criterion_group!(benches, bench_seed, bench_create, bench_solve, bench_render);
criterion_main!(benches);
