use std::io;
use std::process::ExitCode;

use maze::random::Rng;
use maze::renderers::ink::{ink_pdf, ink_svg, seed_from_id};
use maze::renderers::pdf::{pdf_render, PdfOptions};
use maze::renderers::style::{styled_pdf, styled_svg, Colour, Corners, DeadEnds, Style};
use maze::renderers::svg::svg_render;
use maze::renderers::text::{text_render, text_render_solved};
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
            Command::Pdf { .. } => {
                &['W', 'H', 'p', 'd', 'S', 'L', 'O', 'i', 's', 'I', 'c', 'w', 'e', 'g', 'l', 'k', 'r', 'b', 'F']
            }
            Command::Text => &['W', 'H', 'p', 'd', 'L', 'i', 's'],
            Command::Svg { .. } => {
                &['W', 'H', 'p', 'd', 'S', 'L', 'i', 's', 'I', 'c', 'w', 'e', 'g', 'l', 'k', 'r', 'b', 'F']
            }
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
    ink: bool,
    frame: bool,
    corners: Option<String>,
    corridor: Option<String>,
    dead_ends: Option<String>,
    bridge_gap: Option<String>,
    line: Option<String>,
    wall_colour: Option<String>,
    solution_colour: Option<String>,
    background: Option<String>,
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
            ('I', None) => return set_flag(&mut self.ink),
            ('F', None) => return set_flag(&mut self.frame),
            ('c', Some(value)) => (&mut self.corners, value),
            ('w', Some(value)) => (&mut self.corridor, value),
            ('e', Some(value)) => (&mut self.dead_ends, value),
            ('g', Some(value)) => (&mut self.bridge_gap, value),
            ('l', Some(value)) => (&mut self.line, value),
            ('k', Some(value)) => (&mut self.wall_colour, value),
            ('r', Some(value)) => (&mut self.solution_colour, value),
            ('b', Some(value)) => (&mut self.background, value),
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
    flag('I', Some("ink")),
    valued('c', Some("corners")),
    valued('w', Some("corridor")),
    valued('e', Some("dead-ends")),
    valued('g', Some("bridge-gap")),
    valued('l', Some("line")),
    valued('k', Some("wall-colour")),
    valued('r', Some("solution-colour")),
    valued('b', Some("background")),
    flag('F', Some("frame")),
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
    /// A style option with a value it doesn't accept.
    InvalidOption {
        name: &'static str,
        value: String,
        expected: &'static str,
    },
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
            Exit::InvalidOption { name, value, expected } => {
                eprintln!("{name} can't be '{value}': expected {expected}")
            }
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

/// Read an option's value with `parse`, or report what it should have been.
fn parse_option<T>(
    name: &'static str,
    value: Option<&str>,
    expected: &'static str,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<Option<T>, Exit> {
    value.map(|v| parse(v).ok_or_else(|| Exit::InvalidOption { name, value: v.to_owned(), expected })).transpose()
}

/// A number in `range`.
fn number_in(range: std::ops::RangeInclusive<f64>) -> impl Fn(&str) -> Option<f64> {
    move |v| v.trim().parse::<f64>().ok().filter(|x| range.contains(x))
}

/// The style options, checked.
fn parse_style(options: &Options, maze_id: &str) -> Result<Style, Exit> {
    let colour = "a colour like #e51919 or #333";
    Ok(Style {
        corners: parse_option("--corners", options.corners.as_deref(), "round, bevel or square", |v| match v {
            "round" => Some(Corners::Round),
            "bevel" => Some(Corners::Bevel),
            "square" => Some(Corners::Square),
            _ => None,
        })?,
        dead_ends: parse_option("--dead-ends", options.dead_ends.as_deref(), "bulb, round or flat", |v| match v {
            "bulb" => Some(DeadEnds::Bulb),
            "round" => Some(DeadEnds::Round),
            "flat" => Some(DeadEnds::Flat),
            _ => None,
        })?,
        corridor: parse_option(
            "--corridor",
            options.corridor.as_deref(),
            "a number from 0.2 to 0.9",
            number_in(0.2..=0.9),
        )?,
        bridge_gap: parse_option(
            "--bridge-gap",
            options.bridge_gap.as_deref(),
            "a number from 0 to 0.15",
            number_in(0.0..=0.15),
        )?,
        line: parse_option("--line", options.line.as_deref(), "a number from 0.1 to 20", number_in(0.1..=20.0))?,
        wall_colour: parse_option("--wall-colour", options.wall_colour.as_deref(), colour, Colour::parse)?,
        solution_colour: parse_option("--solution-colour", options.solution_colour.as_deref(), colour, Colour::parse)?,
        background: parse_option("--background", options.background.as_deref(), colour, Colour::parse)?,
        frame: options.frame.then(|| maze_id.to_owned()),
    })
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
    let maze_id = options.maze_id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let mut rng = Rng::from_str_seed(&maze_id);
    let ink_seed = seed_from_id(&maze_id);

    let page = parse_page(options.page.as_deref().unwrap_or("A4"))?;
    println!("width = {width}, height={height}");

    let grid = create_maze(width, height, density, options.add_a_loop, &mut rng);
    let draw_with_curves = !options.straight;
    let style = parse_style(&options, &maze_id)?;
    let solve = || find_solution(&grid);

    match command {
        Command::Pdf { filename } => {
            let landscape = options.orientation.as_deref() == Some("L");
            let pdf_options = PdfOptions { draw_with_curves, page, landscape };
            // the plain PDF is drawn upside down from the SVG, with other
            // start and end cells, so a solution needs the styled renderer
            // for the maze too
            let render = |solution: Option<&[(isize, isize)]>| {
                if options.ink {
                    ink_pdf(&grid, &pdf_options, solution, ink_seed, &style)
                } else if style.is_plain() && options.solution.is_none() {
                    pdf_render(&grid, &pdf_options)
                } else {
                    styled_pdf(&grid, &pdf_options, solution, &style)
                }
            };
            if let Some(solution_filename) = &options.solution {
                write_file(solution_filename, render(solve().as_deref()))?;
            }
            write_file(&filename, render(None))
        }
        // to screen
        Command::Text => {
            print!("{}", text_render(&grid));
            if let Some(solution_filename) = &options.solution {
                write_file(solution_filename, text_render_solved(&grid, &solve().unwrap_or_default()))?;
            }
            Ok(())
        }
        Command::Svg { filename } => {
            let render = |solution: Option<&[(isize, isize)]>| {
                if options.ink {
                    ink_svg(&grid, page, draw_with_curves, solution, ink_seed, &style)
                } else if style.is_plain() {
                    svg_render(&grid, page, draw_with_curves, solution)
                } else {
                    styled_svg(&grid, page, draw_with_curves, solution, &style)
                }
            };
            let svg = render(None);
            if let Some(solution_filename) = &options.solution {
                write_file(solution_filename, render(solve().as_deref()))?;
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
