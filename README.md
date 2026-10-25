maze-rs
=======

A Rust port of the Python weave-maze generator in `../maze`. Same command
line, same output: for a given `-i MAZE_ID` the text and SVG output is byte
for byte identical to the Python version, and the PDF page drawing is
identical to what ReportLab produces.

```bash
cargo build --release
maze svg maze.svg -s maze-solved.svg -W 23 -H 30
maze text -W 10 -H 8 -i my-maze
maze pdf my_maze.pdf -S -O L
```

Run `maze` with no arguments to see all options.

How parity is kept:

- `src/random.rs` reimplements CPython's Mersenne Twister, `seed(str)`,
  `randint` and `shuffle`, so the same maze id yields the same maze.
- `src/renderers/pdf.rs` writes the PDF directly (no ReportLab), copying
  ReportLab's number formatting, transform merging and Bezier arcs.
- `tests/snapshot.rs` pins hashes of the text, SVG and PDF output for a
  set of maze ids, so changes can't silently alter what is generated.

Benchmarks for generation, solving and each renderer:

```bash
cargo bench
```

Differences from the Python version:

- The solver uses an explicit stack, so very large mazes (e.g. 168x240)
  can't hit a recursion limit.
- PDF metadata (creator/producer, dates in UTC) differs from ReportLab's.
- Invalid numbers print a one-line error instead of a Python traceback.
