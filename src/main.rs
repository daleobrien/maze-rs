use std::process::exit;

use maze::maze::{create_maze, find_solution};
use maze::random::PyRandom;
use maze::renderers::pdf::{pdf_render, PdfOptions};
use maze::renderers::svg::svg_render;
use maze::renderers::text::text_render;
use maze::renderers::PageSize;

const DOC: &str = include_str!("usage.txt");

#[derive(Clone, Copy, PartialEq)]
enum Command {
    Pdf,
    Text,
    Svg,
}

#[derive(Default)]
struct Args {
    command: Option<Command>,
    filename: Option<String>,
    width: Option<String>,
    height: Option<String>,
    density: Option<String>,
    add_a_loop: bool,
    straight: bool,
    page: Option<String>,
    orientation: Option<String>,
    maze_id: Option<String>,
    solution: Option<String>,
}

/// (short, long, takes a value)
const OPTIONS: &[(char, Option<&str>, bool)] = &[
    ('h', Some("help"), false),
    ('W', Some("width"), true),
    ('H', Some("height"), true),
    ('d', Some("density"), true),
    ('L', None, false),
    ('S', None, false),
    ('p', None, true),
    ('O', Some("orientation"), true),
    ('i', Some("maze_id"), true),
    ('s', Some("solution"), true),
];

/// Print the usage section and exit, as docopt does on bad arguments.
fn usage_error() -> ! {
    let start = DOC.find("Usage:").unwrap();
    let section = &DOC[start..];
    let end = section.find("\n\n").unwrap_or(section.len());
    eprintln!("{}", &section[..end]);
    exit(1);
}

fn set(slot: &mut Option<String>, value: String) {
    if slot.is_some() {
        usage_error();
    }
    *slot = Some(value);
}

fn apply(args: &mut Args, short: char, value: Option<String>) {
    match short {
        'h' => {
            println!("{}", DOC.trim_matches('\n'));
            exit(0);
        }
        'W' => set(&mut args.width, value.unwrap()),
        'H' => set(&mut args.height, value.unwrap()),
        'd' => set(&mut args.density, value.unwrap()),
        'p' => set(&mut args.page, value.unwrap()),
        'O' => set(&mut args.orientation, value.unwrap()),
        'i' => set(&mut args.maze_id, value.unwrap()),
        's' => set(&mut args.solution, value.unwrap()),
        'L' if !args.add_a_loop => args.add_a_loop = true,
        'S' if !args.straight => args.straight = true,
        _ => usage_error(),
    }
}

fn parse_args(argv: &[String]) -> Args {
    let mut args = Args::default();
    let mut positionals: Vec<String> = Vec::new();
    let mut seen: Vec<char> = Vec::new();

    let mut it = argv.iter().cloned();
    while let Some(token) = it.next() {
        if token == "--" {
            positionals.extend(it.by_ref());
        } else if let Some(long) = token.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (long, None),
            };
            // exact match, otherwise a unique prefix (as docopt allows)
            let candidates: Vec<_> = OPTIONS.iter().filter(|o| o.1.is_some_and(|l| l.starts_with(name))).collect();
            let opt = OPTIONS
                .iter()
                .find(|o| o.1 == Some(name))
                .or(if candidates.len() == 1 { Some(candidates[0]) } else { None })
                .unwrap_or_else(|| usage_error());
            let value = match (opt.2, inline) {
                (true, Some(v)) => Some(v),
                (true, None) => Some(it.next().unwrap_or_else(|| usage_error())),
                (false, Some(_)) => usage_error(),
                (false, None) => None,
            };
            seen.push(opt.0);
            apply(&mut args, opt.0, value);
        } else if token.len() > 1 && token.starts_with('-') {
            let chars: Vec<char> = token[1..].chars().collect();
            let mut k = 0;
            while k < chars.len() {
                let opt = OPTIONS.iter().find(|o| o.0 == chars[k]).unwrap_or_else(|| usage_error());
                let value = if opt.2 {
                    let rest: String = chars[k + 1..].iter().collect();
                    k = chars.len();
                    Some(if rest.is_empty() { it.next().unwrap_or_else(|| usage_error()) } else { rest })
                } else {
                    k += 1;
                    None
                };
                seen.push(opt.0);
                apply(&mut args, opt.0, value);
            }
        } else {
            positionals.push(token);
        }
    }

    let mut positionals = positionals.into_iter();
    args.command = match positionals.next().as_deref() {
        Some("pdf") => Some(Command::Pdf),
        Some("text") => Some(Command::Text),
        Some("svg") => Some(Command::Svg),
        _ => usage_error(),
    };
    let command = args.command.unwrap();
    if command != Command::Text {
        args.filename = Some(positionals.next().unwrap_or_else(|| usage_error()));
    }
    if positionals.next().is_some() {
        usage_error();
    }

    // options each command's usage line accepts
    let allowed: &[char] = match command {
        Command::Pdf => &['W', 'H', 'p', 'd', 'S', 'L', 'O', 'i'],
        Command::Text => &['W', 'H', 'p', 'd', 'L', 'i'],
        Command::Svg => &['W', 'H', 'p', 'd', 'S', 'L', 'i', 's'],
    };
    if seen.iter().any(|c| !allowed.contains(c)) {
        usage_error();
    }

    args
}

fn parse_int(name: &str, value: &str) -> i64 {
    value.trim().parse().unwrap_or_else(|_| {
        eprintln!("ValueError: invalid literal for int() with base 10: '{value}' ({name})");
        exit(1);
    })
}

fn write_file(path: &str, contents: &[u8]) {
    if let Err(e) = std::fs::write(path, contents) {
        eprintln!("Could not write {path}: {e}");
        exit(1);
    }
}

fn run(args: Args) {
    let width = parse_int("--width", args.width.as_deref().unwrap_or("21"));
    let height = parse_int("--height", args.height.as_deref().unwrap_or("30"));
    let density = parse_int("--density", args.density.as_deref().unwrap_or("50"));
    if width < 1 || height < 1 {
        eprintln!("Width and height must be at least 1");
        exit(1);
    }
    let (width, height) = (width as usize, height as usize);

    // allow the same maze to be generated if a maze_id is given.
    let maze_id = args.maze_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let mut rng = PyRandom::from_str_seed(&maze_id);

    let page_arg = args.page.as_deref().unwrap_or("A4");
    if !["a4", "letter", "a0"].contains(&page_arg.to_lowercase().as_str()) {
        println!("Page size can only be A4, Letter, or A0");
        println!("e.g.  -p A4");
        println!("You had -p {page_arg}");
        exit(-1);
    }

    // As in the Python version, only the exact upper case spellings select
    // A4/A0; anything else (e.g. "a4") falls through to Letter.
    let page = match page_arg {
        "A4" => PageSize::A4,
        "A0" => PageSize::A0,
        _ => PageSize::Letter,
    };
    println!("width = {width}, height={height}");

    let grid = create_maze(width, height, density, args.add_a_loop, &mut rng);

    let solution = args.solution.as_ref().and_then(|_| find_solution(&grid));
    let draw_with_curves = !args.straight;

    match args.command.unwrap() {
        Command::Pdf => {
            let options = PdfOptions {
                draw_with_curves,
                page,
                landscape: args.orientation.as_deref() == Some("L"),
                width,
                height,
            };
            write_file(args.filename.as_deref().unwrap(), &pdf_render(&grid, &options));
        }
        // to screen
        Command::Text => print!("{}", text_render(&grid)),
        Command::Svg => {
            let svg = svg_render(&grid, width, height, page, draw_with_curves, None);
            if let Some(solution_filename) = &args.solution {
                let solved = svg_render(&grid, width, height, page, draw_with_curves, solution.as_deref());
                write_file(solution_filename, solved.as_bytes());
            }
            write_file(args.filename.as_deref().unwrap(), svg.as_bytes());
        }
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        println!("{DOC}");
        exit(0);
    }
    run(parse_args(&argv));
}
