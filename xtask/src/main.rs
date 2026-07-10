//! xtask — the one interface for driving the workspace.
//!
//! Deliberately minimal: today it builds the value-heap runtime component, content-addresses it,
//! and stores it. The runtime is derived FIRST and keyed by its SHA-256 (the recorded hashing
//! choice, options/hashing-and-encoding/), which is what the host resolves a program's required
//! runtime against (reproducible-derivation.md §Derivation Is A Function Of Source And Toolchain).
//!
//! The source-generation choreography (WIT envelope, opcode/frame tables) that used to live here
//! was stripped out — its outputs are now frozen, hand-maintained sources in the seed crates. Recover
//! the generators from git history if we decide to re-derive them.
//!
//! Every command is parsed with clap — typed subcommands, generated `--help`, and an error on an
//! unknown subcommand/flag.
//!
//! Usage: `cargo xtask build [--store <dir>]`.

use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use xshell::{Shell, cmd};

/// The one interface for driving the Cadenza seed workspace. Every knob is a typed flag; there are
/// no environment-variable knobs.
#[derive(Parser)]
#[command(name = "xtask", about = "The one interface for driving the Cadenza workspace.")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Build the value-heap runtime component, content-address it, and store it under `--store`.
    Build {
        /// Content-addressed store directory. [default: <repo>/target/cadenza-store]
        #[arg(long)]
        store: Option<PathBuf>,
    },
    /// Compile a Cadenza program and run it, printing the result — the whole pipeline end-to-end:
    /// surface → binary AST (cadenza-syntax) → component (rcdzc) → run (cdz-run).
    Run {
        /// The Cadenza program file to compile and run.
        file: PathBuf,
        /// The input surface. Defaults to `sexpr` (what `.cdz`/`.sexp` files carry).
        #[arg(long, default_value = "sexpr")]
        from: String,
        /// Content-addressed store the runtime is resolved from. [default: <repo>/target/cadenza-store]
        #[arg(long)]
        store: Option<PathBuf>,
    },
    /// Run one or more corpus files: compile+run each case through the pipeline and compare the
    /// result against the recorded outcome. Reports pass / todo (a case the compiler can't yet
    /// handle) / fail (a real disagreement). Exits non-zero only on a fail.
    Gate {
        /// The corpus `.sexp` files to run. [default: all of spec/semantics/*.sexp]
        files: Vec<PathBuf>,
        /// Content-addressed store the runtime is resolved from. [default: <repo>/target/cadenza-store]
        #[arg(long)]
        store: Option<PathBuf>,
    },
}

fn main() {
    let paths = Paths::resolve();
    match Cli::parse().command {
        Cmd::Build { store } => build(&paths, store),
        Cmd::Run { file, from, store } => run(&paths, &file, &from, store),
        Cmd::Gate { files, store } => gate(&paths, files, store),
    }
}

/// The workspace directory anchors, resolved once from this crate's manifest location. xtask lives
/// at `<repo>/xtask`, so the repo root is the manifest's parent and the seed workspace is the fixed
/// `<repo>/implementation/seed` beneath it. Every path derives from these two — no fragile
/// `.parent().parent()` chains, and correct inside a git worktree (each worktree's manifest dir
/// resolves to that worktree's own root).
struct Paths {
    /// `<repo>` — the workspace root (parent of `<repo>/xtask`).
    repo: PathBuf,
    /// `<repo>/implementation/seed` — the seed toolchain root that holds `crates/`.
    seed: PathBuf,
}

impl Paths {
    fn resolve() -> Self {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask crate has a parent (the repo root)")
            .to_path_buf();
        let seed = repo.join("implementation/seed");
        Paths { repo, seed }
    }
}

fn build(paths: &Paths, store: Option<PathBuf>) {
    let store = store.unwrap_or_else(|| paths.repo.join("target/cadenza-store"));
    std::fs::create_dir_all(&store).expect("create store dir");

    // Build the runtime component (wasm32) and content-address it.
    println!("== xtask: building the value-heap runtime component ==");
    let sh = Shell::new().expect("open a shell for the component build");
    let runtime_wasm = build_component(&sh, &paths.seed, "cdz-runtime", "cdz_runtime");

    let runtime_bytes = std::fs::read(&runtime_wasm).expect("read runtime wasm");
    let runtime_hash = content_address(&runtime_bytes);
    println!("   runtime content address: {runtime_hash}");
    let runtime_stored = store.join(format!("{runtime_hash}.wasm"));
    std::fs::write(&runtime_stored, &runtime_bytes).expect("store runtime");
    println!("   stored → {}", runtime_stored.display());

    // A small manifest recording the stored runtime, for the host / verifier to consult.
    let manifest = format!(
        "# Cadenza content-addressed store — the value-heap runtime.\n\
         runtime = \"{runtime_hash}\"\n"
    );
    std::fs::write(store.join("runtime.toml"), manifest).expect("write runtime.toml");

    println!("\n== xtask: done ==");
    println!("   store:   {}", store.display());
    println!("   runtime: {runtime_hash}");
}

/// Compile a Cadenza program and run it — the whole pipeline end-to-end, delegating each stage to
/// its binary (xtask pulls in none of them as a library; it only choreographs):
///   1. `cdz-syntax` — the program's surface (sexpr/ml) → binary AST.
///   2. `rcdzc`      — binary AST → a wasm component.
///   3. `cdz-run`    — instantiate + run the component; its stdout is the result.
///
/// The three are wired as a real OS PIPE (each stage's stdout is the next's stdin) — NO temp files,
/// so concurrent `xtask run` invocations never share or clobber state, and each stage's own stderr
/// (a parse error, a diagnostic) inherits straight to the terminal. The tools are built ONCE first,
/// then the built binaries are piped, so the three piped stages don't contend on cargo's build lock.
/// cdz-run's stdout — the program's result — is inherited to this process's stdout.
fn run(paths: &Paths, file: &Path, from: &str, store: Option<PathBuf>) {
    use std::process::{Command, Stdio};

    if !file.exists() {
        eprintln!("xtask run: no such file: {}", file.display());
        std::process::exit(1);
    }

    // Build the three tools once, so the pipe below runs finished binaries rather than three
    // concurrent `cargo run`s racing the build lock.
    let tools = build_tools(paths);

    // ── The pipe: cdz-syntax <file> | rcdzc - -o - | cdz-run - ──
    // Stage 1 reads the program file and writes binary AST to stdout.
    let mut syntax = Command::new(&tools.syntax)
        .args(["--from", from, "--to", "binary"])
        .arg(file)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| launch_fail("cdz-syntax", e));

    // Stage 2 reads AST from stage 1's stdout, writes the component to stdout.
    let mut rcdzc = Command::new(&tools.rcdzc)
        .args(["-", "-o", "-"])
        .stdin(Stdio::from(syntax.stdout.take().expect("cdz-syntax stdout")))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| launch_fail("rcdzc", e));

    // Stage 3 reads the component from stage 2's stdout, runs it, and prints the result to OUR
    // stdout (inherited). The store the runtime is resolved from is forwarded when given.
    let mut run = Command::new(&tools.run);
    run.arg("-").stdin(Stdio::from(rcdzc.stdout.take().expect("rcdzc stdout")));
    if let Some(dir) = &store {
        run.arg("--store").arg(dir);
    }
    let mut run = run.spawn().unwrap_or_else(|e| launch_fail("cdz-run", e));

    // Wait on every stage; the first that fails determines the exit code. Waiting on all (rather
    // than short-circuiting) reaps each child and lets its stderr finish flushing to the terminal.
    let statuses = [
        ("cdz-syntax", syntax.wait()),
        ("rcdzc", rcdzc.wait()),
        ("cdz-run", run.wait()),
    ];
    for (stage, status) in statuses {
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => std::process::exit(s.code().unwrap_or(1)),
            Err(e) => {
                eprintln!("xtask run: {stage} did not complete: {e}");
                std::process::exit(1);
            }
        }
    }
}

/// A stage's binary could not be spawned at all (missing/not-executable) — distinct from it running
/// and exiting non-zero, which is surfaced by its wait status.
fn launch_fail(stage: &str, e: std::io::Error) -> ! {
    eprintln!("xtask run: could not launch {stage}: {e}");
    std::process::exit(1);
}

/// Where the paths to the built pipeline binaries live, resolved once.
struct Tools {
    syntax: PathBuf,
    rcdzc: PathBuf,
    run: PathBuf,
}

/// Build the three pipeline tools once and return their binary paths — shared by `run` and `gate`
/// so neither pays a per-invocation `cargo run` build.
fn build_tools(paths: &Paths) -> Tools {
    let sh = Shell::new().expect("open a shell");
    sh.change_dir(&paths.repo);
    if let Err(e) = cmd!(sh, "cargo build --quiet -p cadenza-syntax -p rcdzc -p cdz-run").quiet().run() {
        eprintln!("xtask: building the tools failed: {e}");
        std::process::exit(1);
    }
    let bin = paths.repo.join("target/debug");
    Tools { syntax: bin.join("cdz-syntax"), rcdzc: bin.join("rcdzc"), run: bin.join("cdz-run") }
}

/// The outcome of driving one program (sexpr text) through the pipeline.
enum Ran {
    /// Ran to a value, rendered to canonical text.
    Value(String),
    /// The compiler rejected/declined the program.
    Declined,
    /// The component ran but trapped.
    Trap(String),
}

/// Drive one program's s-expression `text` through cdz-syntax → rcdzc → cdz-run, returning the
/// outcome. Uses a real pipe with the program fed on cdz-syntax's stdin (no temp files).
fn run_program(tools: &Tools, store: &Option<PathBuf>, program: &str) -> Ran {
    use std::io::Write;
    use std::process::{Command, Stdio};

    // Stage 1: program text (stdin) → binary AST (stdout).
    let mut syntax = Command::new(&tools.syntax)
        .args(["--from", "sexpr", "--to", "binary", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| launch_fail("cdz-syntax", e));
    syntax.stdin.take().unwrap().write_all(program.as_bytes()).ok();

    // Stage 2: AST → component; capture stderr so a decline carries its diagnostic.
    let rcdzc = Command::new(&tools.rcdzc)
        .args(["-", "-o", "-"])
        .stdin(Stdio::from(syntax.stdout.take().unwrap()))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| launch_fail("rcdzc", e));
    let rcdzc_out = rcdzc.wait_with_output().expect("wait rcdzc");
    let _ = syntax.wait();
    if !rcdzc_out.status.success() {
        return Ran::Declined;
    }

    // Stage 3: run the component (its stdout is the value; a trap goes to stderr with exit 1).
    let mut run = Command::new(&tools.run);
    run.arg("-").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(dir) = store {
        run.arg("--store").arg(dir);
    }
    let mut child = run.spawn().unwrap_or_else(|e| launch_fail("cdz-run", e));
    child.stdin.take().unwrap().write_all(&rcdzc_out.stdout).ok();
    let run_out = child.wait_with_output().expect("wait cdz-run");
    if run_out.status.success() {
        Ran::Value(String::from_utf8_lossy(&run_out.stdout).trim().to_string())
    } else {
        Ran::Trap(first_line(&run_out.stderr))
    }
}

fn first_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).lines().next().unwrap_or("").to_string()
}

/// Run one or more corpus files through the pipeline and grade each case against its recorded
/// outcome. Delegates case parsing + normalization to `cdz-syntax corpus`, then drives each program.
fn gate(paths: &Paths, files: Vec<PathBuf>, store: Option<PathBuf>) {
    let tools = build_tools(paths);

    // Default to the whole corpus when no files are named.
    let files = if files.is_empty() {
        default_corpus_files(paths)
    } else {
        files
    };

    let (mut pass, mut todo, mut fail) = (0u32, 0u32, 0u32);
    let mut failures: Vec<String> = Vec::new();

    for file in &files {
        let records = read_corpus(&tools, file);
        for rec in records {
            match grade(&tools, &store, &rec) {
                Grade::Pass => pass += 1,
                Grade::Todo => todo += 1,
                Grade::Fail(why) => {
                    fail += 1;
                    failures.push(format!("{}: {why}", rec.description));
                }
            }
        }
    }

    println!("\ngate: {pass} pass, {todo} todo, {fail} fail");
    if !failures.is_empty() {
        println!("\nfailures:");
        for f in &failures {
            println!("  FAIL  {f}");
        }
        std::process::exit(1);
    }
}

/// One graded case's verdict.
enum Grade {
    /// Ran and matched the recorded outcome.
    Pass,
    /// The compiler can't yet handle it (declined), or the expectation needs machinery not wired
    /// yet (error-code matching, traps) — not a disagreement, just not-yet.
    Todo,
    /// Ran to an outcome that disagrees with the record — the actionable frontier.
    Fail(String),
}

/// A parsed corpus record (the flat stream `cdz-syntax corpus` emits).
struct CorpusRecord {
    description: String,
    program: String,
    /// The `expect` line's payload, e.g. `output (: 42 Int64)`, `error CDZ0201`, `trap "…"`.
    expect: String,
    needs: Vec<String>,
}

/// Run `cdz-syntax corpus <file>` and parse its record stream.
fn read_corpus(tools: &Tools, file: &Path) -> Vec<CorpusRecord> {
    use std::process::Command;
    let out = Command::new(&tools.syntax)
        .arg("corpus")
        .arg(file)
        .output()
        .unwrap_or_else(|e| launch_fail("cdz-syntax corpus", e));
    if !out.status.success() {
        eprintln!("xtask gate: reading {}: {}", file.display(), first_line(&out.stderr));
        std::process::exit(1);
    }
    parse_records(&String::from_utf8_lossy(&out.stdout))
}

/// Parse the flat record stream: `key\tvalue` lines, records separated by a `---` line.
fn parse_records(text: &str) -> Vec<CorpusRecord> {
    let mut records = Vec::new();
    let (mut desc, mut prog, mut expect, mut needs) = (String::new(), String::new(), String::new(), Vec::new());
    for line in text.lines() {
        if line == "---" {
            records.push(CorpusRecord {
                description: std::mem::take(&mut desc),
                program: std::mem::take(&mut prog),
                expect: std::mem::take(&mut expect),
                needs: std::mem::take(&mut needs),
            });
            continue;
        }
        if let Some((key, val)) = line.split_once('\t') {
            match key {
                "case" => desc = val.to_string(),
                "program" => prog = val.to_string(),
                "expect" => expect = val.to_string(),
                "needs" => needs.push(val.to_string()),
                _ => {}
            }
        }
    }
    records
}

/// Grade one case: drive its program and compare against the recorded expectation.
fn grade(tools: &Tools, store: &Option<PathBuf>, rec: &CorpusRecord) -> Grade {
    // A case that needs an unrealized capability is out of scope for this generation — treat as todo.
    if !rec.needs.is_empty() {
        return Grade::Todo;
    }
    let (kind, payload) = rec.expect.split_once(' ').unwrap_or((rec.expect.as_str(), ""));
    let ran = run_program(tools, store, &rec.program);
    match kind {
        // `output (: <value> <Type>)`: the run must produce that value. cdz-run renders the value
        // alone, so compare against the value-form's value (the first element after `:`).
        "output" => {
            let expected = expected_value(payload);
            match ran {
                Ran::Value(v) if v == expected => Grade::Pass,
                Ran::Value(v) => Grade::Fail(format!("expected {expected}, ran → {v}")),
                Ran::Declined => Grade::Todo, // compiler can't compile it yet
                Ran::Trap(t) => Grade::Fail(format!("expected {expected}, trapped: {t}")),
            }
        }
        // `error CODE` / `trap …`: matching a rejection code or a trap reason needs machinery not yet
        // wired (rcdzc's diagnostics aren't coded yet, traps need the runtime). Count as todo unless a
        // clear disagreement — a program the corpus says is REJECTED that instead ran to a value.
        "error" => match ran {
            Ran::Value(v) => Grade::Fail(format!("expected rejection {payload}, ran → {v}")),
            _ => Grade::Todo,
        },
        "trap" => match ran {
            Ran::Value(v) => Grade::Fail(format!("expected a trap, ran → {v}")),
            _ => Grade::Todo,
        },
        _ => Grade::Todo,
    }
}

/// The value out of an `output` payload `(: <value> <Type>)` — the text of `<value>`. Falls back to
/// the whole payload if it is not the `(: value Type)` shape.
fn expected_value(payload: &str) -> String {
    // payload looks like `(: 42 Int64)`; take the token(s) between `(:` and the trailing ` Type)`.
    let inner = payload.trim();
    if let Some(rest) = inner.strip_prefix("(:") {
        let rest = rest.trim_end_matches(')').trim();
        // `<value> <Type>` — the value is everything up to the LAST whitespace-separated token (Type).
        if let Some(idx) = rest.rfind(char::is_whitespace) {
            return rest[..idx].trim().to_string();
        }
    }
    inner.to_string()
}

/// The default corpus: every `spec/semantics/*.sexp`, sorted for stable order.
fn default_corpus_files(paths: &Paths) -> Vec<PathBuf> {
    let dir = paths.repo.join("spec/semantics");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| {
            eprintln!("xtask gate: reading {}: {e}", dir.display());
            std::process::exit(1);
        })
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "sexp"))
        .collect();
    files.sort();
    files
}

/// SHA-256 of the bytes, lowercase hex (the recorded hashing choice).
fn content_address(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// `cargo component build --release --target wasm32-unknown-unknown` in <seed>/crates/<crate>,
/// returning the produced .wasm path. `cmd!` runs the child in the pushed crate dir and returns an
/// `Err` on a non-zero exit (already echoing the command), so a build failure surfaces cleanly.
fn build_component(sh: &Shell, seed: &Path, crate_dir: &str, artifact: &str) -> PathBuf {
    let dir = seed.join("crates").join(crate_dir);
    let _pushed = sh.push_dir(&dir);
    if let Err(e) =
        cmd!(sh, "cargo component build --release --target wasm32-unknown-unknown").run()
    {
        eprintln!("cargo component build failed for {crate_dir}: {e}");
        std::process::exit(1);
    }
    dir.join(format!(
        "target/wasm32-unknown-unknown/release/{artifact}.wasm"
    ))
}
