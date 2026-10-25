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

/// Render the maze with box drawing characters, exactly as printed by the
/// Python version.
pub fn text_render(grid: &Grid) -> String {
    let width = grid.first().map_or(0, |r| r.len());
    let mut out = String::new();

    out.push_str("Start");
    out.push_str(&" ".repeat(width * 2));
    out.push('\n');

    for (z, row) in grid.iter().enumerate() {
        out.extend(row.iter().map(|&r| tile(r).0));
        out.push('\n');
        out.extend(row.iter().map(|&r| tile(r).1));

        // bottom right corner ?
        if z == grid.len() - 1 {
            out.push_str("End");
        }
        out.push('\n');
    }

    out.push('\n');
    out
}
