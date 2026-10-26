use std::io;
use std::process::ExitCode;

use maze::random::PyRandom;
use maze::renderers::pdf::{pdf_render, PdfOptions};
use maze::renderers::svg::svg_render;
use maze::renderers::text::text_render;
use maze::renderers::PageSize;
use maze::{create_maze, find_solution};

const DOC: &str = include_str!("usage.txt");

enum Command {
    Pdf { filename: String },
    Text,
    Svg { filename: String },
}

impl Command {
    /// The options this command's usage line accepts.
    fn allowed_options(&self) -> &'static [char] {
        match self {
            Command::Pdf { .. } => &['W', 'H', 'p', 'd', 'S', 'L', 'O', 'i'],
            Command::Text => &['W', 'H', 'p', 'd', 'L', 'i'],
            Command::Svg { .. } => &['W', 'H', 'p', 'd', 'S', 'L', 'i', 's'],
        }
    }
}

/// The options as given on the command line, before any are checked.
#[derive(Default)]
struct Options {
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

impl Options {
    /// Record one option; each may be given at most once.
    fn apply(&mut self, short: char, value: Option<String>) -> Result<(), Exit> {
        let (slot, value) = match (short, value) {
            ('h', _) => return Err(Exit::Help),
            ('L', None) => return set_flag(&mut self.add_a_loop),
            ('S', None) => return set_flag(&mut self.straight),
            ('W', Some(value)) => (&mut self.width, value),
            ('H', Some(value)) => (&mut self.height, value),
            ('d', Some(value)) => (&mut self.density, value),
            ('p', Some(value)) => (&mut self.page, value),
            ('O', Some(value)) => (&mut self.orientation, value),
            ('i', Some(value)) => (&mut self.maze_id, value),
            ('s', Some(value)) => (&mut self.solution, value),
            _ => return Err(Exit::Usage),
        };
        if slot.is_some() {
            return Err(Exit::Usage);
        }
        *slot = Some(value);
        Ok(())
    }
}

fn set_flag(flag: &mut bool) -> Result<(), Exit> {
    if *flag {
        return Err(Exit::Usage);
    }
    *flag = true;
    Ok(())
}

struct Args {
    command: Command,
    options: Options,
}

struct OptionSpec {
    short: char,
    long: Option<&'static str>,
    takes_value: bool,
}

const fn flag(short: char, long: Option<&'static str>) -> OptionSpec {
    OptionSpec { short, long, takes_value: false }
}

const fn valued(short: char, long: Option<&'static str>) -> OptionSpec {
    OptionSpec { short, long, takes_value: true }
}

const OPTIONS: &[OptionSpec] = &[
    flag('h', Some("help")),
    valued('W', Some("width")),
    valued('H', Some("height")),
    valued('d', Some("density")),
    flag('L', None),
    flag('S', None),
    valued('p', None),
    valued('O', Some("orientation")),
    valued('i', Some("maze_id")),
    valued('s', Some("solution")),
];

/// Why the program stopped early.
enum Exit {
    /// `-h`/`--help`: show the full usage text.
    Help,
    /// Arguments docopt would reject: show the usage section.
    Usage,
    InvalidInt {
        name: &'static str,
        value: String,
    },
    TooSmall,
    InvalidPage(String),
    Write {
        path: String,
        error: io::Error,
    },
}

impl Exit {
    /// Tell the user what happened, as the Python version does.
    fn report(self) -> ExitCode {
        match self {
            Exit::Help => {
                println!("{}", DOC.trim_matches('\n'));
                return ExitCode::SUCCESS;
            }
            Exit::Usage => {
                let start = DOC.find("Usage:").expect("the usage text has a Usage section");
                let section = &DOC[start..];
                let end = section.find("\n\n").unwrap_or(section.len());
                eprintln!("{}", &section[..end]);
            }
            Exit::InvalidInt { name, value } => {
                eprintln!("ValueError: invalid literal for int() with base 10: '{value}' ({name})");
            }
            Exit::TooSmall => eprintln!("Width and height must be at least 1"),
            Exit::InvalidPage(page) => {
                println!("Page size can only be A4, Letter, or A0");
                println!("e.g.  -p A4");
                println!("You had -p {page}");
                // Python's exit(-1)
                return ExitCode::from(255);
            }
            Exit::Write { path, error } => eprintln!("Could not write {path}: {error}"),
        }
        ExitCode::FAILURE
    }
}

/// Find a long option by its name, or else by a unique prefix of it (as
/// docopt allows).
fn find_long(name: &str) -> Result<&'static OptionSpec, Exit> {
    if let Some(spec) = OPTIONS.iter().find(|o| o.long == Some(name)) {
        return Ok(spec);
    }
    let mut candidates = OPTIONS.iter().filter(|o| o.long.is_some_and(|l| l.starts_with(name)));
    match (candidates.next(), candidates.next()) {
        (Some(spec), None) => Ok(spec),
        _ => Err(Exit::Usage),
    }
}

fn find_short(short: char) -> Result<&'static OptionSpec, Exit> {
    OPTIONS.iter().find(|o| o.short == short).ok_or(Exit::Usage)
}

fn parse_args(argv: &[String]) -> Result<Args, Exit> {
    let mut options = Options::default();
    let mut positionals: Vec<String> = Vec::new();
    let mut seen: Vec<char> = Vec::new();

    let mut tokens = argv.iter().cloned();
    while let Some(token) = tokens.next() {
        if token == "--" {
            positionals.extend(tokens.by_ref());
        } else if let Some(long) = token.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((name, value)) => (name, Some(value.to_owned())),
                None => (long, None),
            };
            let spec = find_long(name)?;
            let value = match (spec.takes_value, inline) {
                (true, Some(value)) => Some(value),
                (true, None) => Some(tokens.next().ok_or(Exit::Usage)?),
                (false, Some(_)) => return Err(Exit::Usage),
                (false, None) => None,
            };
            seen.push(spec.short);
            options.apply(spec.short, value)?;
        } else if let Some(mut shorts) = token.strip_prefix('-').filter(|s| !s.is_empty()) {
            // a cluster of short options, the last of which may take a value
            // (either the rest of the token or the next one)
            while let Some(short) = shorts.chars().next() {
                shorts = &shorts[short.len_utf8()..];
                let spec = find_short(short)?;
                let value = if spec.takes_value {
                    let value = if shorts.is_empty() { tokens.next().ok_or(Exit::Usage)? } else { shorts.to_owned() };
                    shorts = "";
                    Some(value)
                } else {
                    None
                };
                seen.push(spec.short);
                options.apply(spec.short, value)?;
            }
        } else {
            positionals.push(token);
        }
    }

    let mut positionals = positionals.into_iter();
    let command = match positionals.next().as_deref() {
        Some("pdf") => Command::Pdf { filename: positionals.next().ok_or(Exit::Usage)? },
        Some("text") => Command::Text,
        Some("svg") => Command::Svg { filename: positionals.next().ok_or(Exit::Usage)? },
        _ => return Err(Exit::Usage),
    };
    if positionals.next().is_some() || seen.iter().any(|c| !command.allowed_options().contains(c)) {
        return Err(Exit::Usage);
    }

    Ok(Args { command, options })
}

fn parse_int(name: &'static str, value: Option<&str>, default: &str) -> Result<i64, Exit> {
    let value = value.unwrap_or(default);
    value.trim().parse().map_err(|_| Exit::InvalidInt { name, value: value.to_owned() })
}

fn parse_page(page: &str) -> Result<PageSize, Exit> {
    if !["a4", "letter", "a0"].contains(&page.to_lowercase().as_str()) {
        return Err(Exit::InvalidPage(page.to_owned()));
    }
    // As in the Python version, only the exact upper case spellings select
    // A4/A0; anything else (e.g. "a4") falls through to Letter.
    Ok(match page {
        "A4" => PageSize::A4,
        "A0" => PageSize::A0,
        _ => PageSize::Letter,
    })
}

fn write_file(path: &str, contents: impl AsRef<[u8]>) -> Result<(), Exit> {
    std::fs::write(path, contents).map_err(|error| Exit::Write { path: path.to_owned(), error })
}

fn run(Args { command, options }: Args) -> Result<(), Exit> {
    let width = parse_int("--width", options.width.as_deref(), "21")?;
    let height = parse_int("--height", options.height.as_deref(), "30")?;
    let density = parse_int("--density", options.density.as_deref(), "50")?;
    let at_least_one = |n: i64| usize::try_from(n).ok().filter(|&n| n >= 1);
    let (Some(width), Some(height)) = (at_least_one(width), at_least_one(height)) else {
        return Err(Exit::TooSmall);
    };

    // allow the same maze to be generated if a maze_id is given.
    let maze_id = options.maze_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let mut rng = PyRandom::from_str_seed(&maze_id);

    let page = parse_page(options.page.as_deref().unwrap_or("A4"))?;
    println!("width = {width}, height={height}");

    let grid = create_maze(width, height, density, options.add_a_loop, &mut rng);
    let draw_with_curves = !options.straight;

    match command {
        Command::Pdf { filename } => {
            let landscape = options.orientation.as_deref() == Some("L");
            write_file(&filename, pdf_render(&grid, &PdfOptions { draw_with_curves, page, landscape }))
        }
        // to screen
        Command::Text => {
            print!("{}", text_render(&grid));
            Ok(())
        }
        Command::Svg { filename } => {
            let svg = svg_render(&grid, page, draw_with_curves, None);
            if let Some(solution_filename) = &options.solution {
                let solution = find_solution(&grid);
                write_file(solution_filename, svg_render(&grid, page, draw_with_curves, solution.as_deref()))?;
            }
            write_file(&filename, svg)
        }
    }
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        println!("{DOC}");
        return ExitCode::SUCCESS;
    }
    match parse_args(&argv).and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(exit) => exit.report(),
    }
}
