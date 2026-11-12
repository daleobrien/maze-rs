use crate::maze::Grid;

fn tile(cell: u8) -> (&'static str, &'static str) {
    match cell {
        0 => ("   ", "   "),
        1 => ("│ │", "└─┘"),
        2 => ("┌─┐", "│ │"),
        3 => ("│ │", "│ │"),
        4 => ("┌──", "└──"),
        5 => ("│ └", "└──"),
        6 => ("┌──", "│ ┌"),
        7 => ("│ └", "│ ┌"),
        8 => ("──┐", "──┘"),
        9 => ("┘ │", "──┘"),
        10 => ("──┐", "┐ │"),
        11 => ("┘ │", "┐ │"),
        12 => ("───", "───"),
        13 => ("┘ └", "───"),
        14 => ("───", "┐ ┌"),
        15 => ("┘ └", "┐ ┌"),
        19 => ("┤ ├", "┤ ├"),
        28 => ("┴─┴", "┬─┬"),
        _ => panic!("no text tile for cell {cell}"),
    }
}

/// The longest a tile's half can be in bytes (three 3-byte box characters).
const MAX_HALF: usize = 9;

/// Each tile's top and bottom halves as UTF-8, padded to 16 bytes so a half
/// is copied with one 16-byte store whatever its length, and their lengths
/// (0 for a cell with no tile).
struct Halves {
    bytes: [[[u8; 16]; 2]; 32],
    lens: [[u8; 2]; 32],
}

impl Halves {
    fn new() -> Self {
        let mut halves = Halves { bytes: [[[0; 16]; 2]; 32], lens: [[0; 2]; 32] };
        for cell in [0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 19, 28] {
            let (top, bottom) = tile(cell);
            for (k, half) in [top, bottom].into_iter().enumerate() {
                halves.bytes[usize::from(cell)][k][..half.len()].copy_from_slice(half.as_bytes());
                halves.lens[usize::from(cell)][k] = half.len() as u8;
            }
        }
        halves
    }

    /// Append half `k` (0 for the top, 1 for the bottom) of each cell's tile.
    fn push_row(&self, out: &mut Vec<u8>, row: &[u8], k: usize) {
        // every half is written as 16 bytes, of which the first len are kept
        out.reserve(row.len() * MAX_HALF + 16);
        let mut len = out.len();
        for &cell in row {
            let cell = usize::from(cell);
            let half_len = usize::from(self.lens[cell][k]);
            if half_len == 0 {
                tile(cell as u8);
            }
            // SAFETY: at most MAX_HALF bytes are kept per cell, so the 16
            // bytes written at `len` stay inside the reserved capacity
            unsafe { out.as_mut_ptr().add(len).cast::<[u8; 16]>().write_unaligned(self.bytes[cell][k]) };
            len += half_len;
        }
        // SAFETY: the bytes up to `len` were all written above
        unsafe { out.set_len(len) };
    }
}

/// Render the maze with box drawing characters, exactly as printed by the
/// Python version.
#[must_use]
pub fn text_render(grid: &Grid) -> String {
    let halves = Halves::new();
    let mut out = Vec::with_capacity(grid.height() * (2 * grid.width() * MAX_HALF + 2) + 2 * grid.width() + 32);

    out.extend_from_slice(b"Start");
    out.resize(out.len() + grid.width() * 2, b' ');
    out.push(b'\n');

    for (z, row) in grid.rows().enumerate() {
        halves.push_row(&mut out, row, 0);
        out.push(b'\n');
        halves.push_row(&mut out, row, 1);

        // bottom right corner ?
        if z == grid.height() - 1 {
            out.extend_from_slice(b"End");
        }
        out.push(b'\n');
    }

    out.push(b'\n');
    // checking would take longer than the rendering, as almost every
    // character is multi-byte
    debug_assert!(std::str::from_utf8(&out).is_ok());
    // SAFETY: only ASCII and whole tile strings (each valid UTF-8) were
    // written, so the bytes are valid UTF-8
    unsafe { String::from_utf8_unchecked(out) }
}

/// The heavy box drawing character for a light one.
fn heavy(c: char) -> char {
    match c {
        '│' => '┃',
        '─' => '━',
        '┌' => '┏',
        '┐' => '┓',
        '└' => '┗',
        '┘' => '┛',
        '├' => '┣',
        '┤' => '┫',
        '┬' => '┳',
        '┴' => '┻',
        c => c,
    }
}

/// Render the maze as [`text_render`] does, with the cells on `solution`
/// drawn in heavy lines.
#[must_use]
pub fn text_render_solved(grid: &Grid, solution: &[(isize, isize)]) -> String {
    let mut on_path = vec![false; grid.width() * grid.height()];
    let mut mark = |(x, y): (isize, isize)| {
        if let (Ok(x), Ok(y)) = (usize::try_from(x), usize::try_from(y)) {
            if x < grid.width() && y < grid.height() {
                on_path[y * grid.width() + x] = true;
            }
        }
    };
    for pair in solution.windows(2) {
        let (p, q) = (pair[0], pair[1]);
        mark(p);
        mark(q);
        // a jump of two cells passes through a crossing
        mark(((p.0 + q.0).div_euclid(2), (p.1 + q.1).div_euclid(2)));
    }
    if let [only] = solution {
        mark(*only);
    }

    let plain = text_render(grid);
    let mut out = String::with_capacity(plain.len() + plain.len() / 4);
    for (n, line) in plain.split_inclusive('\n').enumerate() {
        // line 0 is "Start", then each row of cells takes two lines
        let row = n.checked_sub(1).map(|n| n / 2).filter(|&y| y < grid.height());
        for (k, c) in line.chars().enumerate() {
            let cell = row.map(|y| (y, k / 3)).filter(|&(_, x)| x < grid.width());
            out.push(match cell {
                Some((y, x)) if on_path[y * grid.width() + x] => heavy(c),
                _ => c,
            });
        }
    }
    out
}
