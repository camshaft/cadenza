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
}

fn main() {
    let paths = Paths::resolve();
    match Cli::parse().command {
        Cmd::Build { store } => build(&paths, store),
        Cmd::Run { file, from, store } => run(&paths, &file, &from, store),
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

    // Build the three tools once (one cargo invocation), so the pipe below runs the finished
    // binaries rather than three concurrent `cargo run`s racing the build lock.
    eprintln!("== xtask run: building the pipeline tools ==");
    let sh = Shell::new().expect("open a shell");
    sh.change_dir(&paths.repo);
    if let Err(e) = cmd!(sh, "cargo build --quiet -p cadenza-syntax -p rcdzc -p cdz-run").run() {
        eprintln!("xtask run: building the tools failed: {e}");
        std::process::exit(1);
    }
    let bin = paths.repo.join("target/debug");

    // ── The pipe: cdz-syntax <file> | rcdzc - -o - | cdz-run - ──
    // Stage 1 reads the program file and writes binary AST to stdout.
    let mut syntax = Command::new(bin.join("cdz-syntax"))
        .args(["--from", from, "--to", "binary"])
        .arg(file)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| launch_fail("cdz-syntax", e));

    // Stage 2 reads AST from stage 1's stdout, writes the component to stdout.
    let mut rcdzc = Command::new(bin.join("rcdzc"))
        .args(["-", "-o", "-"])
        .stdin(Stdio::from(syntax.stdout.take().expect("cdz-syntax stdout")))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| launch_fail("rcdzc", e));

    // Stage 3 reads the component from stage 2's stdout, runs it, and prints the result to OUR
    // stdout (inherited). The store the runtime is resolved from is forwarded when given.
    let mut run = Command::new(bin.join("cdz-run"));
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
