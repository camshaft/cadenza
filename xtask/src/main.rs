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
//! unknown subcommand/flag. xtask pulls in NO workspace crate as a library; it choreographs the
//! built binaries (cdz-syntax / rcdzc / cdz-run) so every command exercises the real tools.
//!
//! Commands:
//!   setup       one-time: symlink .claude/{skills,commands} → the tracked skills/ + commands/
//!   build       build the value-heap runtime component + content-address it into the store
//!   run         compile-and-run one program end-to-end (surface → AST → component → result)
//!   emit        the compile-only half of `run` — write the component, don't run it
//!   gate        run the corpus and grade each case (--case, --save, --check baseline)
//!   check       omnibus health check: fmt + build + test + clippy(-D warnings) + wasm-runtime + gate
//!   roundtrip   every corpus program round-trips through the syntax surfaces
//!   fmt         format program file(s) through the printer (--check for CI)
//!
//! Global `--profile <name>` chooses the cargo profile the pipeline tools are built under; it
//! defaults to `release-debug` (optimized, so the corpus gate is fast). Pass `--profile dev` for a
//! quick unoptimized build when iterating on the tools themselves.

use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use xshell::{Shell, cmd};

/// The one interface for driving the Cadenza seed workspace. Every knob is a typed flag; there are
/// no environment-variable knobs.
#[derive(Parser)]
#[command(
    name = "xtask",
    about = "The one interface for driving the Cadenza workspace."
)]
struct Cli {
    /// The cargo profile the pipeline tools (cdz-syntax / rcdzc / cdz-run) are built under. Defaults
    /// to `release-debug` (optimized, so the corpus gate is fast); pass `--profile dev` for a quick
    /// unoptimized build when iterating on the tools themselves.
    #[arg(long, global = true, default_value = "release-debug")]
    profile: String,

    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// One-time checkout bootstrap: symlink the gitignored `.claude/{skills,commands}` at the tracked
    /// `skills/`/`commands/` at the repo root, so this checkout's Claude picks them up. Idempotent.
    Setup,
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
        /// Run only cases whose description contains this substring, printing each one's normalized
        /// program, expected result, and actual outcome — the single-case debug loop.
        #[arg(long)]
        case: Option<String>,
        /// Save the current per-case verdicts as the committed baseline, then exit.
        #[arg(long, conflicts_with = "check")]
        save: bool,
        /// Compare the current verdicts to the baseline; fail on any case that REGRESSED
        /// (pass→not-pass), even while totals shift. Reports regressions and newly-passing cases.
        #[arg(long)]
        check: bool,
    },
    /// The omnibus health check: cargo fmt --check, workspace build, tests, clippy (`-D warnings`),
    /// the wasm runtime build, and the behavior gate. Each step's output is captured to a log file
    /// (`target/xtask-logs/`); the console shows one ✓ per step, and the first failing step prints
    /// the whole log + its path.
    Check,
    /// Round-trip every corpus program through the syntax surfaces (sexpr→binary→sexpr and
    /// sexpr→ml→sexpr) and confirm each reproduces a structurally-equal AST — guards `cadenza-syntax`
    /// independently of whether the compiler can compile anything.
    Roundtrip {
        /// The corpus `.sexp` files to check. [default: all of spec/semantics/*.sexp]
        files: Vec<PathBuf>,
    },
    /// Format Cadenza program file(s) through the printer, rewriting them in place.
    Fmt {
        /// The `.cdz`/`.sexp` files to format.
        files: Vec<PathBuf>,
        /// The surface to format to. [default: sexpr]
        #[arg(long, default_value = "sexpr")]
        to: String,
        /// Don't write; exit non-zero if any file is not already formatted (for CI).
        #[arg(long)]
        check: bool,
    },
    /// Compile a Cadenza program to a component and write it out (the compile-only half of `run`).
    Emit {
        /// The Cadenza program file to compile.
        file: PathBuf,
        /// The input surface. [default: sexpr]
        #[arg(long, default_value = "sexpr")]
        from: String,
        /// Where to write the component. [default: <file>.wasm]
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
    /// Generate the structured value-heap runtime-ABI table the wasm backend consumes. Reads the
    /// runtime's `wit/runtime.wit` (the ABI's source of truth) with `wit-parser` and writes
    /// `crates/rcdzc/src/backend/wasm/runtime_abi.rs` — every declared op as `{ name, params, result }`
    /// core-signature data. The compiler builds its per-program import section from this rather than
    /// pasting opaque envelope blobs. Run after changing the runtime WIT; the output is committed.
    Codegen {
        /// Don't write; regenerate in memory and exit non-zero if the committed file is out of date.
        /// This is the STALENESS GATE (wired into `xtask check`): it makes a forgotten regeneration a
        /// hard failure rather than a silent drift, so the generated ABI can never fall behind the WIT.
        #[arg(long)]
        check: bool,
    },
    /// Run the runtime allocation benchmark (gross heap allocs per hot op) and diff against the
    /// committed baseline `spec/bench/.alloc-baseline`. Allocation count — not wall-clock — is the
    /// tracked metric: it is identical native↔wasm and deterministic, so it catches an allocation
    /// regression the way `gate --check` catches a behavior one. Exits non-zero on a regression.
    Bench {
        /// Record the current counts as the committed baseline, then exit.
        #[arg(long)]
        save: bool,
    },
}

fn main() {
    let paths = Paths::resolve();
    let cli = Cli::parse();
    let profile = cli.profile.as_str();
    match cli.command {
        Cmd::Setup => setup(&paths),
        // `build` builds the runtime component under its own release settings (cargo component), not
        // the tool profile — so the profile flag doesn't apply to it.
        Cmd::Build { store } => build(&paths, store),
        Cmd::Run { file, from, store } => run(&paths, profile, &file, &from, store),
        Cmd::Gate {
            files,
            store,
            case,
            save,
            check,
        } => gate(
            &paths,
            profile,
            GateOpts {
                files,
                store,
                case,
                save,
                check,
            },
        ),
        Cmd::Check => check(&paths, profile),
        Cmd::Roundtrip { files } => roundtrip(&paths, profile, files),
        Cmd::Fmt { files, to, check } => fmt(&paths, profile, files, &to, check),
        Cmd::Emit { file, from, out } => emit(&paths, profile, &file, &from, out),
        Cmd::Codegen { check } => codegen::run(&paths, check),
        Cmd::Bench { save } => bench::run(&paths, save),
    }
}

mod bench;
mod codegen;

/// The workspace directory anchors, resolved once from this crate's manifest location. xtask lives
/// at `<repo>/xtask`, so the repo root is the manifest's parent and the seed workspace is the fixed
/// `<repo>/implementation/seed` beneath it. Every path derives from these two — no fragile
/// `.parent().parent()` chains, and correct inside a git worktree (each worktree's manifest dir
/// resolves to that worktree's own root).
struct Paths {
    /// `<repo>` — the workspace root (parent of `<repo>/xtask`).
    repo: PathBuf,
    /// `<repo>/implementation/seed` — the seed toolchain root that holds `crates/`.
    pub(crate) seed: PathBuf,
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

/// One-time checkout bootstrap: point the gitignored `.claude/{skills,commands}` at the tracked
/// `skills/`/`commands/` at the repo root. `.claude/` is not committed (so a fresh clone or a new
/// worktree lacks these links), but the sources travel with the repo — this wires them up. Idempotent:
/// an already-correct link is left alone; a real directory or wrong link in the way is reported, not
/// clobbered, so a hand-made setup is never destroyed silently.
fn setup(paths: &Paths) {
    let claude = paths.repo.join(".claude");
    std::fs::create_dir_all(&claude).expect("create .claude dir");

    let mut any_change = false;
    for name in ["skills", "commands"] {
        // The source must exist to link at (it is tracked in the repo).
        if !paths.repo.join(name).is_dir() {
            eprintln!("  ! {name}: no `{name}/` at the repo root to link — skipped");
            continue;
        }
        let link = claude.join(name);
        let want = PathBuf::from("..").join(name); // relative: `.claude/<name>` → `../<name>`

        // Already the correct symlink? Leave it.
        if link.is_symlink() && std::fs::read_link(&link).ok() == Some(want.clone()) {
            println!("  ✓ .claude/{name} already linked");
            continue;
        }
        // A real directory (e.g. the old copied layout) — do not delete the user's files.
        if link.exists() && !link.is_symlink() {
            eprintln!(
                "  ! .claude/{name} is a real directory, not a symlink — move it aside and re-run \
                 `cargo xtask setup` to link it to `{name}/`"
            );
            continue;
        }
        // A stale/wrong symlink: safe to replace (a symlink owns no content).
        if link.is_symlink() {
            let _ = std::fs::remove_file(&link);
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&want, &link).expect("create symlink");
        #[cfg(not(unix))]
        {
            eprintln!(
                "  ! symlinks not supported on this platform — link .claude/{name} → {name}/ by hand"
            );
            continue;
        }
        println!("  + linked .claude/{name} → {}", want.display());
        any_change = true;
    }

    if any_change {
        println!("\nsetup: done — Claude will pick up this checkout's skills and commands.");
    } else {
        println!("\nsetup: nothing to do (already wired up).");
    }
}

fn build(paths: &Paths, store: Option<PathBuf>) {
    let store = store.unwrap_or_else(|| paths.repo.join("target/cadenza-store"));
    std::fs::create_dir_all(&store).expect("create store dir");

    // Build BOTH runtime components (wasm32) and content-address each into the store: the RELEASE
    // runtime (what a shipped program pins + composes) and the DEBUG-COUNTERS runtime (the same code
    // with the `live-objects` leak counter — the Perceus leak-check harness composes it, located by
    // content address, never rebuilt). The debug build is read BEFORE the release build overwrites the
    // shared output path.
    println!("== xtask: building the value-heap runtime component (release + debug-counters) ==");
    let sh = Shell::new().expect("open a shell for the component build");

    let debug_wasm = build_component_with_features(
        &sh,
        &paths.seed,
        "cdz-runtime",
        "cdz_runtime",
        &["debug-counters"],
    );
    // CANONICALIZE (strip the tool-version `producers` sections) before hashing + storing, so the hash
    // is reproducible across machines with the same rustc — the stored artifact IS the stripped bytes,
    // so a composed program's imported hash matches the file on disk.
    let debug_bytes = canonicalize_runtime(&debug_wasm);
    let debug_hash = content_address(&debug_bytes);
    std::fs::write(store.join(format!("{debug_hash}.wasm")), &debug_bytes).expect("store debug rt");
    println!("   debug-counters runtime content address: {debug_hash}");

    let runtime_wasm = build_component(&sh, &paths.seed, "cdz-runtime", "cdz_runtime");
    let runtime_bytes = canonicalize_runtime(&runtime_wasm);
    let runtime_hash = content_address(&runtime_bytes);
    println!("   runtime content address: {runtime_hash}");
    let runtime_stored = store.join(format!("{runtime_hash}.wasm"));
    std::fs::write(&runtime_stored, &runtime_bytes).expect("store runtime");
    println!("   stored → {}", runtime_stored.display());

    // A small manifest recording both stored runtimes, for the host / verifier to consult.
    let manifest = format!(
        "# Cadenza content-addressed store — the value-heap runtime.\n\
         runtime = \"{runtime_hash}\"\n\
         debug_runtime = \"{debug_hash}\"\n"
    );
    std::fs::write(store.join("runtime.toml"), manifest).expect("write runtime.toml");

    println!("\n== xtask: done ==");
    println!("   store:   {}", store.display());
    println!("   runtime: {runtime_hash}");
    println!("   debug:   {debug_hash}");
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
fn run(paths: &Paths, profile: &str, file: &Path, from: &str, store: Option<PathBuf>) {
    use std::process::{Command, Stdio};

    if !file.exists() {
        eprintln!("xtask run: no such file: {}", file.display());
        std::process::exit(1);
    }

    // Build the three tools once, so the pipe below runs finished binaries rather than three
    // concurrent `cargo run`s racing the build lock.
    let tools = build_tools(paths, profile);

    // ── The pipe: cdz-syntax <file> | rcdzc - -o - | cdz-run - ──
    // Stage 1 reads the program file and writes binary AST to stdout.
    let mut syntax = Command::new(&tools.syntax)
        .args(["convert", "--from", from, "--to", "binary"])
        .arg(file)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| launch_fail("cdz-syntax", e));

    // Stage 2 reads AST from stage 1's stdout, writes the component to stdout.
    let mut rcdzc = Command::new(&tools.rcdzc)
        .args(["-", "-o", "-"])
        .stdin(Stdio::from(
            syntax.stdout.take().expect("cdz-syntax stdout"),
        ))
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| launch_fail("rcdzc", e));

    // Stage 3 reads the component from stage 2's stdout, runs it, and prints the result to OUR
    // stdout (inherited). The store the runtime is resolved from is forwarded when given.
    let mut run = Command::new(&tools.run);
    run.arg("-")
        .stdin(Stdio::from(rcdzc.stdout.take().expect("rcdzc stdout")));
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
    corpus: PathBuf,
    rcdzc: PathBuf,
    run: PathBuf,
}

/// Build the three pipeline tools once (under `profile`) and return their binary paths — shared by
/// `run`/`gate`/`roundtrip`/`fmt`/`emit` so none pays a per-invocation `cargo run` build. The
/// interactive commands use `dev` (fast build); the corpus gate uses `release-debug` (optimized), so
/// that the ~900-case run is not dominated by unoptimized tools.
fn build_tools(paths: &Paths, profile: &str) -> Tools {
    let sh = Shell::new().expect("open a shell");
    sh.change_dir(&paths.repo);
    if let Err(e) = cmd!(
        sh,
        "cargo build --quiet --profile {profile} -p cadenza-syntax -p cdz-corpus -p rcdzc -p cdz-run"
    )
    .quiet()
    .run()
    {
        eprintln!("xtask: building the tools failed: {e}");
        std::process::exit(1);
    }
    // Cargo puts the `dev` profile's artifacts under `target/debug`; every other profile lands under
    // `target/<profile>`.
    let subdir = if profile == "dev" { "debug" } else { profile };
    let bin = paths.repo.join("target").join(subdir);
    Tools {
        syntax: bin.join("cdz-syntax"),
        corpus: bin.join("cdz-corpus"),
        rcdzc: bin.join("rcdzc"),
        run: bin.join("cdz-run"),
    }
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
/// outcome. Uses a real pipe with the program fed on cdz-syntax's stdin (no temp files). When `call`
/// is given, the export is invoked with those runtime arguments (`--call <export> --arg <v>…`) — how a
/// case exercises a parameterized entrypoint rather than a nullary one; `None` runs the sole export
/// with no arguments (the common case).
fn run_program(tools: &Tools, store: &Option<PathBuf>, program: &str, call: Option<&Call>) -> Ran {
    use std::io::Write;
    use std::process::{Command, Stdio};

    // Stage 1: program text (stdin) → binary AST (stdout).
    let mut syntax = Command::new(&tools.syntax)
        .args(["convert", "--from", "sexpr", "--to", "binary", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| launch_fail("cdz-syntax", e));
    syntax
        .stdin
        .take()
        .unwrap()
        .write_all(program.as_bytes())
        .ok();

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
    run.arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = store {
        run.arg("--store").arg(dir);
    }
    // A `(call …)` case names the export and passes runtime arguments; cdz-run coerces each `--arg` to
    // the export's declared parameter type (its `--arg` allows a leading `-`, so a negative value is
    // taken as the argument, not a flag).
    if let Some(call) = call {
        run.arg("--call").arg(&call.export);
        for arg in &call.args {
            run.arg("--arg").arg(arg);
        }
    }
    let mut child = run.spawn().unwrap_or_else(|e| launch_fail("cdz-run", e));
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&rcdzc_out.stdout)
        .ok();
    let run_out = child.wait_with_output().expect("wait cdz-run");
    if run_out.status.success() {
        Ran::Value(String::from_utf8_lossy(&run_out.stdout).trim().to_string())
    } else {
        Ran::Trap(first_line(&run_out.stderr))
    }
}

fn first_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .next()
        .unwrap_or("")
        .to_string()
}

/// Options for `gate` (grows without re-threading a widening arg list).
struct GateOpts {
    files: Vec<PathBuf>,
    store: Option<PathBuf>,
    case: Option<String>,
    save: bool,
    check: bool,
}

/// Run one or more corpus files through the pipeline and grade each case against its recorded
/// outcome. Delegates case parsing + normalization to `cdz-syntax corpus`, then drives each program.
fn gate(paths: &Paths, profile: &str, opts: GateOpts) {
    let tools = build_tools(paths, profile);
    let files = if opts.files.is_empty() {
        default_corpus_files(paths)
    } else {
        opts.files.clone()
    };

    // `--case`: run only matching cases and print each one's program / expected / actual — the
    // single-case debug loop, not a pass/fail tally.
    if let Some(needle) = &opts.case {
        gate_one_case(&tools, &opts.store, &files, needle);
        return;
    }

    // Gather every case (file then case order) into one flat list, then grade them in PARALLEL. Each
    // case is independent — `grade` only READS `tools`/`store` and spawns its own subprocess pipeline,
    // with no shared mutable state — so grading is embarrassingly parallel. The serial loop spent ~all
    // its wall time waiting on ~3 spawned processes per case (cdz-syntax → rcdzc → cdz-run) over 1000+
    // cases; fanning the cases across cores collapses that wait. Order is PRESERVED (each result is
    // written to its own index) because the baseline compare (`check_baseline`/`save_baseline`) is
    // positional — a race in verdict order would spuriously flag regressions.
    let records: Vec<CorpusRecord> = files
        .iter()
        .flat_map(|file| read_corpus(&tools, file))
        .collect();
    let graded = grade_all_parallel(&tools, &opts.store, records);

    // Reassemble the tally + the ordered verdict list from the in-order graded results.
    let (mut pass, mut todo, mut fail) = (0u32, 0u32, 0u32);
    let mut failures: Vec<String> = Vec::new();
    let mut verdicts: Vec<(String, Verdict)> = Vec::new();
    for (description, grade) in graded {
        let v = match grade {
            Grade::Pass => {
                pass += 1;
                Verdict::Pass
            }
            Grade::Todo => {
                todo += 1;
                Verdict::Todo
            }
            Grade::Fail(why) => {
                fail += 1;
                failures.push(format!("{description}: {why}"));
                Verdict::Fail
            }
        };
        verdicts.push((description, v));
    }

    println!("\ngate: {pass} pass, {todo} todo, {fail} fail");
    if !failures.is_empty() {
        println!("\nfailures:");
        for f in &failures {
            println!("  FAIL  {f}");
        }
    }

    if opts.save {
        save_baseline(paths, &verdicts);
        println!(
            "\nbaseline saved: {} cases → {}",
            verdicts.len(),
            baseline_path(paths).display()
        );
        return;
    }
    if opts.check {
        // A regression (a case that used to pass and now doesn't) fails the check even if the
        // totals look fine — the trap the raw counts hide. Newly-passing cases are reported, not failed.
        std::process::exit(check_baseline(paths, &verdicts));
    }
    if fail > 0 {
        std::process::exit(1);
    }
}

/// Grade every record in PARALLEL, returning `(description, grade)` in the SAME order as `records`.
///
/// The work is process-bound (each `grade` spawns cdz-syntax → rcdzc → cdz-run and waits), so a pool
/// of worker threads pulling from a shared cursor keeps many pipelines in flight at once. Uses
/// `std::thread::scope` — the workers borrow `tools`/`store`/`records` for the scope's lifetime, so no
/// `Arc`/clone and no extra dependency. Order is preserved by writing each result into its own slot
/// (indexed by the case's position), never by push order — the baseline compare is positional, so a
/// reordering would read as a spurious regression. The worker count is bounded by the machine's
/// parallelism but not below 1; the cases-per-core ratio is high, so a simple shared atomic cursor
/// (no work-stealing) balances well enough.
fn grade_all_parallel(
    tools: &Tools,
    store: &Option<PathBuf>,
    records: Vec<CorpusRecord>,
) -> Vec<(String, Grade)> {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let n = records.len();
    // One result slot per case, filled by index (order-preserving). `Mutex<Option<_>>` is the simplest
    // sound cell here; contention is nil (each slot is written once, by one worker).
    let slots: Vec<Mutex<Option<(String, Grade)>>> = (0..n).map(|_| Mutex::new(None)).collect();
    let cursor = AtomicUsize::new(0);

    // Cap workers at the machine's parallelism (min 1). More threads than cores buys nothing here —
    // the pipeline stages are separate processes the OS already schedules across cores.
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .max(1);

    std::thread::scope(|scope| {
        for _ in 0..workers {
            let cursor = &cursor;
            let slots = &slots;
            let records = &records;
            scope.spawn(move || {
                loop {
                    let i = cursor.fetch_add(1, Ordering::Relaxed);
                    if i >= records.len() {
                        break;
                    }
                    let rec = &records[i];
                    let grade = grade(tools, store, rec);
                    *slots[i].lock().unwrap() = Some((rec.description.clone(), grade));
                }
            });
        }
    });

    // All workers joined at scope end; every slot is filled exactly once.
    slots
        .into_iter()
        .map(|slot| slot.into_inner().unwrap().expect("every case graded"))
        .collect()
}

/// Run only the case(s) whose description contains `needle`, printing each one's normalized program,
/// expected result, and actual outcome. A focused debug view, not a tally.
fn gate_one_case(tools: &Tools, store: &Option<PathBuf>, files: &[PathBuf], needle: &str) {
    let mut found = 0;
    for file in files {
        for rec in read_corpus(tools, file) {
            if !rec.description.contains(needle) {
                continue;
            }
            found += 1;
            let ran = run_program(tools, store, &rec.program, rec.call.as_ref());
            let actual = match &ran {
                Ran::Value(v) => format!("value {v}"),
                Ran::Declined => "declined (compiler can't compile it yet)".to_string(),
                Ran::Trap(t) => format!("trap: {t}"),
            };
            let verdict = match grade_ran(&rec, &ran) {
                Grade::Pass => "PASS",
                Grade::Todo => "todo",
                Grade::Fail(_) => "FAIL",
            };
            println!("case:     {}", rec.description);
            println!("program:  {}", rec.program);
            if let Some(call) = &rec.call {
                println!("call:     {} {}", call.export, call.args.join(" "));
            }
            println!("expect:   {}", rec.expect);
            println!("actual:   {actual}");
            println!("verdict:  {verdict}\n");
        }
    }
    if found == 0 {
        eprintln!("xtask gate --case: no case matched {needle:?}");
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
    /// A `(call …)` clause: the export to invoke and its runtime arguments. `None` when the case has
    /// no `(call …)` — the program's sole export is run with no arguments (the common nullary case).
    call: Option<Call>,
    /// The `expect` line's payload, e.g. `output (: 42 Int64)`, `error CDZ0201`, `trap "…"`.
    expect: String,
    needs: Vec<String>,
}

/// A corpus case's `(call <export> <arg>…)` clause, parsed from the record stream. The export is run
/// with these arguments (already reduced to bare value text by `cdz-syntax corpus`), which cdz-run
/// coerces to the export's declared parameter types — the path that exercises a parameterized entry.
struct Call {
    export: String,
    args: Vec<String>,
}

/// Run `cdz-corpus records <file>` and parse its record stream.
fn read_corpus(tools: &Tools, file: &Path) -> Vec<CorpusRecord> {
    use std::process::Command;
    let out = Command::new(&tools.corpus)
        .arg("records")
        .arg(file)
        .output()
        .unwrap_or_else(|e| launch_fail("cdz-corpus records", e));
    if !out.status.success() {
        eprintln!(
            "xtask gate: reading {}: {}",
            file.display(),
            first_line(&out.stderr)
        );
        std::process::exit(1);
    }
    parse_records(&String::from_utf8_lossy(&out.stdout))
}

/// Parse the flat record stream: `key\tvalue` lines, records separated by a `---` line. A `call` line
/// (the export) and any following `arg` lines (its arguments, in order) build the record's optional
/// `(call …)` clause.
fn parse_records(text: &str) -> Vec<CorpusRecord> {
    let mut records = Vec::new();
    let (mut desc, mut prog, mut expect, mut needs) =
        (String::new(), String::new(), String::new(), Vec::new());
    let (mut call_export, mut call_args): (Option<String>, Vec<String>) = (None, Vec::new());
    for line in text.lines() {
        if line == "---" {
            let call = call_export.take().map(|export| Call {
                export,
                args: std::mem::take(&mut call_args),
            });
            records.push(CorpusRecord {
                description: std::mem::take(&mut desc),
                program: std::mem::take(&mut prog),
                call,
                expect: std::mem::take(&mut expect),
                needs: std::mem::take(&mut needs),
            });
            call_args.clear();
            continue;
        }
        if let Some((key, val)) = line.split_once('\t') {
            match key {
                "case" => desc = val.to_string(),
                "program" => prog = val.to_string(),
                "call" => call_export = Some(val.to_string()),
                "arg" => call_args.push(val.to_string()),
                "expect" => expect = val.to_string(),
                "needs" => needs.push(val.to_string()),
                _ => {}
            }
        }
    }
    records
}

/// Grade one case: drive its program, then compare against the recorded expectation.
fn grade(tools: &Tools, store: &Option<PathBuf>, rec: &CorpusRecord) -> Grade {
    let ran = run_program(tools, store, &rec.program, rec.call.as_ref());
    grade_ran(rec, &ran)
}

/// Compare an already-run outcome against a case's recorded expectation — the pure grading logic,
/// shared by the tally path and the single-case debug view.
fn grade_ran(rec: &CorpusRecord, ran: &Ran) -> Grade {
    // A case that needs an unrealized capability is out of scope for this generation — treat as todo.
    if !rec.needs.is_empty() {
        return Grade::Todo;
    }
    let (kind, payload) = rec
        .expect
        .split_once(' ')
        .unwrap_or((rec.expect.as_str(), ""));
    match kind {
        // `output (: <value> <Type>)`: the run must produce that value. A SCALAR crosses as a bare value
        // (`cdz-run` renders `42`), so it matches the value-form's value alone; a COMPOUND crosses via
        // the resource escape as the WHOLE `(: value type)` form (the host decodes the canonical bytes
        // and prints value AND type). Accept EITHER — the bare value (scalar) or the full form (compound)
        // — so both ABIs grade against the one recorded `(: value type)` outcome.
        "output" => {
            let expected_val = expected_value(payload);
            let expected_full = payload.trim().to_string();
            match ran {
                Ran::Value(v) if *v == expected_val || *v == expected_full => Grade::Pass,
                Ran::Value(v) => Grade::Fail(format!("expected {expected_full}, ran → {v}")),
                Ran::Declined => Grade::Todo, // compiler can't compile it yet
                Ran::Trap(t) => Grade::Fail(format!("expected {expected_full}, trapped: {t}")),
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

/// The default corpus: every `spec/semantics/NN-*.{sexp,md}`, sorted for stable order. Corpus files
/// follow the `NN-feature` naming convention (a numeric prefix), which distinguishes a migrated
/// corpus `.md` from an ordinary docs `.md` like `README.md` — only digit-led stems are corpus files.
/// A stem may exist as `.sexp` (source) and/or `.md` (migrated); during the migration both may
/// coexist, so a `.sexp` whose stem also has a `.md` is dropped — the `.md` wins. That way a file
/// cuts over to markdown the moment it is migrated, and its `.sexp` can be deleted afterward with
/// zero change to the gate.
fn default_corpus_files(paths: &Paths) -> Vec<PathBuf> {
    let dir = paths.repo.join("spec/semantics");
    let entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| {
            eprintln!("xtask gate: reading {}: {e}", dir.display());
            std::process::exit(1);
        })
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "sexp" || x == "md"))
        // Only the `NN-feature` corpus files, never an ordinary docs `.md` (e.g. README.md).
        .filter(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.starts_with(|c: char| c.is_ascii_digit()))
        })
        .collect();
    let migrated: std::collections::HashSet<_> = entries
        .iter()
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .filter_map(|p| p.file_stem().map(|s| s.to_os_string()))
        .collect();
    let mut files: Vec<PathBuf> = entries
        .into_iter()
        .filter(|p| {
            // Keep every `.md`; keep a `.sexp` only if its stem has no migrated `.md`.
            !(p.extension().is_some_and(|x| x == "sexp")
                && p.file_stem().is_some_and(|s| migrated.contains(s)))
        })
        .collect();
    files.sort();
    files
}

// ============================================================================================
// gate baseline — a committed per-case verdict snapshot, so a REGRESSION (a case that used to pass
// and now doesn't) fails `gate --check` even while the pass/todo/fail totals drift.
// ============================================================================================

#[derive(Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Pass,
    Todo,
    Fail,
}

impl Verdict {
    fn tag(self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Todo => "todo",
            Verdict::Fail => "fail",
        }
    }
    fn parse(s: &str) -> Option<Verdict> {
        match s {
            "pass" => Some(Verdict::Pass),
            "todo" => Some(Verdict::Todo),
            "fail" => Some(Verdict::Fail),
            _ => None,
        }
    }
}

/// The committed baseline file: `<repo>/spec/semantics/.gate-baseline`.
fn baseline_path(paths: &Paths) -> PathBuf {
    paths.repo.join("spec/semantics/.gate-baseline")
}

/// Write the current verdicts as the baseline: one `verdict\tdescription` line per case, sorted by
/// description so the file is stable and a diff is meaningful.
fn save_baseline(paths: &Paths, verdicts: &[(String, Verdict)]) {
    let mut lines: Vec<String> = verdicts
        .iter()
        .map(|(d, v)| format!("{}\t{d}", v.tag()))
        .collect();
    lines.sort();
    let body = format!(
        "# gate baseline — per-case verdicts (verdict\\tdescription). Regenerate with `cargo xtask gate --save`.\n{}\n",
        lines.join("\n")
    );
    std::fs::write(baseline_path(paths), body).expect("write baseline");
}

/// Compare current verdicts to the baseline. Returns the process exit code: non-zero if any case
/// REGRESSED (baseline pass → now not pass) or a baseline case vanished. Newly-passing cases and
/// new cases are reported but do not fail the check.
fn check_baseline(paths: &Paths, verdicts: &[(String, Verdict)]) -> i32 {
    let path = baseline_path(paths);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => {
            eprintln!(
                "xtask gate --check: no baseline at {} (create it with `gate --save`)",
                path.display()
            );
            return 2;
        }
    };
    let mut base: std::collections::HashMap<String, Verdict> = std::collections::HashMap::new();
    for line in text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some((v, d)) = line.split_once('\t')
            && let Some(verdict) = Verdict::parse(v)
        {
            base.insert(d.to_string(), verdict);
        }
    }

    let now: std::collections::HashMap<&str, Verdict> =
        verdicts.iter().map(|(d, v)| (d.as_str(), *v)).collect();

    let mut regressed: Vec<String> = Vec::new();
    let mut gained: Vec<String> = Vec::new();
    let mut vanished: Vec<String> = Vec::new();

    for (desc, &was) in &base {
        match now.get(desc.as_str()) {
            None => vanished.push(desc.clone()),
            Some(&is) => {
                if was == Verdict::Pass && is != Verdict::Pass {
                    regressed.push(format!("{desc} ({} → {})", was.tag(), is.tag()));
                } else if was != Verdict::Pass && is == Verdict::Pass {
                    gained.push(desc.clone());
                }
            }
        }
    }

    if !gained.is_empty() {
        println!("\nnewly passing ({}):", gained.len());
        for g in &gained {
            println!("  +  {g}");
        }
    }
    if !regressed.is_empty() {
        println!("\nREGRESSED ({}):", regressed.len());
        for r in &regressed {
            println!("  -  {r}");
        }
    }
    if !vanished.is_empty() {
        println!("\nvanished from the corpus ({}):", vanished.len());
        for v in &vanished {
            println!("  ?  {v}");
        }
    }

    if regressed.is_empty() && vanished.is_empty() {
        println!(
            "\ngate --check: OK (no regressions vs baseline; {} newly passing)",
            gained.len()
        );
        0
    } else {
        println!(
            "\ngate --check: FAIL ({} regressed, {} vanished)",
            regressed.len(),
            vanished.len()
        );
        1
    }
}

// ============================================================================================
// check — the omnibus health check.
// ============================================================================================

/// Run the whole health check: cargo fmt --check, workspace build, tests, clippy (`-D warnings`),
/// the wasm runtime build, and the behavior gate. Each step's full output is CAPTURED to a log file
/// rather than flooding the console; the console shows one ✓ per passing step. The first failing step
/// prints the whole captured log
/// (so an agent reads it in place instead of re-running with `| tail`) and its path, then exits.
fn check(paths: &Paths, profile: &str) {
    let mut log = Log::create(paths, "check");
    println!("check: logging to {}", log.path.display());

    // Each step runs its command with stdout+stderr appended to the log. Native workspace first:
    // formatting, build, test, then clippy. `fmt --check` and clippy `-D warnings` are HARD gates —
    // the workspace is cargo-fmt-clean and clippy-clean, and this keeps it that way (a lint or a
    // stray format is a failing step, with the offending diff/lint captured in the log to read).
    let repo = &paths.repo;
    log.step("fmt", "cargo fmt --all --check", repo);
    log.step("build", "cargo build --workspace", repo);
    log.step("test", "cargo test --workspace", repo);
    log.step("clippy", "cargo clippy --workspace -- -D warnings", repo);

    // The generated runtime-ABI table (`runtime_abi.rs`) MUST stay current with the runtime WIT — a
    // forgotten `cargo xtask codegen` after a WIT change would silently drift the compiler's import
    // ABI from the runtime. `codegen --check` regenerates in memory and fails if the committed file is
    // stale, so this is a HARD GATE (like `fmt --check`) — nobody has to remember to regenerate.
    let xtask = std::env::current_exe().expect("current exe");
    let xtask = xtask.to_string_lossy().to_string();
    log.step("codegen", &format!("{xtask} codegen --check"), repo);

    // The wasm runtime is EXCLUDED from the native workspace, so a plain `cargo build` skips it — a
    // silent gap the check closes by building it explicitly for its target.
    let rt = paths.seed.join("crates/cdz-runtime");
    log.step(
        "wasm-runtime",
        "cargo build --release --target wasm32-unknown-unknown",
        &rt,
    );

    // The behavior gate — invoke this same xtask binary. Use `gate --check` (vs the baseline) when a
    // baseline exists, so `check` asks "did anything REGRESS?" rather than "are there any known
    // gaps?" — a green check means the library is healthy AND the compiler didn't backslide. With no
    // baseline, fall back to a plain `gate`. The gate's own summary is short, so let it print to the
    // console (it is the useful signal) while its verbose build noise still lands in the log.
    let gate_cmd = if baseline_path(paths).exists() {
        format!("{xtask} --profile {profile} gate --check")
    } else {
        format!("{xtask} --profile {profile} gate")
    };
    log.step_show("gate", &gate_cmd, repo);

    println!("\ncheck: all green ✓  (full log: {})", log.path.display());
}

/// A captured-output log for a multi-step command. Each step's child process writes its stdout and
/// stderr into one appended file; the console stays quiet on success and gets the whole log on the
/// first failure (with the path), so there is nothing to re-run to see what happened.
struct Log {
    path: PathBuf,
    file: std::fs::File,
}

impl Log {
    /// Open `target/xtask-logs/<cmd>-<timestamp>.log` (timestamped, so runs don't overwrite).
    fn create(paths: &Paths, cmd: &str) -> Log {
        let dir = paths.repo.join("target/xtask-logs");
        std::fs::create_dir_all(&dir).expect("create xtask-logs dir");
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = dir.join(format!("{cmd}-{stamp}.log"));
        let file = std::fs::File::create(&path).expect("create log file");
        Log { path, file }
    }

    /// Run a step with its output captured to the log; print `✓ name` on success. On failure, dump
    /// the whole log to the console and exit. `cmd` is a `program arg arg…` string run in `dir`.
    fn step(&mut self, name: &str, cmd: &str, dir: &Path) {
        self.run_step(name, cmd, dir, false);
    }

    /// Like `step`, but the child's output ALSO streams to the console (tee) — for a step whose own
    /// output is a concise, useful signal (the gate's tally), not build noise.
    fn step_show(&mut self, name: &str, cmd: &str, dir: &Path) {
        self.run_step(name, cmd, dir, true);
    }

    fn run_step(&mut self, name: &str, cmd: &str, dir: &Path, show: bool) {
        use std::io::Write;
        writeln!(self.file, "\n==== {name}: {cmd} ====").ok();
        self.file.flush().ok();

        // Split the command string into program + args (our commands have no quoted args).
        let mut parts = cmd.split_whitespace();
        let program = parts.next().expect("non-empty command");
        let args: Vec<&str> = parts.collect();

        // Capture stdout+stderr. (A true live tee would need thread-per-pipe; capture-then-print is
        // enough here — steps are short and the console output is the point, not streaming.)
        let out = std::process::Command::new(program)
            .args(&args)
            .current_dir(dir)
            .output()
            .unwrap_or_else(|e| {
                eprintln!("  ✗ {name} — could not launch: {e}");
                std::process::exit(1);
            });
        self.file.write_all(&out.stdout).ok();
        self.file.write_all(&out.stderr).ok();
        self.file.flush().ok();
        if show {
            std::io::stdout().write_all(&out.stdout).ok();
            std::io::stderr().write_all(&out.stderr).ok();
        }

        if out.status.success() {
            println!("  ✓ {name}");
        } else {
            eprintln!("  ✗ {name} — FAILED");
            self.dump_and_exit(name);
        }
    }

    /// Print the whole captured log to the console (so the failure is readable without re-running)
    /// plus its path, then exit non-zero.
    fn dump_and_exit(&self, failed_step: &str) -> ! {
        eprintln!("\n──── full log ({}) ────", self.path.display());
        if let Ok(text) = std::fs::read_to_string(&self.path) {
            eprint!("{text}");
        }
        eprintln!("──── end log ────");
        eprintln!(
            "\ncheck: FAILED at `{failed_step}` — full log above and at {}",
            self.path.display()
        );
        std::process::exit(1);
    }
}

// ============================================================================================
// roundtrip — the syntax surfaces round-trip on every corpus program.
// ============================================================================================

/// For every corpus program, confirm sexpr→binary→sexpr and sexpr→ml→sexpr reproduce a
/// structurally-equal AST — i.e. re-encoding the round-tripped text to binary yields the SAME bytes
/// as the original. Guards `cadenza-syntax` (reader/printer/codec) independently of the compiler.
fn roundtrip(paths: &Paths, profile: &str, files: Vec<PathBuf>) {
    let tools = build_tools(paths, profile);
    let files = if files.is_empty() {
        default_corpus_files(paths)
    } else {
        files
    };

    let (mut ok, mut fail) = (0u32, 0u32);
    let mut failures: Vec<String> = Vec::new();

    for file in &files {
        for rec in read_corpus(&tools, file) {
            // The reference: the program's canonical binary AST.
            let bin0 = match to_binary(&tools, &rec.program) {
                Some(b) => b,
                None => {
                    fail += 1;
                    failures.push(format!("{}: sexpr→binary failed", rec.description));
                    continue;
                }
            };
            // Round-trip through each intermediate surface, back to binary, and compare bytes.
            for surface in ["sexpr", "ml"] {
                match roundtrip_via(&tools, &bin0, surface) {
                    Some(bin1) if bin1 == bin0 => {}
                    Some(_) => {
                        fail += 1;
                        failures.push(format!("{}: binary≠binary via {surface}", rec.description));
                    }
                    None => {
                        fail += 1;
                        failures.push(format!(
                            "{}: round-trip via {surface} errored",
                            rec.description
                        ));
                    }
                }
            }
            ok += 1;
        }
    }

    println!("\nroundtrip: {ok} programs ok, {fail} failures");
    if !failures.is_empty() {
        println!();
        for f in failures.iter().take(40) {
            println!("  FAIL  {f}");
        }
        if failures.len() > 40 {
            println!("  … and {} more", failures.len() - 40);
        }
        std::process::exit(1);
    }
}

/// A program's sexpr text → its canonical binary AST bytes (via `cdz-syntax`).
fn to_binary(tools: &Tools, program: &str) -> Option<Vec<u8>> {
    convert_bytes(tools, program.as_bytes(), "sexpr", "binary")
}

/// binary → <surface> text → binary, returning the re-encoded bytes (to compare to the original).
fn roundtrip_via(tools: &Tools, bin0: &[u8], surface: &str) -> Option<Vec<u8>> {
    let text = convert_bytes(tools, bin0, "binary", surface)?;
    convert_bytes(tools, &text, surface, "binary")
}

/// Run `cdz-syntax --from <from> --to <to>` over `input` bytes (stdin) and return its stdout.
fn convert_bytes(tools: &Tools, input: &[u8], from: &str, to: &str) -> Option<Vec<u8>> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new(&tools.syntax)
        .args(["convert", "--from", from, "--to", to, "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| launch_fail("cdz-syntax", e));
    child.stdin.take().unwrap().write_all(input).ok();
    let out = child.wait_with_output().expect("wait cdz-syntax");
    out.status.success().then_some(out.stdout)
}

// ============================================================================================
// fmt — format program files through the printer.
// ============================================================================================

/// Format each file through the printer (round-trip its own surface to canonical form). `--check`
/// writes nothing and exits non-zero if any file is not already canonical.
fn fmt(paths: &Paths, profile: &str, files: Vec<PathBuf>, to: &str, check: bool) {
    if files.is_empty() {
        eprintln!("xtask fmt: name at least one file");
        std::process::exit(1);
    }
    let tools = build_tools(paths, profile);
    let mut unformatted: Vec<String> = Vec::new();

    for file in &files {
        let original = match std::fs::read(file) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("xtask fmt: {}: {e}", file.display());
                std::process::exit(1);
            }
        };
        // Format = parse the surface and re-print it canonically (same surface in and out).
        let formatted = match convert_bytes(&tools, &original, to, to) {
            Some(b) => b,
            None => {
                eprintln!("xtask fmt: {}: does not parse as {to}", file.display());
                std::process::exit(1);
            }
        };
        // The printer emits no trailing newline; keep files newline-terminated.
        let mut formatted = formatted;
        if !formatted.ends_with(b"\n") {
            formatted.push(b'\n');
        }
        if formatted == original {
            continue;
        }
        if check {
            unformatted.push(file.display().to_string());
        } else if let Err(e) = std::fs::write(file, &formatted) {
            eprintln!("xtask fmt: writing {}: {e}", file.display());
            std::process::exit(1);
        } else {
            println!("formatted {}", file.display());
        }
    }

    if check && !unformatted.is_empty() {
        println!("not formatted ({}):", unformatted.len());
        for f in &unformatted {
            println!("  {f}");
        }
        std::process::exit(1);
    }
}

// ============================================================================================
// emit — compile a program to a component and write it out (the compile-only half of `run`).
// ============================================================================================

fn emit(paths: &Paths, profile: &str, file: &Path, from: &str, out: Option<PathBuf>) {
    use std::io::Write;
    use std::process::{Command, Stdio};

    if !file.exists() {
        eprintln!("xtask emit: no such file: {}", file.display());
        std::process::exit(1);
    }
    let tools = build_tools(paths, profile);
    let out = out.unwrap_or_else(|| file.with_extension("wasm"));

    // cdz-syntax convert <file> | rcdzc - -o <out>.
    let syntax = Command::new(&tools.syntax)
        .args(["convert", "--from", from, "--to", "binary"])
        .arg(file)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| launch_fail("cdz-syntax", e));
    let ast = syntax.wait_with_output().expect("wait cdz-syntax");
    if !ast.status.success() {
        std::process::exit(ast.status.code().unwrap_or(1));
    }
    let mut rcdzc = Command::new(&tools.rcdzc)
        .args(["-", "-o"])
        .arg(&out)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| launch_fail("rcdzc", e));
    rcdzc.stdin.take().unwrap().write_all(&ast.stdout).ok();
    let status = rcdzc.wait().expect("wait rcdzc");
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    println!("wrote {}", out.display());
}

/// SHA-256 of the bytes, lowercase hex (the recorded hashing choice).
pub(crate) fn content_address(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// CANONICALIZE a built runtime component for content-addressing: strip ALL custom sections
/// (`wasm-tools strip -a`), which removes the non-deterministic `producers` sections that embed
/// tool-version strings — `rustc 1.95.0 (<commit>)`, `wit-component X.Y`, `cargo-component X.Y`,
/// `clang …`, `wit-bindgen …`. Those version strings differ across machines/toolchains, so hashing the
/// RAW build made `REQUIRED_RUNTIME_HASH` machine-specific (a program pinned one hash on box A that box
/// B's rebuild never reproduced). Stripping them makes the hash reproducible for a given rustc RELEASE
/// (a residual `/rustc/<commit>/…/raw_vec/mod.rs` panic-location string still lives in a data segment —
/// program data, not a custom section, so `strip` can't remove it; it is identical for the same rustc
/// commit, so it does not break reproducibility across machines sharing a toolchain). Both the stored
/// artifact AND the hash use the stripped bytes, so a composed program's imported hash matches the
/// stored file. Requires `wasm-tools` on PATH (`cargo install wasm-tools`); a missing binary is a hard
/// error (the hash would otherwise silently regress to the non-reproducible raw form).
pub(crate) fn canonicalize_runtime(raw_wasm_path: &Path) -> Vec<u8> {
    let out = raw_wasm_path.with_extension("stripped.wasm");
    let status = std::process::Command::new("wasm-tools")
        .arg("strip")
        .arg("-a")
        .arg(raw_wasm_path)
        .arg("-o")
        .arg(&out)
        .status();
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!(
                "wasm-tools strip failed ({s}) on {} — cannot canonicalize the runtime",
                raw_wasm_path.display()
            );
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!(
                "wasm-tools not runnable ({e}); install it with `cargo install wasm-tools` so the \
                 runtime hash is reproducible (it strips the tool-version `producers` sections)"
            );
            std::process::exit(1);
        }
    }
    let bytes = std::fs::read(&out)
        .unwrap_or_else(|e| panic!("read stripped runtime {}: {e}", out.display()));
    let _ = std::fs::remove_file(&out);
    bytes
}

/// `cargo component build --release --target wasm32-unknown-unknown` in <seed>/crates/<crate>,
/// returning the produced .wasm path. `cmd!` runs the child in the pushed crate dir and returns an
/// `Err` on a non-zero exit (already echoing the command), so a build failure surfaces cleanly.
pub(crate) fn build_component(sh: &Shell, seed: &Path, crate_dir: &str, artifact: &str) -> PathBuf {
    build_component_with_features(sh, seed, crate_dir, artifact, &[])
}

/// As [`build_component`], but with a list of cargo `--features` to enable — e.g. the runtime's
/// `debug-counters` (its `live-objects` leak counter). A feature-flagged build writes to the SAME
/// `target/.../<artifact>.wasm` path as the default, so a caller that needs BOTH must read each build's
/// bytes before the next overwrites (see `codegen`'s two-build hashing). Same crate dir, target, and
/// error handling as the default build.
pub(crate) fn build_component_with_features(
    sh: &Shell,
    seed: &Path,
    crate_dir: &str,
    artifact: &str,
    features: &[&str],
) -> PathBuf {
    let dir = seed.join("crates").join(crate_dir);
    let _pushed = sh.push_dir(&dir);
    let build = if features.is_empty() {
        cmd!(
            sh,
            "cargo component build --release --target wasm32-unknown-unknown"
        )
    } else {
        let feats = features.join(",");
        cmd!(
            sh,
            "cargo component build --release --target wasm32-unknown-unknown --features {feats}"
        )
    };
    if let Err(e) = build.run() {
        eprintln!("cargo component build failed for {crate_dir}: {e}");
        std::process::exit(1);
    }
    dir.join(format!(
        "target/wasm32-unknown-unknown/release/{artifact}.wasm"
    ))
}
