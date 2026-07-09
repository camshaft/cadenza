//! Build choreography for the compiler↔runtime versioned pair (Amendment 0.6.0).
//!
//! Enforces the build-pair invariant (reproducible-derivation.md §Derivation Is A Function Of
//! Source And Toolchain): the runtime is derived FIRST, its content address computed, and the
//! compiler built against that address — so a compiler and the runtime it emits programs against
//! are never independently versioned. Both artifacts land in a content-addressed store keyed by
//! their SHA-256 (the recorded hashing choice, options/hashing-and-encoding/), which is what the
//! host resolves a program's required runtime against.
//!
//! Usage: `cargo run -p xtask -- build [--store <dir>]`
//!   Default store: `implementation/seed/target/cadenza-store`.

mod frame;
mod opcodes;
mod wit_envelope;

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Write `contents` to `path` ONLY if it differs from what is already there — the Membrain
/// `core-codegen` pattern. A no-op regeneration must not bump the file's mtime, or cargo recompiles
/// the compiler on every `xtask build` and the incremental cache is defeated. Shared by every code
/// generator (the WIT envelope, the opcode table).
pub(crate) fn write_if_changed(path: &Path, contents: &str) -> std::io::Result<bool> {
    if let Ok(existing) = std::fs::read(path) {
        if existing == contents.as_bytes() {
            return Ok(false);
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;
    Ok(true)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("build");
    // Optional `--store <dir>`.
    let store = args
        .iter()
        .position(|a| a == "--store")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);

    match cmd {
        "build" => build(store),
        // Generate the envelope sources ONLY (skip building the runtime component) — used when the
        // runtime component cannot build yet (e.g. unimplemented ops mid-development) but the envelope
        // must be regenerated from the WIT. Uses a placeholder hash; `build` bakes the real one.
        "gen-only" => {
            let seed = seed_root();
            match wit_envelope::generate(&seed, "0000000000000000000000000000000000000000000000000000000000000000") {
                Ok(changed) => println!("gen-only: envelope generated (changed={changed})"),
                Err(e) => {
                    eprintln!("envelope generation failed: {e}");
                    std::process::exit(1);
                }
            }
            // The opcode table (op.rs + cdzc/op.cdz) is independent of the runtime hash, so it is safe to
            // regenerate under gen-only too — keeps the Cadenza compiler's shared tables current without a
            // full component build.
            let repo = seed
                .parent()
                .and_then(|p| p.parent())
                .expect("seed is <repo>/implementation/seed")
                .to_path_buf();
            match opcodes::generate(&seed, &repo) {
                Ok(changed) => println!("gen-only: opcode table generated (changed={changed})"),
                Err(e) => {
                    eprintln!("opcode generation failed: {e}");
                    std::process::exit(1);
                }
            }
            match frame::generate(&seed, &repo) {
                Ok(changed) => println!("gen-only: cdzc frame segments generated (changed={changed})"),
                Err(e) => {
                    eprintln!("frame generation failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        other => {
            eprintln!("unknown xtask: {other}");
            eprintln!("usage: cargo run -p xtask -- build [--store <dir>]");
            std::process::exit(2);
        }
    }
}

/// The seed root (this crate lives at <seed>/xtask), resolved from CARGO_MANIFEST_DIR.
fn seed_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .expect("xtask has a parent dir")
        .to_path_buf()
}

fn build(store: Option<PathBuf>) {
    let seed = seed_root();
    let store = store.unwrap_or_else(|| seed.join("target/cadenza-store"));
    std::fs::create_dir_all(&store).expect("create store dir");

    // ── Step 1: build the runtime component (wasm32). ──
    println!("== xtask: building the value-heap runtime component ==");
    let runtime_wasm = build_component(&seed, "cdz-runtime", "cdz_runtime");

    // ── Step 2: content address of the runtime. ──
    let runtime_bytes = std::fs::read(&runtime_wasm).expect("read runtime wasm");
    let runtime_hash = content_address(&runtime_bytes);
    println!("   runtime content address: {runtime_hash}");
    let runtime_stored = store.join(format!("{runtime_hash}.wasm"));
    std::fs::write(&runtime_stored, &runtime_bytes).expect("store runtime");
    println!("   stored → {}", runtime_stored.display());

    // ── Step 3: GENERATE the compiler's envelope sources from the runtime WIT + this hash. ──
    // The runtime interface has one source of truth — the runtime's WIT — and the compiler's view of
    // it (the component-model envelope byte-chunks, import indices, core signatures, and the
    // required-runtime pin) is DERIVED from that contract here, never hand-maintained. Baking the
    // hash into the generated source is what records each emitted program's required runtime
    // (component-abi.md §The Emitted Component Records Its Required Runtime) — no build-time env var.
    // Write-if-changed: an unchanged contract leaves the generated files (and their mtimes) alone, so
    // the compiler's incremental cache survives a no-op `build`.
    println!(
        "== xtask: generating the compiler envelope from the runtime WIT (pin {runtime_hash}) =="
    );
    match wit_envelope::generate(&seed, &runtime_hash) {
        Ok(true) => println!("   generated sources changed → cdz-compiler will recompile"),
        Ok(false) => println!("   generated sources unchanged (cache preserved)"),
        Err(e) => {
            eprintln!("envelope generation failed: {e}");
            std::process::exit(1);
        }
    }

    // ── Step 3b: GENERATE the wasm opcode table into BOTH compilers. ──
    // The opcode bytes are the WebAssembly spec's, derived by encoding `wasm_encoder::Instruction`s
    // (the authoritative source), and emitted as both the Rust seed's `op.rs` and the Cadenza
    // compiler's `op.cdz` — so the two implementations share one opcode table and can never disagree
    // on a byte. Independent of the runtime hash, but same generate-then-build discipline.
    let repo = seed
        .parent()
        .and_then(|p| p.parent())
        .expect("seed is <repo>/implementation/seed")
        .to_path_buf();
    println!("== xtask: generating the wasm opcode table (op.rs + op.cdz) ==");
    match opcodes::generate(&seed, &repo) {
        Ok(true) => println!("   opcode table changed → cdz-compiler will recompile"),
        Ok(false) => println!("   opcode table unchanged (cache preserved)"),
        Err(e) => {
            eprintln!("opcode generation failed: {e}");
            std::process::exit(1);
        }
    }

    // ── Step 3c: GENERATE the cdzc scalar-frame byte segments (wasm-encoder) into cdzc/40-frame.cdz. ──
    println!("== xtask: generating the cdzc frame segments (cdzc/40-frame.cdz) ==");
    match frame::generate(&seed, &repo) {
        Ok(true) => println!("   frame segments changed"),
        Ok(false) => println!("   frame segments unchanged (cache preserved)"),
        Err(e) => {
            eprintln!("frame generation failed: {e}");
            std::process::exit(1);
        }
    }

    // ── Step 4: build the compiler component against the freshly-generated sources. ──
    println!("== xtask: building the compiler component pinned to runtime {runtime_hash} ==");
    let compiler_wasm = build_component(&seed, "cdz-compiler-component", "cdz_compiler_component");

    // ── Step 5: content address of the compiler; store it too. ──
    let compiler_bytes = std::fs::read(&compiler_wasm).expect("read compiler wasm");
    let compiler_hash = content_address(&compiler_bytes);
    println!("   compiler content address: {compiler_hash}");
    let compiler_stored = store.join(format!("{compiler_hash}.wasm"));
    std::fs::write(&compiler_stored, &compiler_bytes).expect("store compiler");
    println!("   stored → {}", compiler_stored.display());

    // A small manifest recording the versioned pair, for the host / verifier to consult.
    let manifest = format!(
        "# Cadenza content-addressed store — the compiler↔runtime versioned pair.\n\
         runtime = \"{runtime_hash}\"\n\
         compiler = \"{compiler_hash}\"\n"
    );
    std::fs::write(store.join("pair.toml"), manifest).expect("write pair.toml");

    println!("\n== xtask: done ==");
    println!("   store:    {}", store.display());
    println!("   runtime:  {runtime_hash}");
    println!("   compiler: {compiler_hash}  (pinned to the runtime above)");
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
/// returning the produced .wasm path.
fn build_component(seed: &Path, crate_dir: &str, artifact: &str) -> PathBuf {
    let dir = seed.join("crates").join(crate_dir);
    let mut cmd = Command::new("cargo");
    cmd.current_dir(&dir).args([
        "component",
        "build",
        "--release",
        "--target",
        "wasm32-unknown-unknown",
    ]);
    let status = cmd.status().expect("run cargo component build");
    if !status.success() {
        eprintln!("cargo component build failed for {crate_dir}");
        std::process::exit(1);
    }
    dir.join(format!(
        "target/wasm32-unknown-unknown/release/{artifact}.wasm"
    ))
}
