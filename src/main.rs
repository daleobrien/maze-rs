use std::io;
use std::process::ExitCode;

use maze::random::Rng;
use maze::renderers::ink::{ink_pdf, ink_svg, seed_from_id};
use maze::renderers::pdf::{pdf_render, PdfOptions};
use maze::renderers::style::{styled_pdf, styled_svg, Colour, Corners, DeadEnds, Style};
use maze::renderers::svg::svg_render;
use maze::renderers::text::{text_render, text_render_solved};
use maze::renderers::{PageSize, Solution};
use maze::{create_maze, find_solution};

const DOC: &str = include_str!("usage.txt");

enum Command {
    Pdf { filename: String },
    Text,
    Svg { filename: String },
}

impl Command {
    /// The bit for this command in [`OptionSpec::commands`].
    fn bit(&self) -> u8 {
        match self {
            Command::Pdf { .. } => PDF,
            Command::Text => TEXT,
            Command::Svg { .. } => SVG,
        }
    }
}

/// The options as given on the command line, before any are checked, by
/// their short name (with the value, for those that take one).
#[derive(Default)]
struct Options {
    given: Vec<(char, Option<String>)>,
}

impl Options {
    /// Record one option; each may be given at most once.
    fn apply(&mut self, short: char, value: Option<String>) -> Result<(), Exit> {
        if short == 'h' {
            return Err(Exit::Help);
        }
        if self.given.iter().any(|&(c, _)| c == short) {
            return Err(Exit::Usage);
        }
        self.given.push((short, value));
        Ok(())
    }

    /// The value of an option that takes one, if it was given.
    fn value(&self, short: char) -> Option<&str> {
        self.given.iter().find(|&&(c, _)| c == short).and_then(|(_, value)| value.as_deref())
    }

    /// Whether a flag was given.
    fn flag(&self, short: char) -> bool {
        self.given.iter().any(|&(c, _)| c == short)
    }
}

struct Args {
    command: Command,
    options: Options,
}

/// The commands, as bits for [`OptionSpec::commands`].
const PDF: u8 = 1;
const TEXT: u8 = 2;
const SVG: u8 = 4;
const ALL: u8 = PDF | TEXT | SVG;
/// The commands that draw, rather than print, the maze.
const DRAWN: u8 = PDF | SVG;

struct OptionSpec {
    short: char,
    long: Option<&'static str>,
    takes_value: bool,
    /// The commands whose usage line accepts it.
    commands: u8,
}

const fn flag(short: char, long: Option<&'static str>, commands: u8) -> OptionSpec {
    OptionSpec { short, long, takes_value: false, commands }
}

const fn valued(short: char, long: Option<&'static str>, commands: u8) -> OptionSpec {
    OptionSpec { short, long, takes_value: true, commands }
}

/// Every option, as in the usage text (which leaves out the style options'
/// short names).
const OPTIONS: &[OptionSpec] = &[
    flag('h', Some("help"), ALL),
    valued('W', Some("width"), ALL),
    valued('H', Some("height"), ALL),
    valued('d', Some("density"), ALL),
    flag('L', None, ALL),
    flag('S', None, DRAWN),
    flag('I', Some("ink"), DRAWN),
    valued('c', Some("corners"), DRAWN),
    valued('w', Some("corridor"), DRAWN),
    valued('e', Some("dead-ends"), DRAWN),
    valued('g', Some("bridge-gap"), DRAWN),
    valued('l', Some("line"), DRAWN),
    valued('k', Some("wall-colour"), DRAWN),
    valued('r', Some("solution-colour"), DRAWN),
    valued('b', Some("background"), DRAWN),
    flag('F', Some("frame"), DRAWN),
    valued('p', None, ALL),
    valued('O', Some("orientation"), PDF),
    valued('i', Some("maze_id"), ALL),
    valued('s', Some("solution"), ALL),
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
    let allowed = |short: char| OPTIONS.iter().any(|o| o.short == short && o.commands & command.bit() != 0);
    if positionals.next().is_some() || options.given.iter().any(|&(c, _)| !allowed(c)) {
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
        corners: parse_option("--corners", options.value('c'), "round, bevel or square", |v| match v {
            "round" => Some(Corners::Round),
            "bevel" => Some(Corners::Bevel),
            "square" => Some(Corners::Square),
            _ => None,
        })?,
        dead_ends: parse_option("--dead-ends", options.value('e'), "bulb, round or flat", |v| match v {
            "bulb" => Some(DeadEnds::Bulb),
            "round" => Some(DeadEnds::Round),
            "flat" => Some(DeadEnds::Flat),
            _ => None,
        })?,
        corridor: parse_option("--corridor", options.value('w'), "a number from 0.2 to 0.9", number_in(0.2..=0.9))?,
        bridge_gap: parse_option("--bridge-gap", options.value('g'), "a number from 0 to 0.15", number_in(0.0..=0.15))?,
        line: parse_option("--line", options.value('l'), "a number from 0.1 to 20", number_in(0.1..=20.0))?,
        wall_colour: parse_option("--wall-colour", options.value('k'), colour, Colour::parse)?,
        solution_colour: parse_option("--solution-colour", options.value('r'), colour, Colour::parse)?,
        background: parse_option("--background", options.value('b'), colour, Colour::parse)?,
        frame: options.flag('F').then(|| maze_id.to_owned()),
    })
}

/// Which renderers draw the maze.
#[derive(Clone, Copy, PartialEq)]
enum Look {
    /// The original drawing, as the Python version made it.
    Plain,
    /// With the style options.
    Styled,
    /// By hand, in ink (`-I`).
    Ink,
}

fn run(Args { command, options }: Args) -> Result<(), Exit> {
    let width = parse_int("--width", options.value('W'), "21")?;
    let height = parse_int("--height", options.value('H'), "30")?;
    let density = parse_int("--density", options.value('d'), "50")?;
    let at_least_one = |n: i64| usize::try_from(n).ok().filter(|&n| n >= 1);
    let (Some(width), Some(height)) = (at_least_one(width), at_least_one(height)) else {
        return Err(Exit::TooSmall);
    };

    // allow the same maze to be generated if a maze_id is given.
    let maze_id = options.value('i').map_or_else(|| uuid::Uuid::new_v4().to_string(), str::to_owned);
    let mut rng = Rng::from_str_seed(&maze_id);
    let ink_seed = seed_from_id(&maze_id);

    let page = parse_page(options.value('p').unwrap_or("A4"))?;
    println!("width = {width}, height={height}");

    let grid = create_maze(width, height, density, options.flag('L'), &mut rng);
    let draw_with_curves = !options.flag('S');
    let style = parse_style(&options, &maze_id)?;
    let solution_filename = options.value('s');
    let solve = || find_solution(&grid);
    let look = if options.flag('I') {
        Look::Ink
    } else if style.is_plain() {
        Look::Plain
    } else {
        Look::Styled
    };

    match command {
        Command::Pdf { filename } => {
            let landscape = options.value('O') == Some("L");
            let pdf_options = PdfOptions { draw_with_curves, page, landscape };
            // the plain PDF is drawn upside down from the SVG, with other
            // start and end cells, so a solution needs the styled renderer
            // for the maze too
            let look = if look == Look::Plain && solution_filename.is_some() { Look::Styled } else { look };
            let render = |solution: Option<Solution>| match look {
                Look::Plain => pdf_render(&grid, &pdf_options),
                Look::Styled => styled_pdf(&grid, &pdf_options, solution, &style),
                Look::Ink => ink_pdf(&grid, &pdf_options, solution, ink_seed, &style),
            };
            if let Some(solution_filename) = solution_filename {
                write_file(solution_filename, render(solve().as_deref()))?;
            }
            write_file(&filename, render(None))
        }
        // to screen
        Command::Text => {
            print!("{}", text_render(&grid));
            if let Some(solution_filename) = solution_filename {
                write_file(solution_filename, text_render_solved(&grid, &solve().unwrap_or_default()))?;
            }
            Ok(())
        }
        Command::Svg { filename } => {
            let render = |solution: Option<Solution>| match look {
                Look::Plain => svg_render(&grid, page, draw_with_curves, solution),
                Look::Styled => styled_svg(&grid, page, draw_with_curves, solution, &style),
                Look::Ink => ink_svg(&grid, page, draw_with_curves, solution, ink_seed, &style),
            };
            let svg = render(None);
            if let Some(solution_filename) = solution_filename {
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
