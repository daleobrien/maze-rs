# How it works

[Back to the README](../README.md)

- Each kind of tile is drawn once and reused in every cell of that kind
  (`<defs>` and `<use>` in SVG, a Form XObject in PDF). This makes the files
  faster to write and smaller than the Python version's.
- `src/random.rs` is wyrand seeded from a hash of the maze id. It replaced a
  bit-for-bit copy of CPython's Mersenne Twister, which made generation about
  3x slower.
- `src/renderers/pdf.rs` writes the PDF directly (no ReportLab), copying
  ReportLab's number formatting, transform merging and Bezier arcs.
- `tests/snapshot.rs` pins hashes of the text, SVG and PDF output for a set of
  maze ids, so changes can't silently alter what is generated (`make test`).

## Benchmarks

There are benchmarks for generation, solving and each renderer:

```bash
make bench                # all of them
make bench BENCH=render   # just the renderers
```

## Differences from the Python version

maze-rs is a port of the Python generator
[daleobrien/maze](https://github.com/daleobrien/maze), with the same command
line and the same drawing, except:

- Mazes come from a different random number generator, so maze ids don't carry
  over between the two versions.
- The solver uses an explicit stack, so very large mazes (e.g. 168x240) can't
  hit a recursion limit.
- PDF metadata (creator/producer, dates in UTC) differs from ReportLab's.
- Invalid numbers print a one-line error instead of a Python traceback.
- `-I` (ink) is new here.
