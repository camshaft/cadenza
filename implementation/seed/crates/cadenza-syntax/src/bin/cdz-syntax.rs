//! `cdz-syntax` — convert a Cadenza program between its three surfaces.
//!
//! Reads a program in one format and writes it in another. Formats: `binary`, `sexpr`, `ml`.
//!
//! ```text
//! cdz-syntax --from <fmt> --to <fmt> [FILE]
//! cdz-syntax -f <fmt> -t <fmt> [FILE]
//! ```
//!
//! With no `FILE` (or `-`), input is read from stdin. Output goes to stdout. This bin is the only
//! place in the crate that touches the filesystem/stdio; the conversion itself is the pure
//! `cadenza_syntax::convert` module.

use cadenza_syntax::convert::{self, Format, Options};
use std::io::{Read, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("cdz-syntax: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args = Args::parse(std::env::args().skip(1))?;

    let input = read_input(args.file.as_deref())?;
    let opts = Options { width: args.width.unwrap_or(Options::default().width) };
    let output =
        convert::convert_with(&input, args.from, args.to, opts).map_err(|e| e.to_string())?;

    std::io::stdout().write_all(&output).map_err(|e| format!("writing stdout: {e}"))?;
    // Add a trailing newline after text output so a terminal prompt lands on its own line; binary
    // output is emitted exactly.
    if args.to != Format::Binary {
        let _ = std::io::stdout().write_all(b"\n");
    }
    Ok(())
}

struct Args {
    from: Format,
    to: Format,
    file: Option<String>,
    width: Option<usize>,
}

impl Args {
    fn parse(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
        let mut from: Option<Format> = None;
        let mut to: Option<Format> = None;
        let mut file: Option<String> = None;
        let mut width: Option<usize> = None;
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "-h" | "--help" => return Err(USAGE.to_string()),
                "-f" | "--from" => from = Some(parse_fmt(&next(&mut it, "--from")?)?),
                "-t" | "--to" => to = Some(parse_fmt(&next(&mut it, "--to")?)?),
                "-w" | "--width" => width = Some(parse_width(&next(&mut it, "--width")?)?),
                s if s.starts_with("--from=") => from = Some(parse_fmt(&s["--from=".len()..])?),
                s if s.starts_with("--to=") => to = Some(parse_fmt(&s["--to=".len()..])?),
                s if s.starts_with("--width=") => width = Some(parse_width(&s["--width=".len()..])?),
                s if s.starts_with('-') && s != "-" => {
                    return Err(format!("unknown option `{s}`\n{USAGE}"));
                }
                _ => {
                    if file.is_some() {
                        return Err(format!("unexpected extra argument `{arg}`\n{USAGE}"));
                    }
                    file = Some(arg);
                }
            }
        }
        let from = from.ok_or_else(|| format!("missing --from FORMAT\n{USAGE}"))?;
        let to = to.ok_or_else(|| format!("missing --to FORMAT\n{USAGE}"))?;
        Ok(Args { from, to, file, width })
    }
}

fn next(it: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    it.next().ok_or_else(|| format!("{flag} needs an argument\n{USAGE}"))
}

fn parse_width(s: &str) -> Result<usize, String> {
    s.parse::<usize>()
        .ok()
        .filter(|&w| w > 0)
        .ok_or_else(|| format!("invalid width `{s}` (want a positive integer)"))
}

fn parse_fmt(name: &str) -> Result<Format, String> {
    Format::parse(name).ok_or_else(|| format!("unknown format `{name}` (want binary|sexpr|ml)"))
}

/// Read the whole input from `file` (or stdin when `None` or `"-"`).
fn read_input(file: Option<&str>) -> Result<Vec<u8>, String> {
    match file {
        None | Some("-") => {
            let mut buf = Vec::new();
            std::io::stdin()
                .read_to_end(&mut buf)
                .map_err(|e| format!("reading stdin: {e}"))?;
            Ok(buf)
        }
        Some(path) => std::fs::read(path).map_err(|e| format!("reading {path}: {e}")),
    }
}

const USAGE: &str = "\
usage: cdz-syntax --from <fmt> --to <fmt> [--width N] [FILE]
  convert a Cadenza program between surfaces (reads FILE or stdin, writes stdout)
  formats: binary | sexpr | ml
  --width N   target line width for `ml` output (default 100)
  e.g.  cdz-syntax --from sexpr --to binary prog.sexp > prog.bin
        cat prog.bin | cdz-syntax -f binary -t sexpr
        cdz-syntax -f sexpr -t ml --width 40 prog.sexp";
