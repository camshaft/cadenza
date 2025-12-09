//! REPL (Read-Eval-Print Loop) for Cadenza.
//!
//! Provides an interactive environment with:
//! - Command history (saved to ~/.cadenza_history)
//! - Syntax highlighting
//! - Auto-completion for identifiers
//! - Option to load files into scope

use anyhow::Result;
use cadenza_eval::{Compiler, Env, Value};
use cadenza_syntax::{lexer::Lexer, parse::parse, token::Kind};
use rustyline::{
    Context, Editor, Helper,
    completion::{Completer, Pair},
    error::ReadlineError,
    highlight::{CmdKind, Highlighter},
    hint::Hinter,
    validate::Validator,
};
use std::{
    borrow::Cow,
    io::{BufRead, Write},
    path::PathBuf,
};

/// REPL helper that provides completion and syntax highlighting
struct CadenzaHelper;

impl CadenzaHelper {
    fn new() -> Self {
        Self
    }
}

impl Helper for CadenzaHelper {}

impl Completer for CadenzaHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        // Find the start of the current word
        let start = line[..pos]
            .rfind(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
            .map(|i| i + 1)
            .unwrap_or(0);

        let word = &line[start..pos];
        if word.is_empty() {
            return Ok((pos, Vec::new()));
        }

        // Collect matching identifiers from the environment
        let mut candidates = Vec::new();

        // Get identifiers from the environment
        // Note: Env doesn't expose a way to iterate over bindings,
        // so we'll provide a basic set of built-in names
        let builtins = [
            "let", "fn", "=", "match", "assert", "typeof", "measure", "+", "-", "*", "/", "==",
            "!=", "<", "<=", ">", ">=", "|>",
        ];

        for builtin in &builtins {
            if builtin.starts_with(word) {
                candidates.push(Pair {
                    display: builtin.to_string(),
                    replacement: builtin.to_string(),
                });
            }
        }

        Ok((start, candidates))
    }
}

impl Hinter for CadenzaHelper {
    type Hint = String;

    fn hint(&self, _line: &str, _pos: usize, _ctx: &Context<'_>) -> Option<String> {
        None
    }
}

impl Highlighter for CadenzaHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        // Use the cadenza lexer to tokenize the line
        let tokens: Vec<_> = Lexer::new(line).collect();

        // Build a colored version of the line
        let mut result = String::new();
        let mut last_end = 0;

        for token in tokens {
            let span = token.span;
            let start = span.start;
            let end = span.end;

            // Add any whitespace/text between tokens
            if start > last_end {
                result.push_str(&line[last_end..start]);
            }

            // Add the token with color based on its kind
            let text = &line[start..end];
            let colored = match token.kind {
                Kind::Integer | Kind::Float => format!("\x1b[33m{}\x1b[0m", text), // Yellow
                Kind::StringStart
                | Kind::StringContent
                | Kind::StringContentWithEscape
                | Kind::StringEnd => {
                    format!("\x1b[32m{}\x1b[0m", text) // Green
                }
                Kind::Identifier => {
                    // Check if it's a builtin keyword
                    if matches!(
                        text,
                        "let" | "fn" | "match" | "assert" | "typeof" | "measure"
                    ) {
                        format!("\x1b[35m{}\x1b[0m", text) // Magenta
                    } else {
                        text.to_string()
                    }
                }
                Kind::CommentStart
                | Kind::CommentContent
                | Kind::DocCommentStart
                | Kind::DocCommentContent => {
                    format!("\x1b[90m{}\x1b[0m", text) // Gray
                }
                _ => text.to_string(),
            };
            result.push_str(&colored);
            last_end = end;
        }

        // Add any remaining text
        if last_end < line.len() {
            result.push_str(&line[last_end..]);
        }

        Cow::Owned(result)
    }

    fn highlight_char(&self, _line: &str, _pos: usize, _kind: CmdKind) -> bool {
        true
    }
}

impl Validator for CadenzaHelper {}

/// Run a REPL session with generic input/output streams.
///
/// This function provides the core REPL logic that can be used for both
/// interactive sessions (with rustyline) and for testing (with in-memory streams).
///
/// # Arguments
/// * `input` - A buffered reader to read input lines from
/// * `output` - A writer to write output to
/// * `error` - A writer to write errors to
/// * `load_file` - Optional file to load into the environment before starting
///
/// Returns `Ok(())` on success, or an error if the REPL fails to initialize.
#[allow(dead_code)]
pub fn run_repl<R: BufRead, W: Write, E: Write>(
    mut input: R,
    mut output: W,
    mut error: E,
    load_file: Option<PathBuf>,
) -> Result<()> {
    writeln!(output, "Cadenza REPL v{}", env!("CARGO_PKG_VERSION"))?;
    writeln!(
        output,
        "Type expressions to evaluate. Press Ctrl+D or Ctrl+C to exit."
    )?;
    writeln!(output)?;

    // Initialize environment and compiler
    let mut env = Env::with_standard_builtins();
    let mut compiler = Compiler::new();

    // Load file if specified
    if let Some(path) = load_file {
        writeln!(output, "Loading {}...", path.display())?;
        let source = std::fs::read_to_string(&path)?;
        let parsed = parse(&source);

        if !parsed.errors.is_empty() {
            writeln!(error, "Parse errors in {}:", path.display())?;
            for err in &parsed.errors {
                writeln!(error, "  {:?}", err)?;
            }
            return Err(anyhow::anyhow!("Failed to parse {}", path.display()));
        }

        cadenza_eval::eval(&parsed.ast(), &mut env, &mut compiler);

        if compiler.has_errors() {
            writeln!(error, "Evaluation errors in {}:", path.display())?;
            for diagnostic in compiler.diagnostics() {
                writeln!(error, "  {}", diagnostic)?;
            }
            return Err(anyhow::anyhow!("Failed to evaluate {}", path.display()));
        }

        writeln!(output, "Loaded successfully.\n")?;
    }

    // REPL loop - read lines from input
    let mut line = String::new();
    loop {
        line.clear();
        let bytes_read = input.read_line(&mut line)?;

        // EOF reached
        if bytes_read == 0 {
            writeln!(output, "^D")?;
            break;
        }

        let line_trimmed = line.trim();
        if line_trimmed.is_empty() {
            continue;
        }

        // Parse and evaluate
        let parsed = parse(line_trimmed);

        if !parsed.errors.is_empty() {
            writeln!(error, "Parse errors:")?;
            for err in &parsed.errors {
                writeln!(error, "  {:?}", err)?;
            }
            continue;
        }

        let results = cadenza_eval::eval(&parsed.ast(), &mut env, &mut compiler);

        if compiler.has_errors() {
            writeln!(error, "Evaluation errors:")?;
            for diagnostic in compiler.diagnostics() {
                writeln!(error, "  {}", diagnostic)?;
            }
            compiler.clear_diagnostics();
            continue;
        }

        // Print results
        for (i, result) in results.iter().enumerate() {
            if results.len() > 1 {
                writeln!(output, "[{}] {}", i, format_value(result))?;
            } else {
                writeln!(output, "{}", format_value(result))?;
            }
        }
    }

    Ok(())
}

/// Start the Cadenza REPL with interactive readline support
pub fn start_repl(load_file: Option<PathBuf>) -> Result<()> {
    println!("Cadenza REPL v{}", env!("CARGO_PKG_VERSION"));
    println!("Type expressions to evaluate. Press Ctrl+D or Ctrl+C to exit.");
    println!();

    // Initialize environment and compiler
    let mut env = Env::with_standard_builtins();
    let mut compiler = Compiler::new();

    // Load file if specified
    if let Some(path) = load_file {
        println!("Loading {}...", path.display());
        let source = std::fs::read_to_string(&path)?;
        let parsed = parse(&source);

        if !parsed.errors.is_empty() {
            eprintln!("Parse errors in {}:", path.display());
            for error in &parsed.errors {
                eprintln!("  {:?}", error);
            }
            return Err(anyhow::anyhow!("Failed to parse {}", path.display()));
        }

        cadenza_eval::eval(&parsed.ast(), &mut env, &mut compiler);

        if compiler.has_errors() {
            eprintln!("Evaluation errors in {}:", path.display());
            for diagnostic in compiler.diagnostics() {
                eprintln!("  {}", diagnostic);
            }
            return Err(anyhow::anyhow!("Failed to evaluate {}", path.display()));
        }

        println!("Loaded successfully.\n");
    }

    // Create readline editor with helper
    let helper = CadenzaHelper::new();
    let mut rl = Editor::new()?;
    rl.set_helper(Some(helper));

    // Load history
    let history_path = dirs::home_dir()
        .map(|mut p| {
            p.push(".cadenza_history");
            p
        })
        .unwrap_or_else(|| PathBuf::from(".cadenza_history"));

    let _ = rl.load_history(&history_path);

    // REPL loop
    loop {
        match rl.readline("cadenza> ") {
            Ok(line) => {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }

                // Add to history
                let _ = rl.add_history_entry(line);

                // Parse and evaluate
                let parsed = parse(line);

                if !parsed.errors.is_empty() {
                    eprintln!("Parse errors:");
                    for error in &parsed.errors {
                        eprintln!("  {:?}", error);
                    }
                    continue;
                }

                let results = cadenza_eval::eval(&parsed.ast(), &mut env, &mut compiler);

                if compiler.has_errors() {
                    eprintln!("Evaluation errors:");
                    for diagnostic in compiler.diagnostics() {
                        eprintln!("  {}", diagnostic);
                    }
                    compiler.clear_diagnostics();
                    continue;
                }

                // Print results
                for (i, result) in results.iter().enumerate() {
                    if results.len() > 1 {
                        println!("[{}] {}", i, format_value(result));
                    } else {
                        println!("{}", format_value(result));
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("^C");
                break;
            }
            Err(ReadlineError::Eof) => {
                println!("^D");
                break;
            }
            Err(err) => {
                eprintln!("Error: {:?}", err);
                break;
            }
        }
    }

    // Save history
    let _ = rl.save_history(&history_path);

    Ok(())
}

/// Format a value for display in the REPL
fn format_value(value: &Value) -> String {
    match value {
        Value::Nil => "nil".to_string(),
        Value::Integer(n) => n.to_string(),
        Value::Float(f) => f.to_string(),
        Value::String(s) => {
            // Properly escape string contents
            let escaped = s
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
                .replace('\t', "\\t");
            format!("\"{}\"", escaped)
        }
        Value::Bool(b) => b.to_string(),
        Value::Symbol(s) => format!(":{}", s),
        Value::List(items) => {
            let items_str = items
                .iter()
                .map(format_value)
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{}]", items_str)
        }
        Value::Tuple { type_name, elements } => {
            let (type_name, tn_space) = if let Some(name) = type_name {
                (&name[..], " ")
            } else {
                ("", "")
            };
            let elements_str = elements
                .iter()
                .map(format_value)
                .collect::<Vec<_>>()
                .join(", ");
            // Single-element tuples need trailing comma
            let trailing_comma = if elements.len() == 1 { "," } else { "" };
            format!("{type_name}{tn_space}({}{trailing_comma})", elements_str)
        }
        Value::Record { type_name, fields } => {
            let (type_name, tn_space) = if let Some(name) = type_name {
                (&name[..], " ")
            } else {
                ("", "")
            };
            let fields_str = fields
                .iter()
                .map(|(k, v)| format!("{}: {}", k, format_value(v)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{type_name}{tn_space}{{{}}}", fields_str)
        }
        Value::StructConstructor { name, field_types } => {
            let fields_str = field_types
                .iter()
                .map(|(k, v)| format!("{}: {}", k, v))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{} {{{}}}", name, fields_str)
        }
        Value::UserFunction(f) => format!("<function {}>", f.name),
        Value::BuiltinFn(f) => format!("<builtin {}>", f.name),
        Value::BuiltinMacro(_) => "<macro>".to_string(),
        Value::SpecialForm(_) => "<special-form>".to_string(),
        Value::Type(t) => format!("<type {}>", t),
        Value::Quantity { value, unit, .. } => format!("{} {:?}", value, unit),
        Value::UnitConstructor(unit) => format!("<unit {:?}>", unit),
    }
}
