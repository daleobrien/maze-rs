//! An implementation of a "weave" maze generator. Weave mazes are those
//! with passages that pass both over and under other passages. The
//! technique used here was described by Robin Houston, and works by first
//! decorating the blank grid with the over/under crossings, and then using
//! Kruskal's algorithm to fill out the rest of the grid. (Kruskal's is very
//! well-suited to this approach, since it treats the cells as separate sets
//! and joins them together.)

use std::ops::{Index, IndexMut};

use crate::random::PyRandom;

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
struct Sets {
    parent: Vec<usize>,
    width: usize,
}

impl Sets {
    fn new(width: usize, height: usize) -> Self {
        Sets { parent: (0..width * height).collect(), width }
    }

    fn root(&mut self, (x, y): (usize, usize)) -> usize {
        let mut i = y * self.width + x;
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]];
            i = self.parent[i];
        }
        i
    }

    fn connected(&mut self, a: (usize, usize), b: (usize, usize)) -> bool {
        self.root(a) == self.root(b)
    }

    fn connect(&mut self, a: (usize, usize), b: (usize, usize)) {
        let (ra, rb) = (self.root(a), self.root(b));
        self.parent[rb] = ra;
    }
}

/// Generate a `width` x `height` weave maze.
///
/// `density` is the percentage chance (0-100) that a cell is *not*
/// considered for an over/under crossing, and `add_a_loop` turns one
/// crossing into a plain four-way junction. Draws from `rng` exactly as the
/// Python version does, so the same seed gives the same maze.
///
/// # Panics
///
/// If `width` or `height` is zero.
pub fn create_maze(width: usize, height: usize, density: i64, add_a_loop: bool, rng: &mut PyRandom) -> Grid {
    // structures to hold the maze
    let mut grid = Grid::new(width, height);
    let mut sets = Sets::new(width, height);

    // build the list of edges; each one joins a cell to its north or west
    // neighbour
    let mut edges: Vec<(usize, usize, u8)> = Vec::with_capacity(2 * width * height);
    for y in 0..height {
        for x in 0..width {
            if y > 0 {
                edges.push((x, y, N));
            }
            if x > 0 {
                edges.push((x, y, W));
            }
        }
    }

    rng.shuffle(&mut edges);

    // each cell has at most one N and one W edge
    let edge_index = |x: usize, y: usize, d: u8| (y * width + x) * 2 + usize::from(d == W);
    let mut removed = vec![false; width * height * 2];

    // build the over/under locations
    for cy in 1..height.saturating_sub(1) {
        for cx in 1..width.saturating_sub(1) {
            if rng.randint(0, 99) < density {
                continue;
            }

            let north = (cx, cy - 1);
            let west = (cx - 1, cy);
            let east = (cx + 1, cy);
            let south = (cx, cy + 1);

            if grid[(cx, cy)] != 0 || sets.connected(north, south) || sets.connected(east, west) {
                continue;
            }

            sets.connect(north, south);
            sets.connect(east, west);

            grid[(cx, cy)] = if rng.randint(0, 1) == 0 { CROSS_EW } else { CROSS_NS };

            grid[north] |= S;
            grid[west] |= E;
            grid[east] |= W;
            grid[south] |= N;

            // the edges into the crossing are no longer available; they are
            // dropped in one pass below, which keeps the order of the rest
            removed[edge_index(cx, cy, N)] = true;
            removed[edge_index(cx, cy, W)] = true;
            removed[edge_index(east.0, east.1, W)] = true;
            removed[edge_index(south.0, south.1, N)] = true;
        }
    }

    edges.retain(|&(x, y, d)| !removed[edge_index(x, y, d)]);

    // Kruskal's algorithm
    while let Some((x, y, direction)) = edges.pop() {
        let (neighbour, back) = match direction {
            N => ((x, y - 1), S),
            W => ((x - 1, y), E),
            _ => unreachable!("edges only point north or west"),
        };

        if !sets.connected((x, y), neighbour) {
            sets.connect((x, y), neighbour);
            grid[(x, y)] |= direction;
            grid[neighbour] |= back;
        }
    }

    // add in a loop, I just replace a under/over with a cross
    if add_a_loop {
        // find all the crossings, if any
        let mut candidates: Vec<(usize, usize)> = (1..height.saturating_sub(1))
            .flat_map(|cy| (1..width.saturating_sub(1)).map(move |cx| (cx, cy)))
            .filter(|&cell| is_crossing(grid[cell]))
            .collect();

        // change just one of them to a crossing, e.g. create a loop
        if !candidates.is_empty() {
            rng.shuffle(&mut candidates);
            grid[candidates[0]] = N | S | W | E;
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
