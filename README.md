# maze-rs

A fast Rust generator for **weave mazes**: mazes whose passages cross over
and under each other. It writes them as SVG, PDF or plain text, with an
optional solution and an optional hand-drawn look.

It is a port of the Python generator
[daleobrien/maze](https://github.com/daleobrien/maze), with the same command line
and the same drawing.

<p align="center">
  <img src="docs/sample-ink-solved.svg" alt="A hand-drawn weave maze with its solution pencilled in" width="560">
</p>

## Quick start

```bash
cargo build --release
maze svg maze.svg -s maze-solved.svg -W 23 -H 30    # maze and its solution
maze pdf my_maze.pdf -S -O L                        # straight lines, landscape
maze text -W 10 -H 8 -i my-maze                     # to the terminal
```

Run `maze` with no arguments to see all the options. The same maze id
(`-i`) always gives the same maze. The Makefile wraps the common tasks: run
`make` to list them.

## Hand-drawn ink

Add `-I` (`--ink`) to `svg` or `pdf` to draw the maze in brown ink on a
transparent background, with every wall wobbling slightly. The wobble is
seeded from the maze id, so a maze always comes out the same. The start and
end are drawn as in [Lubra](https://github.com/daleobrien/lubra): the corridor
opens into a ring with a red dot inside (`--ring` sets its size). The solved
sample at the top was made with:

```bash
maze svg /dev/null -I -s docs/sample-ink-solved.svg -W 30 -H 21 -i readme
```

No two cells look alike, so ink tiles can't be reused the way plain ones are.
Expect files several times bigger and slower to write
(`src/renderers/ink.rs`). `make ink` writes an ink SVG, its solution and a
PDF in one go, and `INK=1` does the same for `make svg` and `make pdf`.

## Styles

These options work with `svg` and `pdf`, and with `-I` too. Leave them all
out and the output is byte for byte what it was before.

| Option | Values | What it changes |
| --- | --- | --- |
| `--corners` | `round`, `bevel`, `square` | Corner shape. `bevel` cuts every corner at 45°. |
| `--corridor` | 0.2 to 0.9 | Corridor width as a fraction of a cell (normally 0.6). |
| `--dead-ends` | `bulb`, `round`, `flat` | How a corridor that goes nowhere ends. |
| `--bridge-gap` | 0 to 0.15 | A gap in the walls where a passage goes under a bridge. |
| `--line` | width | Wall thickness in px (SVG) or pt (PDF). |
| `--wall-colour`, `--solution-colour`, `--background` | `#rrggbb` | Colours. |
| `-F`, `--frame` | | A frame with START and FINISH arrows and a footer with the maze id and size. |
| `--ring` | 0 to 0.48 | With `-I`, the radius of the start and end rings, as a fraction of a cell (normally 0.46); 0 draws the old dot. |

```bash
maze svg maze.svg -s solved.svg --corners bevel --corridor 0.5 --frame
maze pdf maze.pdf -s solved.pdf --wall-colour '#1d3557' --background '#f1faee'
maze text -W 20 -H 10 -s solved.txt      # solution drawn in heavy lines
```

`-s` now works for `pdf` and `text` as well as `svg`. A PDF with a solution
or a style is drawn the same way up as the SVG, starting at the top left
(`src/renderers/style.rs`). The plain PDF is unchanged. Add `OPTS="..."` to
`make svg`, `make pdf` or `make text` to pass any of these.

## As a library

The generator and the renderers are a library too (`maze`), which builds for
WebAssembly with the default features turned off:

```toml
maze = { git = "https://github.com/daleobrien/maze-rs", default-features = false }
```

The `pdf` feature compresses PDFs (with it off they are written uncompressed)
and `cli` builds the `maze` program; both are on by default. For a page that
draws the maze itself, `ink_svg_cells` inks a maze at a given cell size,
`pencil_svg` pencils any trail of cells over it the way the solution is drawn,
and `moves` gives the cells one move away, past any crossings.

## How it works

- Each kind of tile is drawn once and reused in every cell of that kind
  (`<defs>` and `<use>` in SVG, a Form XObject in PDF). This makes the files
  faster to write and smaller than the Python version's.
- `src/random.rs` is wyrand seeded from a hash of the maze id. It replaced a
  bit-for-bit copy of CPython's Mersenne Twister, which made generation
  about 3x slower.
- `src/renderers/pdf.rs` writes the PDF directly (no ReportLab), copying
  ReportLab's number formatting, transform merging and Bezier arcs.
- `tests/snapshot.rs` pins hashes of the text, SVG and PDF output for a set
  of maze ids, so changes can't silently alter what is generated.

Benchmarks for generation, solving and each renderer:

```bash
cargo bench          # or: make bench BENCH=render
```

## Differences from the Python version

- Mazes come from a different random number generator, so maze ids don't
  carry over between the two versions.
- The solver uses an explicit stack, so very large mazes (e.g. 168x240)
  can't hit a recursion limit.
- PDF metadata (creator/producer, dates in UTC) differs from ReportLab's.
- Invalid numbers print a one-line error instead of a Python traceback.
- `-I` (ink) is new here.
