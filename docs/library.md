# As a library

[Back to the README](../README.md)

The generator and the renderers are a library too (`maze`), which builds for
WebAssembly with the default features turned off:

```toml
maze = { git = "https://github.com/daleobrien/maze-rs", default-features = false }
```

| Feature | What it adds | Default |
| ------- | ------------ | ------- |
| `pdf` | Compressed PDFs (with it off they are written uncompressed) | on |
| `cli` | The `maze` program | on |

For a page that draws the maze itself:

- `ink_svg_cells` inks a maze at a given cell size;
- `pencil_svg` pencils any trail of cells over it, the way the solution is
  drawn;
- `moves` gives the cells one move away, past any crossings.

[maze.do.id.au](https://maze.do.id.au) is built this way.
