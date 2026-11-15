# Styles

[Back to the README](../README.md)

These options work with `svg` and `pdf`, and with `-I` ([ink](ink.md)) too.
Leave them all out and the output is byte for byte what it was before.

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

A PDF with a solution or a style is drawn the same way up as the SVG, starting
at the top left (`src/renderers/style.rs`). The plain PDF is unchanged.

Through make, pass any of these in `OPTS`:

```bash
make svg OPTS="--corners bevel --frame"
```
