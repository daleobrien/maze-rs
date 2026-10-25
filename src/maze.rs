//! An implementation of a "weave" maze generator. Weave mazes are those
//! with passages that pass both over and under other passages. The
//! technique used here was described by Robin Houston, and works by first
//! decorating the blank grid with the over/under crossings, and then using
//! Kruskal's algorithm to fill out the rest of the grid. (Kruskal's is very
//! well-suited to this approach, since it treats the cells as separate sets
//! and joins them together.)

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

pub type Grid = Vec<Vec<u8>>;

fn dx(direction: u8) -> isize {
    match direction {
        E => 1,
        W => -1,
        _ => 0,
    }
}

fn dy(direction: u8) -> isize {
    match direction {
        N => -1,
        S => 1,
        _ => 0,
    }
}

fn opposite(direction: u8) -> u8 {
    match direction {
        E => W,
        W => E,
        N => S,
        S => N,
        _ => unreachable!(),
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

    fn root(&mut self, mut i: usize) -> usize {
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]];
            i = self.parent[i];
        }
        i
    }

    fn connected(&mut self, a: (usize, usize), b: (usize, usize)) -> bool {
        let (ia, ib) = (a.1 * self.width + a.0, b.1 * self.width + b.0);
        self.root(ia) == self.root(ib)
    }

    fn connect(&mut self, a: (usize, usize), b: (usize, usize)) {
        let (ia, ib) = (a.1 * self.width + a.0, b.1 * self.width + b.0);
        let (ra, rb) = (self.root(ia), self.root(ib));
        self.parent[rb] = ra;
    }
}

pub fn create_maze(width: usize, height: usize, density: i64, add_a_loop: bool, rng: &mut PyRandom) -> Grid {
    // structures to hold the maze
    let mut grid: Grid = vec![vec![0u8; width]; height];
    let mut sets = Sets::new(width, height);

    // build the list of edges
    let mut edges: Vec<(usize, usize, u8)> = Vec::new();
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

            let (nx, ny) = (cx, cy - 1);
            let (wx, wy) = (cx - 1, cy);
            let (ex, ey) = (cx + 1, cy);
            let (sx, sy) = (cx, cy + 1);

            if grid[cy][cx] != 0 || sets.connected((nx, ny), (sx, sy)) || sets.connected((ex, ey), (wx, wy)) {
                continue;
            }

            sets.connect((nx, ny), (sx, sy));
            sets.connect((ex, ey), (wx, wy));

            if rng.randint(0, 1) == 0 {
                grid[cy][cx] = E | W | U;
            } else {
                grid[cy][cx] = N | S | U;
            }

            grid[ny][nx] |= S;
            grid[wy][wx] |= E;
            grid[ey][ex] |= W;
            grid[sy][sx] |= N;

            // the edges into the crossing are no longer available; they are
            // dropped in one pass below, which keeps the order of the rest
            removed[edge_index(cx, cy, N)] = true;
            removed[edge_index(cx, cy, W)] = true;
            removed[edge_index(ex, ey, W)] = true;
            removed[edge_index(sx, sy, N)] = true;
        }
    }

    edges.retain(|&(x, y, d)| !removed[edge_index(x, y, d)]);

    // Kruskal's algorithm
    while let Some((x, y, direction)) = edges.pop() {
        let nx = (x as isize + dx(direction)) as usize;
        let ny = (y as isize + dy(direction)) as usize;

        if !sets.connected((x, y), (nx, ny)) {
            sets.connect((x, y), (nx, ny));
            grid[y][x] |= direction;
            grid[ny][nx] |= opposite(direction);
        }
    }

    // add in a loop, I just replace a under/over with a cross
    if add_a_loop {
        // find all the crossings, if any
        let mut candidates: Vec<(usize, usize)> = Vec::new();
        for cy in 1..height.saturating_sub(1) {
            for cx in 1..width.saturating_sub(1) {
                if grid[cy][cx] == CROSS_NS || grid[cy][cx] == CROSS_EW {
                    candidates.push((cy, cx));
                }
            }
        }

        // change just one of them to a crossing, e.g. create a loop
        if !candidates.is_empty() {
            rng.shuffle(&mut candidates);
            let (cy, cx) = candidates[0];
            grid[cy][cx] = N | S | W | E;
        }
    }

    grid
}

/// Index like Python does, where -1 means the last element.
fn py_index<T>(v: &[T], i: isize) -> &T {
    if i < 0 {
        &v[(v.len() as isize + i) as usize]
    } else {
        &v[i as usize]
    }
}

fn is_crossing(cell: u8) -> bool {
    cell == CROSS_NS || cell == CROSS_EW
}

/// Up to four moves out of a cell, in the order they are tried.
#[derive(Default)]
struct Moves {
    moves: [(isize, isize); 4],
    len: usize,
}

impl Moves {
    fn push(&mut self, mv: (isize, isize)) {
        self.moves[self.len] = mv;
        self.len += 1;
    }
}

/// For a given position, return all the possible coordinate moves.
fn get_moves(grid: &Grid, coordinate: (isize, isize), cell: u8) -> Moves {
    let mut moves = Moves::default();
    let (x, y) = coordinate;
    let row = &grid[y as usize];
    let col = |yy: isize| py_index(&grid[..], yy)[x as usize];

    if cell & W == W {
        let mut inc = -1;
        // if the next cell is an under or over cell, jump over it
        while is_crossing(*py_index(row, x + inc)) {
            // could be more than one in a row
            inc -= 1;
        }
        moves.push((inc, 0));
    }
    if cell & E == E {
        let mut inc = 1;
        while is_crossing(*py_index(row, x + inc)) {
            inc += 1;
        }
        moves.push((inc, 0));
    }
    if cell & N == N {
        let mut inc = -1;
        while is_crossing(col(y + inc)) {
            inc -= 1;
        }
        moves.push((0, inc));
    }
    if cell & S == S {
        let mut inc = 1;
        while is_crossing(col(y + inc)) {
            inc += 1;
        }
        moves.push((0, inc));
    }
    moves
}

/// Depth-first search from the top left to the bottom right corner.
///
/// Explores moves in the same order as the recursive Python version and so
/// returns the same path, but uses an explicit stack so very large mazes
/// can't overflow the call stack.
pub fn find_solution(grid: &Grid) -> Option<Vec<(isize, isize)>> {
    let height = grid.len();
    let width = grid.first().map_or(0, |r| r.len());
    let end = (width as isize - 1, height as isize - 1);

    let mut path: Vec<(isize, isize)> = vec![(0, 0)];
    let mut on_path = vec![vec![false; width]; height];
    on_path[0][0] = true;

    // one frame per element of `path`: the moves out of it and how many have been tried
    let mut frames: Vec<(Moves, usize)> = vec![(get_moves(grid, (0, 0), grid[0][0]), 0)];

    while let Some((moves, next)) = frames.last_mut() {
        let coordinate = *path.last().unwrap();
        if *next >= moves.len {
            frames.pop();
            let (x, y) = path.pop().unwrap();
            on_path[y as usize][x as usize] = false;
            continue;
        }
        let mv = moves.moves[*next];
        *next += 1;

        let new_coordinate = (coordinate.0 + mv.0, coordinate.1 + mv.1);
        let (nx, ny) = (new_coordinate.0 as usize, new_coordinate.1 as usize);

        // if are back to where we where, then are in a loop
        if on_path[ny][nx] {
            continue;
        }

        path.push(new_coordinate);

        // are we at the end already?
        if new_coordinate == end {
            return Some(path);
        }

        on_path[ny][nx] = true;
        frames.push((get_moves(grid, new_coordinate, grid[ny][nx]), 0));
    }
    None
}
