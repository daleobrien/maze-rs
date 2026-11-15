# The maze command

[Back to the README](../README.md)

`maze` writes a maze as SVG, PDF or plain text, with an optional solution:

```bash
maze svg maze.svg -s maze-solved.svg -W 23 -H 30    # maze and its solution
maze pdf my_maze.pdf -S -O L                        # straight lines, landscape
maze text -W 10 -H 8 -i my-maze                     # to the terminal
```

Run `maze` with no arguments to see all the options. `-W` and `-H` are the
size in cells. The same maze id (`-i`) always gives the same maze; without one,
every run gives a new maze. `-s` writes the solution as well, for `svg`, `pdf`
and `text`.

## Through make

`make install` puts `maze` on your PATH (`cargo install --path .`). Without
installing, the Makefile runs it for you:

| Task | Writes |
| ---- | ------ |
| `make svg` | `out/maze.svg` and its solution, `out/maze-solved.svg` |
| `make pdf` | `out/maze.pdf` |
| `make text` | a maze in the terminal |
| `make ink` | an ink SVG, its solution and a PDF of one maze, `out/maze-ink.*` |

Each takes these settings:

| Setting | Meaning | Default |
| ------- | ------- | ------- |
| `W`, `H` | The size in cells | 21 by 30 |
| `ID` | The maze id | a new maze each time |
| `INK=1` | Draw `make svg` and `make pdf` by hand, in [ink](ink.md) | off |
| `OPTS` | Any other options, e.g. a [style](styles.md) | none |
| `OUT` | The folder to write to | `out` |

```bash
make svg W=30 H=20 ID=hello
make pdf INK=1 OPTS="--frame -s out/solved.pdf"
```
