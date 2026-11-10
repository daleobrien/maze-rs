//! An implementation of a "weave" maze generator. Weave mazes are those
//! with passages that pass both over and under other passages. The
//! technique used here was described by Robin Houston, and works by first
//! decorating the blank grid with the over/under crossings, and then using
//! Kruskal's algorithm to fill out the rest of the grid. (Kruskal's is very
//! well-suited to this approach, since it treats the cells as separate sets
//! and joins them together.)

use std::ops::{Index, IndexMut};

use crate::random::Rng;
use crate::simd::push_row_edges;

// constants to aid with describing the passage directions
pub const N: u8 = 0x1;
pub const S: u8 = 0x2;
pub const E: u8 = 0x4;
pub const W: u8 = 0x8;
pub const U: u8 = 0x10;

/// A crossing where the N/S passage goes over the E/W one (19).
pub const CROSS_NS: u8 = N | S | U;
/// A crossing where the E/W passage goes over the N/S one (28).
pub const CROSS_EW: u8 = E | W | U;

/// Whether a cell is an over/under crossing.
#[must_use]
pub fn is_crossing(cell: u8) -> bool {
    cell == CROSS_NS || cell == CROSS_EW
}

/// The maze: the passages out of each cell, as a set of direction flags,
/// stored row by row. Cells are indexed by `(x, y)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grid {
    cells: Vec<u8>,
    width: usize,
}

impl Grid {
    /// A grid with no passages.
    ///
    /// # Panics
    ///
    /// If `width` or `height` is zero.
    #[must_use]
    pub fn new(width: usize, height: usize) -> Self {
        assert!(width > 0 && height > 0, "a maze needs at least one cell");
        Grid { cells: vec![0; width * height], width }
    }

    #[must_use]
    pub fn width(&self) -> usize {
        self.width
    }

    #[must_use]
    pub fn height(&self) -> usize {
        self.cells.len() / self.width
    }

    /// The cells of row `y`, from west to east.
    #[must_use]
    pub fn row(&self, y: usize) -> &[u8] {
        &self.cells[y * self.width..(y + 1) * self.width]
    }

    /// The rows from north to south.
    pub fn rows(&self) -> impl ExactSizeIterator<Item = &[u8]> {
        self.cells.chunks_exact(self.width)
    }
}

impl Index<(usize, usize)> for Grid {
    type Output = u8;

    fn index(&self, (x, y): (usize, usize)) -> &u8 {
        &self.cells[y * self.width + x]
    }
}

impl IndexMut<(usize, usize)> for Grid {
    fn index_mut(&mut self, (x, y): (usize, usize)) -> &mut u8 {
        &mut self.cells[y * self.width + x]
    }
}

/// Disjoint sets of cells; only "are these connected?" matters to the
/// algorithm, so a plain union-find is equivalent to the Python `Tree` class.
///
/// Cells are numbered row by row (`y * width + x`), and parents are `u32`
/// so the table stays small enough to sit in cache for large mazes. Sets are
/// joined by rank, which keeps the trees shallow; how they are joined can't
/// change the maze, since only connectedness is ever asked.
struct Sets {
    parent: Vec<u32>,
    rank: Vec<u8>,
    width: usize,
}

impl Sets {
    fn new(width: usize, height: usize) -> Self {
        let cells = u32::try_from(width * height).expect("a maze has at most u32::MAX cells");
        Sets { parent: (0..cells).collect(), rank: vec![0; cells as usize], width }
    }

    fn root_of(&mut self, mut i: u32) -> u32 {
        while self.parent[i as usize] != i {
            let grandparent = self.parent[self.parent[i as usize] as usize];
            self.parent[i as usize] = grandparent;
            i = grandparent;
        }
        i
    }

    fn root(&mut self, (x, y): (usize, usize)) -> u32 {
        self.root_of((y * self.width + x) as u32)
    }

    /// Connect cells `a` and `b` (by number), returning false if they
    /// already were.
    fn join(&mut self, a: u32, b: u32) -> bool {
        let (ra, rb) = (self.root_of(a), self.root_of(b));
        if ra == rb {
            return false;
        }
        self.link(ra, rb);
        true
    }

    /// Join the sets with (different) roots `ra` and `rb`.
    fn link(&mut self, ra: u32, rb: u32) {
        // which way round is random, so this is written to compile to
        // conditional moves rather than branches
        let (rank_a, rank_b) = (self.rank[ra as usize], self.rank[rb as usize]);
        let (child, root) = if rank_a < rank_b { (ra, rb) } else { (rb, ra) };
        self.parent[child as usize] = root;
        self.rank[root as usize] += u8::from(rank_a == rank_b);
    }
}

/// Generate a `width` x `height` weave maze.
///
/// `density` is the percentage chance (0-100) that a cell is *not*
/// considered for an over/under crossing, and `add_a_loop` turns one
/// crossing into a plain four-way junction. The same seed gives the same
/// maze.
///
/// # Panics
///
/// If `width` or `height` is zero.
pub fn create_maze(width: usize, height: usize, density: i64, add_a_loop: bool, rng: &mut Rng) -> Grid {
    // structures to hold the maze
    let mut grid = Grid::new(width, height);
    let mut sets = Sets::new(width, height);

    // pick the cells to try for an over/under crossing; written without a
    // branch on the (random) outcome, since a mispredicted branch per cell
    // costs more than the rest of the work here
    let mut candidates = vec![(0usize, 0usize); width.saturating_sub(2) * height.saturating_sub(2)];
    let mut picked = 0;
    for cy in 1..height.saturating_sub(1) {
        for cx in 1..width.saturating_sub(1) {
            candidates[picked] = (cx, cy);
            picked += usize::from(rng.below(100) as i64 >= density);
        }
    }
    candidates.truncate(picked);

    // build the over/under locations
    for (cx, cy) in candidates {
        let north = (cx, cy - 1);
        let west = (cx - 1, cy);
        let east = (cx + 1, cy);
        let south = (cx, cy + 1);

        if grid[(cx, cy)] != 0 {
            continue;
        }
        let (root_n, root_s) = (sets.root(north), sets.root(south));
        if root_n == root_s {
            continue;
        }
        let (root_e, root_w) = (sets.root(east), sets.root(west));
        if root_e == root_w {
            continue;
        }

        // the east/west roots may have moved under the joined north/south
        // set, but are at most a step from their new root
        sets.link(root_n, root_s);
        sets.join(root_e, root_w);

        grid[(cx, cy)] = if rng.coin() { CROSS_EW } else { CROSS_NS };

        grid[north] |= S;
        grid[west] |= E;
        grid[east] |= W;
        grid[south] |= N;
    }

    // build the list of the remaining edges, i.e. those not touching a
    // crossing; each one joins a cell to its north or west neighbour, and is
    // packed into a u32 as `cell * 2 + is_west` (cells numbered row by row)
    // so that shuffling and walking the list stays in cache for large mazes
    let open = |cell: u8| usize::from((cell != CROSS_NS) & (cell != CROSS_EW));
    // room for the vector version to store whole groups past the last edge
    let mut edges = vec![0u32; 2 * width * height + 4];
    let mut kept = 0;
    for x in 1..width {
        // the top row has no crossings, so only edges to the west
        edges[kept] = (x * 2 + 1) as u32;
        kept += 1;
    }
    for y in 1..height {
        let (above, row) = (grid.row(y - 1), grid.row(y));
        let first = y * width;
        // the west column has no crossings either, so its north edge is kept
        // unless the cell above it is one
        edges[kept] = (first * 2) as u32;
        kept += open(above[0]);
        kept = push_row_edges(above, row, first, &mut edges, kept);
    }
    edges.truncate(kept);

    rng.shuffle(&mut edges);

    // Kruskal's algorithm
    for e in edges {
        let cell = e / 2;
        let (direction, neighbour, back) = if e & 1 == 0 { (N, cell - width as u32, S) } else { (W, cell - 1, E) };

        if sets.join(cell, neighbour) {
            grid.cells[cell as usize] |= direction;
            grid.cells[neighbour as usize] |= back;
        }
    }

    // add in a loop, I just replace a under/over with a cross
    if add_a_loop {
        // find all the crossings, if any
        let candidates: Vec<(usize, usize)> = (1..height.saturating_sub(1))
            .flat_map(|cy| (1..width.saturating_sub(1)).map(move |cx| (cx, cy)))
            .filter(|&cell| is_crossing(grid[cell]))
            .collect();

        // change just one of them to a crossing, e.g. create a loop
        if !candidates.is_empty() {
            grid[candidates[rng.below(candidates.len() as u64) as usize]] = N | S | W | E;
        }
    }

    grid
}

/// The moves out of a cell, in the order they are tried.
#[derive(Default)]
struct Moves {
    items: [(isize, isize); 4],
    len: usize,
    next: usize,
}

impl Moves {
    fn push(&mut self, mv: (isize, isize)) {
        self.items[self.len] = mv;
        self.len += 1;
    }
}

impl Iterator for Moves {
    type Item = (isize, isize);

    fn next(&mut self) -> Option<Self::Item> {
        let mv = self.items[..self.len].get(self.next).copied();
        self.next += 1;
        mv
    }
}

/// How far a move from `from` in steps of `step` goes: one cell, plus any
/// crossings it passes over or under (there can be several in a row).
fn jump(cell_at: impl Fn(isize) -> u8, from: isize, step: isize) -> isize {
    let mut inc = step;
    while is_crossing(cell_at(from + inc)) {
        inc += step;
    }
    inc
}

/// For a given position, return all the possible coordinate moves.
fn get_moves(grid: &Grid, (x, y): (isize, isize)) -> Moves {
    let mut moves = Moves::default();
    // crossings are never on the edge of the maze, so jumps stay inside it
    let row = grid.row(y as usize);
    let in_row = |x: isize| row[x as usize];
    let in_column = |y: isize| grid[(x as usize, y as usize)];

    let cell = in_row(x);
    if cell & W == W {
        moves.push((jump(in_row, x, -1), 0));
    }
    if cell & E == E {
        moves.push((jump(in_row, x, 1), 0));
    }
    if cell & N == N {
        moves.push((0, jump(in_column, y, -1)));
    }
    if cell & S == S {
        moves.push((0, jump(in_column, y, 1)));
    }
    moves
}

/// Depth-first search from the top left to the bottom right corner,
/// returning the cells along the way as `(x, y)`.
///
/// Explores moves in the same order as the recursive Python version and so
/// returns the same path, but uses an explicit stack so very large mazes
/// can't overflow the call stack.
#[must_use]
pub fn find_solution(grid: &Grid) -> Option<Vec<(isize, isize)>> {
    let width = grid.width();
    let end = (width as isize - 1, grid.height() as isize - 1);
    let index = |(x, y): (isize, isize)| y as usize * width + x as usize;

    let mut path = vec![(0, 0)];
    let mut on_path = vec![false; width * grid.height()];
    on_path[0] = true;

    // the moves out of each cell on `path` that are still to be tried
    let mut frames = vec![get_moves(grid, (0, 0))];

    while let Some(moves) = frames.last_mut() {
        let Some(mv) = moves.next() else {
            frames.pop();
            let cell = path.pop().expect("one path cell per frame");
            on_path[index(cell)] = false;
            continue;
        };

        let (x, y) = *path.last().expect("one path cell per frame");
        let next = (x + mv.0, y + mv.1);

        // if are back to where we where, then are in a loop
        if on_path[index(next)] {
            continue;
        }

        path.push(next);

        // are we at the end already?
        if next == end {
            return Some(path);
        }

        on_path[index(next)] = true;
        frames.push(get_moves(grid, next));
    }
    None
}
