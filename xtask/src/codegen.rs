//! `codegen` — generate the wasm backend's two ABI tables from their authoritative oracles.
//!
//! The wasm backend emits bytes against two frozen ABIs. Rather than hand-transcribing either (a
//! hard-coded list that could silently drift from its source), each is DERIVED from its oracle and
//! written as a plain-data Rust file the backend consumes. Both generated files are PLAIN DATA — no
//! external dependency — so they ship in the portable compiler; the oracle crates (`wit-parser`,
//! `wasm-encoder`) live ONLY here in xtask (a dev desk). Re-run `cargo xtask codegen` after changing
//! either source; `cargo xtask codegen --check` (a hard gate in `xtask check`) fails if a committed
//! file has drifted from its oracle.
//!
//!  - `runtime_abi.rs` — the VALUE-HEAP RUNTIME interface, declared once in the runtime crate's
//!    `wit/runtime.wit` (the ABI's source of truth). Read with `wit-parser` into one `RtOp { name,
//!    params, result }` per declared op, so the compiler builds a program's per-program import
//!    section from structured signature data (importing only the ops it uses) rather than pasting
//!    opaque envelope blobs. Also carries the runtime's content hash, so a runtime-code change (not
//!    only a WIT change) is caught by the staleness gate.
//!
//!  - `wasm_abi.rs` — every WASM / COMPONENT-MODEL byte the backend lays down: opcodes, core and
//!    component valtypes, section ids, the two magic headers, and the functype form bytes. Each is
//!    EXTRACTED FROM `wasm-encoder` (the spec byte encoder) by encoding a one-off value and reading
//!    the byte back — so no opcode or magic number is hand-written. `wasm-encoder` is already the
//!    byte ORACLE the rcdzc tests diff the hand-emitted bytes against; this makes it the SOURCE of
//!    those bytes too, not just the after-the-fact check.
//!
//! (The built-in CONTRACT schemas `cdz-platform/src/contracts/*.rs` USED to be generated here too, but
//! were carved out into the standalone `xtask-codegen-contracts` bin + the `cdzPlatformContracts` nix
//! derivation — the operator codegen→build-time-nix directive; `cdzPlatformContractsMatch` guards their
//! byte-identity now, so `run` no longer touches contracts.)

use crate::{Paths, build_component_with_features, content_address};
use proc_macro2::{Span, TokenStream};
use quote::quote;
use std::path::PathBuf;
use wit_parser::{Resolve, Type as WitType};
use xshell::Shell;

/// The LOGICAL (component-model) value type of a runtime op's param/result, as the WIT declares it.
/// The runtime interface uses only these four; a richer WIT type (string, list, record) is NOT a bare
/// scalar and is reported so the ABI cannot silently admit one. Kept LOGICAL (not collapsed to a core
/// valtype) because the backend needs BOTH projections and they are not recoverable from each other: a
/// core `i32` is the lowering of `u32`, `bool` AND `s32` alike, so the component import instance-type —
/// which must structurally match the runtime's exported type — cannot be rebuilt from the core byte.
/// One WIT source, two read-offs live in the backend (the byte encodings are wasm-spec constants, a
/// TARGET concern, like the opcode table in `encode.rs`).
#[derive(Clone, Copy)]
enum AbiTy {
    U32,  // a heap handle or a small unsigned scalar
    S32,  // a signed 32-bit result (e.g. `value-cmp`'s three-way -1/0/1 order); lowers to core i32
    S64,  // a boxed signed 64-bit integer
    Bool, // a boxed boolean
    F64,  // a boxed Float64
    F32,  // a boxed Float32 (box-float32/get-float32 — a Float32 lives in an f32 machine slot)
}

impl AbiTy {
    /// Map a WIT type to its logical ABI type, or `None` if it is not a bare scalar (e.g. `string`,
    /// `tuple` — the runtime's `str-*` and `vec-split` results, which the envelope does not lower).
    fn from_wit(t: WitType) -> Option<AbiTy> {
        match t {
            WitType::U32 => Some(AbiTy::U32),
            WitType::S32 => Some(AbiTy::S32),
            WitType::Bool => Some(AbiTy::Bool),
            WitType::S64 => Some(AbiTy::S64),
            WitType::F64 => Some(AbiTy::F64),
            WitType::F32 => Some(AbiTy::F32),
            _ => None,
        }
    }

    /// The generated `AbiValType::<variant>` path as tokens (the backend's own enum, defined alongside
    /// the table), for splicing into a `quote!`.
    fn variant_tokens(self) -> TokenStream {
        match self {
            AbiTy::U32 => quote!(AbiValType::U32),
            AbiTy::S32 => quote!(AbiValType::S32),
            AbiTy::S64 => quote!(AbiValType::S64),
            AbiTy::Bool => quote!(AbiValType::Bool),
            AbiTy::F64 => quote!(AbiValType::F64),
            AbiTy::F32 => quote!(AbiValType::F32),
        }
    }
}

/// One runtime op resolved from the WIT: its name and logical signature. A param or result the envelope
/// cannot lower (a non-scalar WIT type) marks the op UNLOWERABLE — it is still listed (so the table
/// mirrors the full interface) but flagged, and the backend never selects it.
struct Op {
    name: String,
    params: Vec<AbiTy>,
    result: Option<AbiTy>,
    /// `false` when a param or result is a non-scalar WIT type (`string`, `tuple`) — the op
    /// exists in the interface but the bare-core-signature envelope cannot import it.
    lowerable: bool,
}

/// Generate BOTH backend ABI tables — `runtime_abi.rs` (from the runtime WIT) and `wasm_abi.rs` (from
/// `wasm-encoder`). In `check` mode, regenerate each in memory and compare to the committed file
/// WITHOUT writing — the STALENESS GATE: exit non-zero if either is out of date, so a forgotten
/// regeneration fails `xtask check` rather than silently drifting from its oracle.
pub fn run(paths: &Paths, check: bool) {
    generate_runtime_abi(paths, check);
    generate_wasm_abi(paths, check);
}

/// Generate `runtime_abi.rs` from the runtime WIT (see the `runtime_abi` bullet in the module doc).
fn generate_runtime_abi(paths: &Paths, check: bool) {
    let wit = paths.seed.join("crates/cdz-runtime/wit/runtime.wit");
    // The runtime world imports `cadenza:nfc/normalize` (FINDING#23), so the NFC package must be in the
    // resolve before the runtime WIT parses — push it first (its authoritative WIT is the cdz-nfc crate).
    let nfc_wit = paths.seed.join("crates/cdz-nfc/wit/nfc.wit");
    let out = paths
        .seed
        .join("crates/rcdzc/src/backend/wasm/runtime_abi.rs");

    let ops = match resolve_ops(&wit, &nfc_wit) {
        Ok(ops) => ops,
        Err(e) => {
            eprintln!("xtask codegen: {e}");
            std::process::exit(1);
        }
    };
    let iface = "cadenza:runtime/heap";
    // The runtime's CONTENT HASHES — built + content-addressed the same way `xtask build` does, so the
    // generated table changes whenever the runtime BINARY changes (not only its WIT). This is what
    // makes staleness automatic across a runtime-code change: rebuild → new hash → `runtime_abi.rs`
    // differs → `codegen --check` fails until regenerated. TWO builds are hashed: the RELEASE runtime
    // (what a shipped program pins + composes) and the DEBUG-COUNTERS runtime (the same code with the
    // `live-objects` leak counter compiled in — the Perceus balance probe composes THIS one). Both are
    // recorded so a program can require the release runtime by hash AND a leak-check harness can locate
    // the debug runtime by hash, neither hard-coded. (`xtask check` already builds the runtime, so the
    // release build's cost is shared; the debug build is one extra `cargo component` invocation.)
    // The NFC component's content hash — built + content-addressed the SAME way the runtime is (build →
    // canonicalize → content_address), so the generated `REQUIRED_NFC_HASH` tracks an NFC-component change
    // automatically, exactly like `REQUIRED_RUNTIME_HASH`. Computed FIRST because it is stamped INLINE into
    // each heap's `cadenza:nfc/normalize` import (`stamp_nfc_into_heap`), so the RECORDED
    // `REQUIRED_RUNTIME_HASH` must be the hash of the STAMPED heap (matching what `build` stores + what a
    // program pins). NFC lives in a separate imported component so the core runtime stays light (FINDING#23);
    // its address now rides inline in the heap's import (operator directive 2026-08-23: self-describing
    // imports, no runtime.toml mapping). One extra `cargo component` build.
    let nfc_hash = build_nfc_hash(paths);
    let (runtime_hash, debug_runtime_hash, imm_unit) = build_runtime_hashes(paths, &nfc_hash);
    // Build the body as tokens (`render`), pretty-print + rustfmt it (`format_tokens`), then prepend the
    // `//!` module banner as text (a module doc is awkward as a token attribute). prettyplease-then-
    // rustfmt makes the committed file agree with BOTH `fmt --check` and `codegen --check`.
    let body = format_tokens(render(
        &ops,
        iface,
        &runtime_hash,
        &debug_runtime_hash,
        &nfc_hash,
        imm_unit,
    ));
    let source = format!("{}{body}", runtime_abi_banner());

    let summary = format!(
        "{} ops, {} lowerable, from {}",
        ops.len(),
        ops.iter().filter(|o| o.lowerable).count(),
        wit.display()
    );
    emit_or_check(&out, &source, check, "the runtime WIT", &summary);
}

/// Generate `wasm_abi.rs` from `wasm-encoder` (see the `wasm_abi` bullet in the module doc). The
/// byte values are extracted, not typed in: `wasm_abi::collect` encodes a one-off value with
/// `wasm-encoder` for each entry and reads the emitted byte back.
fn generate_wasm_abi(paths: &Paths, check: bool) {
    let out = paths.seed.join("crates/rcdzc/src/backend/wasm/wasm_abi.rs");
    let tables = wasm_abi::collect();
    let source = format!(
        "{}{}",
        wasm_abi_banner(),
        format_tokens(wasm_abi::render(&tables))
    );
    let summary = format!(
        "{} opcodes + {} valtype/section/form bytes, from wasm-encoder",
        tables.opcodes.len(),
        tables.singles.len()
    );
    emit_or_check(
        &out,
        &source,
        check,
        "the wasm-encoder byte encoder",
        &summary,
    );
}

/// Emit `source` to `out`, or (in `check` mode) compare without writing and fail on drift. Shared by
/// both generators — one place for the "write vs. staleness-gate" behavior and its guidance message.
/// `oracle` names the source of truth for the failure text; `summary` is the wrote-what line.
fn emit_or_check(out: &PathBuf, source: &str, check: bool, oracle: &str, summary: &str) {
    if check {
        // Compare, don't write. A mismatch (or a missing file) means the committed table is behind
        // its oracle — a hard failure with the fix spelled out, so no one has to REMEMBER to regen.
        let current = std::fs::read_to_string(out).unwrap_or_default();
        if current != source {
            eprintln!(
                "xtask codegen --check: {} is OUT OF DATE with {oracle}.\n  \
                 The source changed but the generated table was not regenerated.\n  \
                 Fix: run `cargo xtask codegen` and commit {}.",
                out.display(),
                out.display()
            );
            std::process::exit(1);
        }
        println!("xtask codegen --check: {} is up to date.", out.display());
        return;
    }
    if let Err(e) = std::fs::write(out, source) {
        eprintln!("xtask codegen: writing {}: {e}", out.display());
        std::process::exit(1);
    }
    println!("xtask codegen: wrote {} ({summary})", out.display());
}

/// Build BOTH runtime components and return their content addresses `(release, debug_counters)` —
/// the SAME BLAKE3 derivation `xtask build` uses to key the store (operator 2026-08-08 blake3
/// unification). Embedding these ties the compiler's
/// recorded hashes to the ACTUAL runtime bytes: a runtime-code change (even with an unchanged WIT)
/// changes a hash, hence the generated file, hence `codegen --check` fails until regenerated. Both
/// builds write the SAME `target/.../cdz_runtime.wasm` path, so each build's bytes are read IMMEDIATELY,
/// before the next overwrites. Exits on a build failure (a hash cannot be honestly recorded without the
/// built artifact).
/// R3 (nix codegen-consumes-nix-bytes, opt-in): the env flag that switches codegen from SELF-BUILDING the
/// runtime/nfc components (via `cargo component`, 3× runtime + 1× nfc) to CONSUMING the bytes the nix
/// derivations already build (`packages.runtime{,-raw,-debug}` + `packages.nfc`), removing the duplicate
/// build. Default (unset) keeps the self-build — a hard swap is forbidden by the operator no-cutover rule,
/// so this lands opt-in + parallel-proven (the nix path must produce the IDENTICAL hashes; see the
/// `codegen_nix_consume_matches_self_build` parity test) before the default ever flips.
const CODEGEN_FROM_NIX_ENV: &str = "CDZ_CODEGEN_FROM_NIX";

/// The `nix` binary to invoke — PATH first (honor a custom/newer nix), else the standard multi-user
/// profile location. Mirrors `fleet::nix_binary` (kept local to avoid cross-module exposure of a private
/// helper; both resolve identically).
fn codegen_nix_binary() -> String {
    if std::process::Command::new("nix")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return "nix".to_string();
    }
    const PROFILE_NIX: &str = "/nix/var/nix/profiles/default/bin/nix";
    if std::path::Path::new(PROFILE_NIX).exists() {
        return PROFILE_NIX.to_string();
    }
    "nix".to_string()
}

/// `nix build .#<attr> --no-link --print-out-paths` → the built store path. Panics on a build/eval
/// failure or empty output (codegen cannot honestly record a hash without the artifact — same
/// exit-on-failure contract as the self-build path). The attr is a flake output name (e.g. `runtime`,
/// `runtime-raw`, `runtime-debug`, `nfc`).
fn nix_build_out_path(attr: &str) -> std::path::PathBuf {
    let nix = codegen_nix_binary();
    let out = std::process::Command::new(&nix)
        .args([
            "build",
            &format!(".#{attr}"),
            "--no-link",
            "--print-out-paths",
        ])
        .output()
        .unwrap_or_else(|e| panic!("xtask codegen: could not invoke `{nix} build .#{attr}`: {e}"));
    if !out.status.success() {
        panic!(
            "xtask codegen: `nix build .#{attr}` failed ({}):\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if path.is_empty() {
        panic!("xtask codegen: `nix build .#{attr}` produced no output path");
    }
    std::path::PathBuf::from(path)
}

/// The content-addressed store a contract's `cdz test` resolves the value-heap runtime from: `CDZ_STORE` if
/// set (the CI/harness override), else the repo default `target/cadenza-store` (matching `cdz-run`'s default
/// and what `cargo xtask build` seeds).
fn codegen_store_dir(paths: &Paths) -> std::path::PathBuf {
    std::env::var_os("CDZ_STORE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| paths.repo.join("target/cadenza-store"))
}

/// Seed one canonicalized component (the value-heap runtime, its debug build, or NFC) into the store at
/// `<store>/<content-address>.wasm`, so a subsequent `cdz test`/`cdz run` resolves it by content address.
/// codegen already builds + canonicalizes these to hash them (`build_runtime_hashes` / `build_nfc_hash`);
/// writing them here makes `codegen` self-sufficient for the contract `@test`s in a bare CI job that never
/// ran `cargo xtask build`. Idempotent (content-addressed): a re-write is the same bytes at the same path.
fn seed_store_component(paths: &Paths, bytes: &[u8]) {
    let store = codegen_store_dir(paths);
    if let Err(e) = std::fs::create_dir_all(&store) {
        eprintln!("xtask codegen: create store dir {}: {e}", store.display());
        std::process::exit(1);
    }
    let path = store.join(format!("{}.wasm", content_address(bytes)));
    if let Err(e) = std::fs::write(&path, bytes) {
        eprintln!("xtask codegen: seed runtime store {}: {e}", path.display());
        std::process::exit(1);
    }
}

/// R3 consumer: derive the runtime hashes by CONSUMING the nix-built outputs instead of self-building.
/// `.#runtime-raw` (pre-strip, carries `cdz-abi`) → `read_abi_imm_unit`; `.#runtime` (stripped/canonicalized
/// in-derivation, the shipped artifact) → `content_address` = REQUIRED_RUNTIME_HASH; `.#runtime-debug`
/// (stripped) → DEBUG_RUNTIME_HASH. The nix `runtime` output is ALREADY canonicalized (the derivation runs
/// `canonicalize_runtime`), so it is content-addressed DIRECTLY — no re-strip. Byte-equivalent to the
/// self-build path by construction (verified: `nix .#runtime` b3sum == the self-built canonicalized hash).
fn runtime_hashes_from_nix(paths: &Paths) -> (String, String, u32) {
    let raw_path = nix_build_out_path("runtime-raw");
    let imm_unit = read_abi_imm_unit(&raw_path);
    let release_path = nix_build_out_path("runtime");
    let release_bytes = std::fs::read(&release_path)
        .unwrap_or_else(|e| panic!("xtask codegen: read nix runtime {release_path:?}: {e}"));
    let release_hash = content_address(&release_bytes);
    let debug_path = nix_build_out_path("runtime-debug");
    let debug_bytes = std::fs::read(&debug_path)
        .unwrap_or_else(|e| panic!("xtask codegen: read nix runtime-debug {debug_path:?}: {e}"));
    let debug_hash = content_address(&debug_bytes);
    // Seed the store so a contract `cdz test` resolves the runtime by content address.
    seed_store_component(paths, &release_bytes);
    seed_store_component(paths, &debug_bytes);
    (release_hash, debug_hash, imm_unit)
}

fn build_runtime_hashes(paths: &Paths, nfc_hash: &str) -> (String, String, u32) {
    // R3 (opt-in): consume the nix-built runtime bytes instead of self-building (removes the duplicate
    // 3× runtime + gate-build rebuild). The nix `.#runtime` derivation stamps the NFC import in-derivation
    // (same as `build`), so the consumed bytes are already stamped — `nfc_hash` is unused on this path.
    if std::env::var_os(CODEGEN_FROM_NIX_ENV).is_some() {
        return runtime_hashes_from_nix(paths);
    }
    let sh = Shell::new().expect("open a shell");
    // Release runtime — what a shipped program pins and composes. STAMP the NFC address inline into the
    // heap's import, then CANONICALIZE (strip the tool-version `producers` sections) before hashing —
    // EXACTLY as `build` does when it stores the artifact — so the committed `REQUIRED_RUNTIME_HASH` is the
    // hash of the STAMPED+stripped heap, matching the stored file + what a program pins. (See
    // `crate::stamp_nfc_into_heap` / `crate::canonicalize_runtime`.)
    let release_wasm =
        build_component_with_features(&sh, &paths.seed, "cdz-runtime", "cdz_runtime", &[]);
    // Read the ABI immediate encodings from the RAW build's `cdz-abi` custom section BEFORE stamp/canonicalize
    // strip all custom sections — so the derived constant costs zero bytes in the shipped/hashed runtime
    // (the stamped+stripped hash is unchanged by the section's presence). See `read_abi_imm_unit`.
    let imm_unit = read_abi_imm_unit(&release_wasm);
    let release_stamped = crate::stamp_nfc_into_heap(&paths.repo, &release_wasm, nfc_hash);
    let release_bytes = crate::canonicalize_runtime(&release_stamped);
    let release_hash = content_address(&release_bytes);
    // Seed the store so a contract `cdz test` resolves the runtime by content address (this build path is
    // the one the bare CI codegen job takes; it never ran `cargo xtask build`).
    seed_store_component(paths, &release_bytes);
    // Debug-counters runtime — the same code with the `live-objects` leak counter compiled in.
    let debug_wasm = build_component_with_features(
        &sh,
        &paths.seed,
        "cdz-runtime",
        "cdz_runtime",
        &["debug-counters"],
    );
    let debug_stamped = crate::stamp_nfc_into_heap(&paths.repo, &debug_wasm, nfc_hash);
    let debug_bytes = crate::canonicalize_runtime(&debug_stamped);
    let debug_hash = content_address(&debug_bytes);
    seed_store_component(paths, &debug_bytes);
    // Leave the RELEASE runtime as the artifact at the shared path (rebuild it last), so a plain
    // `cargo component build` output on disk after codegen is the release one — the default a naive
    // reader expects and the composed tests hash-match against.
    let _ = build_component_with_features(&sh, &paths.seed, "cdz-runtime", "cdz_runtime", &[]);
    (release_hash, debug_hash, imm_unit)
}

/// Build the NFC component (`cdz-nfc`) and return its content hash — the same build → canonicalize →
/// content-address pipeline `build_runtime_hashes` uses for the runtime, so `REQUIRED_NFC_HASH` tracks an
/// NFC-component change automatically (rebuild → new hash → `runtime_abi.rs` differs → `codegen --check`
/// fails until regenerated). FINDING#23 / operator ruling (d): NFC (the heavy `unicode-normalization`
/// tables) lives in this SEPARATE component the emitted program imports BY HASH, so the tagless core
/// runtime carries none of it and its own hash is unaffected. `canonicalize_runtime` strips the tool-version
/// `producers` sections so the hash is reproducible across hosts (identical to the artifact `build` stores).
fn build_nfc_hash(paths: &Paths) -> String {
    // R3 (opt-in): consume the nix-built NFC bytes. `.#nfc` is stripped/canonicalized in-derivation (like
    // `.#runtime`) and carries no custom section codegen reads, so it is content-addressed DIRECTLY.
    if std::env::var_os(CODEGEN_FROM_NIX_ENV).is_some() {
        let nfc_path = nix_build_out_path("nfc");
        let nfc_bytes = std::fs::read(&nfc_path)
            .unwrap_or_else(|e| panic!("xtask codegen: read nix nfc {nfc_path:?}: {e}"));
        // A heap-value `cdz test` composes the heap's NFC import by hash, so NFC must be in the store too.
        seed_store_component(paths, &nfc_bytes);
        return content_address(&nfc_bytes);
    }
    let sh = Shell::new().expect("open a shell");
    let nfc_wasm = build_component_with_features(&sh, &paths.seed, "cdz-nfc", "cdz_nfc", &[]);
    let nfc_bytes = crate::canonicalize_runtime(&nfc_wasm);
    // Seed NFC into the store: a heap-value `cdz test` composes the heap's NFC import by hash.
    seed_store_component(paths, &nfc_bytes);
    content_address(&nfc_bytes)
}

/// Read the inline-unit handle bits from the runtime's `cdz-abi` CUSTOM SECTION (a little-endian `u32`).
/// The runtime declares it via `#[link_section = "cdz-abi"]` so this is a STATIC read — no execution —
/// and it is read from the RAW build before `strip -a` removes it (hence zero shipped-byte cost). The
/// component wraps a core module; `wasmparser` yields custom sections at BOTH the component and the
/// nested-module level, so this scans every `CustomSection` payload for the one named `cdz-abi`.
/// Panics if the section is absent or not 4 bytes — a codegen invariant (the runtime must declare it).
fn read_abi_imm_unit(wasm_path: &std::path::Path) -> u32 {
    let bytes = std::fs::read(wasm_path)
        .unwrap_or_else(|e| panic!("xtask codegen: cannot read runtime wasm {wasm_path:?}: {e}"));
    for payload in wasmparser::Parser::new(0).parse_all(&bytes) {
        if let Ok(wasmparser::Payload::CustomSection(c)) = payload
            && c.name() == "cdz-abi"
        {
            let d = c.data();
            let arr: [u8; 4] = d.try_into().unwrap_or_else(|_| {
                panic!(
                    "xtask codegen: `cdz-abi` section is {} bytes, expected 4",
                    d.len()
                )
            });
            return u32::from_le_bytes(arr);
        }
    }
    panic!(
        "xtask codegen: the runtime wasm has no `cdz-abi` custom section — the runtime must declare \
         the ABI immediate encodings (see `CDZ_ABI_IMM_UNIT` in cdz-runtime)"
    );
}

/// Format a generated `TokenStream` to the committed file's text: `prettyplease` FIRST (deterministic
/// pretty-print from the AST — quote! emits everything on effectively one line, and prettyplease breaks
/// it into readable structure without choking on length), THEN `rustfmt` (the SAME formatter the
/// `fmt --check` gate runs, so its output is stable under that gate — this is what makes `fmt --check`
/// and `codegen --check` agree by construction). rustfmt is best-effort: if it is not on PATH the
/// prettyplease output ships (valid Rust; only `fmt --check` might then re-wrap a line).
fn format_tokens(tokens: proc_macro2::TokenStream) -> String {
    let file = syn::parse2::<syn::File>(tokens)
        .unwrap_or_else(|e| panic!("xtask codegen: generated tokens did not parse (a bug): {e}"));
    let pretty = prettyplease::unparse(&file);
    // rustfmt is REQUIRED for byte-identical output: prettyplease alone diverges from the committed
    // cargo-fmt'd line-wrapping, so a rustfmt-less run would silently emit MIS-FORMATTED source — and
    // `codegen --check` would then compare against it and either false-pass or false-fail. Hard-error
    // rather than fall back (v-nix caught the silent-fallback wiring the codegen-contracts derivation).
    rustfmt_stdin(&pretty).unwrap_or_else(|| {
        eprintln!(
            "xtask codegen: `rustfmt` is required on PATH (prettyplease alone diverges from the committed \
             cargo-fmt'd form → mis-formatted output). Install the pinned toolchain's rustfmt."
        );
        std::process::exit(1);
    })
}

/// Run `src` through the `rustfmt` binary (stdin→stdout). `None` if rustfmt is unavailable or errors.
fn rustfmt_stdin(src: &str) -> Option<String> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(src.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Resolve every function of the runtime's `heap` interface from the WIT, SORTED BY NAME — the full op
/// vocabulary as structured data. (The compiler resolves imports by name, so the order is informational;
/// the ops are sorted by name for a stable, deterministic emission independent of WIT declaration order.)
fn resolve_ops(
    wit_path: &std::path::Path,
    nfc_wit_path: &std::path::Path,
) -> Result<Vec<Op>, String> {
    let mut resolve = Resolve::default();
    // Push the NFC package first so the runtime world's `import cadenza:nfc/normalize` resolves (FINDING#23:
    // NFC is the runtime's component dependency). Only the runtime's `heap` interface functions are read
    // below; the NFC package just needs to EXIST in the resolve for the import to type.
    resolve
        .push_file(nfc_wit_path)
        .map_err(|e| format!("parse {}: {e}", nfc_wit_path.display()))?;
    let pkg = resolve
        .push_file(wit_path)
        .map_err(|e| format!("parse {}: {e}", wit_path.display()))?;
    let iface_id = resolve.packages[pkg]
        .interfaces
        .iter()
        .find(|(name, _)| name.as_str() == "heap")
        .map(|(_, id)| *id)
        .ok_or_else(|| "runtime WIT has no `heap` interface".to_string())?;
    let iface = &resolve.interfaces[iface_id];

    let mut ops = Vec::with_capacity(iface.functions.len());
    for f in iface.functions.values() {
        let mut lowerable = true;
        let mut params = Vec::with_capacity(f.params.len());
        for (_pname, ty) in &f.params {
            match AbiTy::from_wit(*ty) {
                Some(c) => params.push(c),
                None => lowerable = false,
            }
        }
        let result = match f.result {
            Some(ty) => match AbiTy::from_wit(ty) {
                Some(c) => Some(c),
                None => {
                    lowerable = false;
                    None
                }
            },
            None => None,
        };
        ops.push(Op {
            name: f.name.clone(),
            params,
            result,
            lowerable,
        });
    }
    // Sort by name — a deterministic, readable table order (the runtime resolves by name, so any
    // stable order is fine; alphabetical is the most diff-friendly).
    ops.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(ops)
}

/// Build the generated Rust source as a `TokenStream` with `quote!` — the `CoreValType` enum, the
/// `RtOp` struct, the `RUNTIME_OPS` table, the typed `OPS` accessor (a named field per op → its
/// `&RUNTIME_OPS[i]`), the interface name, and the runtime content hash. Building tokens (not strings)
/// reads like the emitted Rust and needs no manual escaping; `format_tokens` pretty-prints it. Doc
/// comments are `#[doc = …]` attributes (which render as `///`). A leading `//!`-style module banner
/// can't be a token attribute cleanly, so it is prepended as text by the caller.
/// A `&str` const initializer that prefers a COMPILE-TIME `CDZ_*_HASH` env override, falling back to the
/// committed `default`. `option_env!` is evaluated when the compiler crate is compiled, so a nix build that
/// exports the env bakes the content hash of the component it built in the same closure; a plain build (no
/// env) uses `default`. Written as a `match` rather than `option_env!(var).unwrap_or(default)` because
/// `Option::unwrap_or` is not a const fn — a `match` is the const-context-legal form.
fn env_or_default_hash(var: &str, default: &str) -> TokenStream {
    quote! {
        match option_env!(#var) {
            Some(h) => h,
            None => #default,
        }
    }
}

fn render(
    ops: &[Op],
    iface: &str,
    runtime_hash: &str,
    debug_runtime_hash: &str,
    nfc_hash: &str,
    imm_unit: u32,
) -> TokenStream {
    // The RUNTIME_OPS rows.
    let rows = ops.iter().map(|op| {
        let name = &op.name;
        let params = op.params.iter().map(|c| c.variant_tokens());
        let result = match op.result {
            Some(c) => {
                let v = c.variant_tokens();
                quote!(Some(#v))
            }
            None => quote!(None),
        };
        let lowerable = op.lowerable;
        quote! {
            RtOp { name: #name, params: &[#(#params),*], result: #result, lowerable: #lowerable },
        }
    });

    // The typed `OPS` accessor: `field: &RUNTIME_OPS[i]`, and the struct field decls.
    let field_idents: Vec<syn::Ident> = ops.iter().map(|op| field_ident(&op.name)).collect();
    let field_decls = field_idents.iter().map(|f| quote!(pub #f: &'static RtOp,));
    let field_inits = field_idents.iter().enumerate().map(|(i, f)| {
        let idx = proc_macro2::Literal::usize_unsuffixed(i);
        quote!(#f: &RUNTIME_OPS[#idx],)
    });

    // Each committed hash is the DEFAULT for a compile-time `CDZ_*_HASH` env override (see the consts'
    // docs below): a nix build exports the env with the hash of the runtime/nfc component it built in the
    // SAME closure, so the compiler stamps exactly that runtime (self-consistent per host, no cross-host
    // byte-reproducibility requirement); a plain cargo build gets no env and uses the committed literal.
    let runtime_hash_expr = env_or_default_hash("CDZ_RUNTIME_HASH", runtime_hash);
    let debug_runtime_hash_expr = env_or_default_hash("CDZ_DEBUG_RUNTIME_HASH", debug_runtime_hash);
    let nfc_hash_expr = env_or_default_hash("CDZ_NFC_HASH", nfc_hash);

    quote! {
        #[doc = " The LOGICAL value type of a runtime op's param/result, as the WIT declares it (the"]
        #[doc = " runtime interface uses only these four). Kept logical, NOT collapsed to a core valtype,"]
        #[doc = " because the two envelope surfaces need different projections that are not recoverable from"]
        #[doc = " each other: a core `i32` is the lowering of `u32`, `bool` AND `s32` alike, so the COMPONENT"]
        #[doc = " import instance-type (which must structurally match the runtime's exported type) cannot be"]
        #[doc = " rebuilt from the core byte. `core_byte` is the core-module import functype's valtype;"]
        #[doc = " `comp_byte` is the component instance-type's primitive valtype. Both are wasm-spec"]
        #[doc = " constants (a TARGET concern) so they live here in the backend, not in the generated data."]
        #[doc = " The runtime interface itself uses only `U32`/`S64`/`Bool`/`F64`/`F32`; the remaining"]
        #[doc = " variants (the narrower aliased int widths `S8`/`U8`/`S16`/`U16`/`S32`/`U64`, and `Char`)"]
        #[doc = " are used by the HOST-op boundary (`host::abi_val_type`), where a delegated effect operation"]
        #[doc = " may carry any aliased scalar as a parameter or result — each crosses as its faithful"]
        #[doc = " component-model primitive, lowered to the core i32/i64/f32/f64 slot the canonical ABI uses."]
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum AbiValType { U32, S64, Bool, F64, F32, S8, U8, S16, U16, S32, U64, Char }

        impl AbiValType {
            #[doc = " The CORE wasm valtype byte a lowered scalar occupies (i32=0x7F, i64=0x7E, f64=0x7C,"]
            #[doc = " f32=0x7D). A `u32` handle / `bool` / `char` / every aliased int of width ≤32 lowers to"]
            #[doc = " core i32 (the canonical ABI sign/zero-extends a narrow value into the i32 slot); a 64-bit"]
            #[doc = " int is i64."]
            pub fn core_byte(self) -> u8 {
                match self {
                    AbiValType::U32
                    | AbiValType::Bool
                    | AbiValType::S8
                    | AbiValType::U8
                    | AbiValType::S16
                    | AbiValType::U16
                    | AbiValType::S32
                    | AbiValType::Char => 0x7F,
                    AbiValType::S64 | AbiValType::U64 => 0x7E,
                    AbiValType::F64 => 0x7C,
                    AbiValType::F32 => 0x7D,
                }
            }
            #[doc = " The COMPONENT-model primitive valtype byte — the faithful boundary type the import"]
            #[doc = " instance-type declares (wasm-spec constants: bool=0x7F, s8=0x7E, u8=0x7D, s16=0x7C,"]
            #[doc = " u16=0x7B, s32=0x7A, u32=0x79, s64=0x78, u64=0x77, f32=0x76, f64=0x75, char=0x74)."]
            pub fn comp_byte(self) -> u8 {
                match self {
                    AbiValType::Bool => 0x7F,
                    AbiValType::S8 => 0x7E,
                    AbiValType::U8 => 0x7D,
                    AbiValType::S16 => 0x7C,
                    AbiValType::U16 => 0x7B,
                    AbiValType::S32 => 0x7A,
                    AbiValType::U32 => 0x79,
                    AbiValType::S64 => 0x78,
                    AbiValType::U64 => 0x77,
                    AbiValType::F32 => 0x76,
                    AbiValType::F64 => 0x75,
                    AbiValType::Char => 0x74,
                }
            }
        }

        #[doc = " One value-heap runtime op the compiler may import: its WIT name and logical signature. A"]
        #[doc = " non-`lowerable` op carries a non-scalar WIT type (string/tuple) the bare-core envelope"]
        #[doc = " cannot import; it is listed for completeness but never selected."]
        pub struct RtOp {
            pub name: &'static str,
            pub params: &'static [AbiValType],
            pub result: Option<AbiValType>,
            pub lowerable: bool,
        }

        #[doc = " The runtime interface a program imports — the fixed ABI identity, prefix of the"]
        #[doc = " versioned `<iface>@…+<hash>` import name (`component-abi.md` §The Value-Heap Runtime"]
        #[doc = " Crosses By A Well-Known Import). The content-address suffix is `REQUIRED_RUNTIME_HASH`."]
        pub const RUNTIME_IFACE: &str = #iface;

        #[doc = " The BLAKE3 content address of the value-heap runtime component this ABI was generated"]
        #[doc = " against — the runtime a program built with this compiler requires. Regenerated from the"]
        #[doc = " built runtime bytes, so it tracks a runtime-code change automatically. Overridable at"]
        #[doc = " COMPILE TIME via the `CDZ_RUNTIME_HASH` env: a nix build bakes the hash of the runtime it"]
        #[doc = " built in the SAME closure (so the compiler and its runtime stay self-consistent per host,"]
        #[doc = " with no cross-host byte-reproducibility requirement); absent, the committed default is used."]
        pub const REQUIRED_RUNTIME_HASH: &str = #runtime_hash_expr;

        #[doc = " The BLAKE3 content address of the DEBUG-COUNTERS runtime build — the same runtime code"]
        #[doc = " with the `live-objects` leak counter compiled in (`--features debug-counters`). A shipped"]
        #[doc = " program pins `REQUIRED_RUNTIME_HASH` (the release build); a Perceus leak-check harness"]
        #[doc = " composes THIS build to assert `live-objects == 0` after a run. Recorded here so the harness"]
        #[doc = " locates the debug runtime by content address (from the store), never by rebuilding it."]
        #[doc = " Overridable at compile time via the `CDZ_DEBUG_RUNTIME_HASH` env (see `REQUIRED_RUNTIME_HASH`)."]
        pub const DEBUG_RUNTIME_HASH: &str = #debug_runtime_hash_expr;

        #[doc = " The NFC-normalization interface — the plain WIT name the value-heap RUNTIME imports for"]
        #[doc = " Unicode Normalization Form C. FINDING#23 (operator ruling d): NFC lives in a SEPARATE"]
        #[doc = " component (the heavy `unicode-normalization` tables); the runtime's WIT `world` declares"]
        #[doc = " `import cadenza:nfc/normalize` (a runtime-world dep under this PLAIN iface name — NOT a"]
        #[doc = " program-emitted versioned `@…+<hash>` import), and the host composes the NFC component into"]
        #[doc = " the runtime by content hash. The compiler emits NO program-side NFC import; this const is"]
        #[doc = " the interface name the host matches when composing (see cdz-run compose_nfc_into_runtime_linker)."]
        pub const NFC_IFACE: &str = "cadenza:nfc/normalize";

        #[doc = " The BLAKE3 content address of the NFC component (`cdz-nfc`) the RUNTIME imports. Regenerated"]
        #[doc = " from the built NFC-component bytes like `REQUIRED_RUNTIME_HASH`, so it tracks an NFC-code"]
        #[doc = " change automatically. The host resolves + composes the NFC component from the CAS by this hash"]
        #[doc = " (the store records `nfc = \"<hash>\"`; cdz-run/main.rs verify the loaded bytes against it). The"]
        #[doc = " NFC dep lives on the RUNTIME's world, so the NFC-code hash feeds `REQUIRED_RUNTIME_HASH`"]
        #[doc = " (the runtime that imports NFC hashes differently); it is not a separate program-import hash."]
        #[doc = " Overridable at compile time via the `CDZ_NFC_HASH` env (see `REQUIRED_RUNTIME_HASH`)."]
        pub const REQUIRED_NFC_HASH: &str = #nfc_hash_expr;

        #[doc = " The runtime's INLINE-UNIT handle — the value `arr-alloc(0)` returns (a compile-time-known"]
        #[doc = " handle carrying the empty tuple/unit, no heap node). DERIVED from the runtime's `cdz-abi`"]
        #[doc = " custom section (read at codegen, then stripped), so the compiler can push it as a constant"]
        #[doc = " for a unit payload (a nullary sum variant, an empty tuple/record/list) instead of emitting"]
        #[doc = " a runtime `arr-alloc(0)` CALL. Guarded by the content hash — never hand-transcribed."]
        pub const IMM_UNIT: u32 = #imm_unit;

        #[doc = " Every op the runtime `heap` interface declares, as structured signature data (sorted)."]
        pub const RUNTIME_OPS: &[RtOp] = &[ #(#rows)* ];

        #[doc = " Typed access to each runtime op by name — `OPS.arr_get` borrows `&RUNTIME_OPS[i]`. A"]
        #[doc = " field per op (kebab-case WIT name → snake_case field), so a rename in the WIT is a"]
        #[doc = " compile error at every use rather than a silent stringly-typed miss."]
        pub struct RuntimeOps { #(#field_decls)* }

        #[doc = " The one `RuntimeOps` value — each field borrows its entry in `RUNTIME_OPS` by offset."]
        pub const OPS: RuntimeOps = RuntimeOps { #(#field_inits)* };
    }
}

/// The Rust field identifier for a runtime op — the WIT's kebab-case name with `-` → `_` (a valid,
/// readable snake-case identifier: `arr-get` → `arr_get`, `map-iter-next` → `map_iter_next`).
fn field_ident(op_name: &str) -> syn::Ident {
    syn::Ident::new(&op_name.replace('-', "_"), proc_macro2::Span::call_site())
}

// ================================================================================================
// wasm_abi — extract the backend's wasm / component-model bytes from `wasm-encoder`.
//
// The backend hand-emits bytes (no encoder in the compile path, so the byte path ports 1:1 to the
// Cadenza self-host), but the byte VALUES are the spec's, not ours to invent. This module recovers
// each one from `wasm-encoder`: encode a single value (an `Instruction`, a `ValType`, a one-func
// section, a whole `Module`/`Component`) and read the byte(s) back. The result is a plain table the
// backend consumes — the same "structured data the compiler builds the wasm up from" shape as
// `runtime_abi.rs`, and byte-identical to what the oracle test already pins.
// ================================================================================================
mod wasm_abi {
    use super::{Span, TokenStream, quote};
    use wasm_encoder::{
        Component, ComponentSectionId, ComponentTypeSection, Encode, ExportKind, Instruction,
        Module, PrimitiveValType, SectionId, TypeSection, ValType,
    };

    /// One generated `op::<IDENT> = 0x..` opcode: the backend's Lir-instruction name and the single
    /// byte `wasm-encoder` emits for the matching `Instruction`.
    pub struct Opcode {
        /// The `SCREAMING_SNAKE` constant name the backend uses (`I32_ADD`, `LOCAL_GET`, …).
        pub ident: &'static str,
        /// The opcode byte, read back from encoding the `Instruction`.
        pub byte: u8,
    }

    /// One generated named single-byte constant — a valtype, section id, export-kind, or functype
    /// form byte. `doc` is the `///` line explaining what the byte is.
    pub struct Single {
        pub ident: &'static str,
        pub byte: u8,
        pub doc: &'static str,
    }

    /// One generated magic-header constant (`&[u8]`) — the 8-byte core-module / component preamble.
    pub struct Magic {
        pub ident: &'static str,
        pub bytes: [u8; 8],
        pub doc: &'static str,
    }

    /// The whole extracted table: the opcode list, the named single bytes, and the magic headers.
    pub struct Tables {
        pub opcodes: Vec<Opcode>,
        pub singles: Vec<Single>,
        pub magics: Vec<Magic>,
    }

    /// The single byte `wasm-encoder` emits for an `Instruction` — the opcode of an instruction whose
    /// operands (if any) follow. Every opcode the backend emits is a one-byte opcode, so byte 0 of the
    /// encoding IS the opcode; a longer encoding here would mean the instruction is multi-byte and the
    /// extraction is unsound, so that is asserted rather than silently truncated.
    fn opcode_of(name: &str, insn: Instruction) -> u8 {
        let mut buf = Vec::new();
        insn.encode(&mut buf);
        assert!(
            !buf.is_empty(),
            "wasm-encoder emitted no bytes for the {name} opcode"
        );
        // The operands follow the opcode byte; the backend emits them itself (LEB128), so only byte 0
        // is the opcode. (For operand-less instructions the encoding is exactly one byte.)
        buf[0]
    }

    /// The single byte `wasm-encoder` emits for a value that encodes to exactly one byte (a valtype,
    /// an export-kind). Asserts the one-byte shape so a spec change that widened it can't slip through.
    fn one_byte<T: Encode>(name: &str, v: T) -> u8 {
        let mut buf = Vec::new();
        v.encode(&mut buf);
        assert_eq!(
            buf.len(),
            1,
            "expected {name} to encode to a single byte, got {buf:?}"
        );
        buf[0]
    }

    /// The core FUNCTYPE FORM byte (`0x60`) — the tag that opens a core function type. It is only
    /// emitted inside a type section, so encode a one-function `TypeSection` and read the tag off the
    /// first (only) type entry — the byte `wasm-encoder` pushes before the param/result vectors.
    fn core_functype_form() -> u8 {
        let mut ts = TypeSection::new();
        ts.ty().function([], []);
        first_type_entry_tag("core functype", &ts)
    }

    /// The COMPONENT functype form byte (`0x40`) — the tag opening a component function type, likewise
    /// only emitted inside a component type section. Encode a one-function `ComponentTypeSection` and
    /// read the tag off its first entry.
    fn component_functype_form() -> u8 {
        let mut ts = ComponentTypeSection::new();
        // `params` must be encoded before `result` (the encoder asserts it); a nullary `() -> ()` is
        // enough — only the leading form tag is read.
        ts.function()
            .params(std::iter::empty::<(&str, wasm_encoder::ComponentValType)>())
            .result(None);
        first_type_entry_tag("component functype", &ts)
    }

    /// The leading tag byte of the FIRST entry in a one-entry type section. `wasm-encoder` frames a
    /// section as `<id:1> <byte-length:leb> <count:leb> <entries…>`; a section holding a single small
    /// function type has both the length and the count in one LEB byte each, so the entry — whose
    /// first byte is the functype form tag — starts at offset 3. (Asserted, so a framing change or a
    /// non-trivial length can't silently shift the read.)
    fn first_type_entry_tag<S: SectionBytes>(name: &str, sec: &S) -> u8 {
        let bytes = sec.section_bytes();
        // [0]=section id, [1]=section byte-length (1 LEB byte, the type is tiny), [2]=entry count.
        assert!(
            bytes.len() > 3,
            "{name} section unexpectedly short: {bytes:?}"
        );
        assert_eq!(
            bytes[2], 1,
            "{name} section should hold exactly one type entry, got count {}",
            bytes[2]
        );
        bytes[3]
    }

    /// A minimal shim over the two section flavors: emit the full `<id> <len> <contents>` bytes of a
    /// section. `wasm-encoder`'s `Section`/`ComponentSection` traits both expose exactly this via
    /// `append_to`, but they are distinct traits; this unifies them for `section_contents_first_byte`.
    trait SectionBytes {
        fn section_bytes(&self) -> Vec<u8>;
    }
    impl SectionBytes for TypeSection {
        fn section_bytes(&self) -> Vec<u8> {
            let mut m = Module::new();
            m.section(self);
            // A fresh `Module` is `[8-byte header][section…]`; the section is everything after it.
            m.finish()[Module::HEADER.len()..].to_vec()
        }
    }
    impl SectionBytes for ComponentTypeSection {
        fn section_bytes(&self) -> Vec<u8> {
            let mut c = Component::new();
            c.section(self);
            c.finish()[Component::HEADER.len()..].to_vec()
        }
    }

    /// Extract every backend byte from `wasm-encoder`. The opcode list is in the backend's own
    /// declaration order (matching the frozen `op` module for a readable, stable diff); the singles
    /// and magics follow.
    pub fn collect() -> Tables {
        // The opcode table: each backend `op::` constant paired with the `Instruction` whose encoding
        // yields its byte. One row per instruction the serializer emits (Lir → bytes).
        let opcodes = vec![
            op("I32_CONST", Instruction::I32Const(0)),
            op("I64_CONST", Instruction::I64Const(0)),
            op("F64_CONST", Instruction::F64Const(0.0f64.into())),
            op("F32_CONST", Instruction::F32Const(0.0f32.into())),
            // Float ARITHMETIC (f64/f32) — the machine ops a runtime `+.`/`-.`/`*.`/`/.` selects.
            // IEEE, never trapping (overflow → inf, x/0 → ±inf/NaN), so no overflow guard (unlike the
            // integer arith). Round-to-nearest-even is the hardware default the determinism contract pins.
            op("F64_ADD", Instruction::F64Add),
            op("F64_SUB", Instruction::F64Sub),
            op("F64_MUL", Instruction::F64Mul),
            op("F64_DIV", Instruction::F64Div),
            op("F32_ADD", Instruction::F32Add),
            op("F32_SUB", Instruction::F32Sub),
            op("F32_MUL", Instruction::F32Mul),
            op("F32_DIV", Instruction::F32Div),
            // Float EQUALITY (f64/f32) — a runtime float `=` (IEEE compare: -0.0 == 0.0, NaN ≠ NaN).
            op("F64_EQ", Instruction::F64Eq),
            op("F64_NE", Instruction::F64Ne),
            op("F32_EQ", Instruction::F32Eq),
            op("F32_NE", Instruction::F32Ne),
            // Float ORDERING (f64/f32) — runtime `< <= > >=` under IEEE PARTIAL order (operator ruling):
            // a NaN operand yields false (unordered — NaN is neither <, >, nor = anything), and -0.0
            // compares equal-under-ordering to +0.0 (`f64.le -0.0 0.0` = true). These are the RAW IEEE
            // compares, DISTINCT from the canonical-byte equality above (which the two relations disagree
            // with on NaN + signed zero — inherent to float).
            op("F64_LT", Instruction::F64Lt),
            op("F64_GT", Instruction::F64Gt),
            op("F64_LE", Instruction::F64Le),
            op("F64_GE", Instruction::F64Ge),
            op("F32_LT", Instruction::F32Lt),
            op("F32_GT", Instruction::F32Gt),
            op("F32_LE", Instruction::F32Le),
            op("F32_GE", Instruction::F32Ge),
            // Float width conversion (F5): `f32.demote_f64` narrows Float64→Float32 (rounds),
            // `f64.promote_f32` widens Float32→Float64 (exact). Int↔float conversions land with `of-int`.
            op("F32_DEMOTE_F64", Instruction::F32DemoteF64),
            op("F64_PROMOTE_F32", Instruction::F64PromoteF32),
            // INT→FLOAT conversion (`Float N.of-int`): signed i64 → f64/f32, round-to-nearest-even.
            op("F64_CONVERT_I64_S", Instruction::F64ConvertI64S),
            op("F32_CONVERT_I64_S", Instruction::F32ConvertI64S),
            // FLOAT→INT bit reinterpret (no value change) — the canonical-byte float `=` reinterprets
            // the bits to an integer to compare (NaN-canonicalizing bit compare, IEEE `f*.eq` won't do).
            op("I32_REINTERPRET_F32", Instruction::I32ReinterpretF32),
            op("I64_REINTERPRET_F64", Instruction::I64ReinterpretF64),
            op("IF", Instruction::If(wasm_encoder::BlockType::Empty)),
            op("ELSE", Instruction::Else),
            op("END", Instruction::End),
            op("SELECT", Instruction::Select),
            op("DROP", Instruction::Drop),
            op("BLOCK", Instruction::Block(wasm_encoder::BlockType::Empty)),
            op("LOOP", Instruction::Loop(wasm_encoder::BlockType::Empty)),
            op("BR", Instruction::Br(0)),
            op("BR_IF", Instruction::BrIf(0)),
            op(
                "BR_TABLE",
                Instruction::BrTable(std::borrow::Cow::Borrowed(&[]), 0),
            ),
            op("LOCAL_GET", Instruction::LocalGet(0)),
            op("LOCAL_SET", Instruction::LocalSet(0)),
            op("LOCAL_TEE", Instruction::LocalTee(0)),
            op("GLOBAL_GET", Instruction::GlobalGet(0)),
            op("GLOBAL_SET", Instruction::GlobalSet(0)),
            op("CALL", Instruction::Call(0)),
            op(
                "CALL_INDIRECT",
                Instruction::CallIndirect {
                    type_index: 0,
                    table_index: 0,
                },
            ),
            op("RETURN_CALL", Instruction::ReturnCall(0)),
            op("RETURN", Instruction::Return),
            op("UNREACHABLE", Instruction::Unreachable),
            op("I32_ADD", Instruction::I32Add),
            op("I32_SUB", Instruction::I32Sub),
            op("I32_MUL", Instruction::I32Mul),
            op("I32_DIV_S", Instruction::I32DivS),
            op("I32_DIV_U", Instruction::I32DivU),
            op("I32_REM_S", Instruction::I32RemS),
            op("I32_REM_U", Instruction::I32RemU),
            op("I32_AND", Instruction::I32And),
            op("I32_OR", Instruction::I32Or),
            op("I32_XOR", Instruction::I32Xor),
            op("I32_SHL", Instruction::I32Shl),
            op("I32_SHR_S", Instruction::I32ShrS),
            op("I32_SHR_U", Instruction::I32ShrU),
            op("I32_EQ", Instruction::I32Eq),
            op("I32_EQZ", Instruction::I32Eqz),
            op("I32_NE", Instruction::I32Ne),
            op("I32_LT_S", Instruction::I32LtS),
            op("I32_LT_U", Instruction::I32LtU),
            op("I32_GT_S", Instruction::I32GtS),
            op("I32_GT_U", Instruction::I32GtU),
            op("I32_LE_S", Instruction::I32LeS),
            op("I32_LE_U", Instruction::I32LeU),
            op("I32_GE_S", Instruction::I32GeS),
            op("I32_GE_U", Instruction::I32GeU),
            op("I64_ADD", Instruction::I64Add),
            op("I64_SUB", Instruction::I64Sub),
            op("I64_MUL", Instruction::I64Mul),
            op("I64_DIV_S", Instruction::I64DivS),
            op("I64_DIV_U", Instruction::I64DivU),
            op("I64_REM_S", Instruction::I64RemS),
            op("I64_REM_U", Instruction::I64RemU),
            op("I64_AND", Instruction::I64And),
            op("I64_OR", Instruction::I64Or),
            op("I64_XOR", Instruction::I64Xor),
            op("I64_SHL", Instruction::I64Shl),
            op("I64_SHR_S", Instruction::I64ShrS),
            op("I64_SHR_U", Instruction::I64ShrU),
            op("I64_EQ", Instruction::I64Eq),
            op("I64_EQZ", Instruction::I64Eqz),
            op("I64_NE", Instruction::I64Ne),
            op("I64_LT_S", Instruction::I64LtS),
            op("I64_LT_U", Instruction::I64LtU),
            op("I64_GT_S", Instruction::I64GtS),
            op("I64_GT_U", Instruction::I64GtU),
            op("I64_LE_S", Instruction::I64LeS),
            op("I64_LE_U", Instruction::I64LeU),
            op("I64_GE_S", Instruction::I64GeS),
            op("I64_GE_U", Instruction::I64GeU),
            op("I32_WRAP_I64", Instruction::I32WrapI64),
            op("I64_EXTEND_I32_S", Instruction::I64ExtendI32S),
            op("I64_EXTEND_I32_U", Instruction::I64ExtendI32U),
            // Memory stores — the `list<u8>`/`string` return path writes the payload bytes and the
            // canonical-ABI return area (`[data-ptr, data-len]`) into linear memory. `MemArg` is
            // irrelevant to the opcode byte (the extraction reads byte 0; the serializer emits the
            // align/offset LEB operands itself), so a zero memarg suffices to recover the opcode.
            op(
                "I32_STORE",
                Instruction::I32Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            op(
                "I32_STORE8",
                Instruction::I32Store8(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            // The wider stores the reducer RESULT-lower (W4c-b canonical writer) needs: an `i64`/`f64`/`f32`
            // scalar field, and the 2-byte `i32.store16` for a `u16`/`s16` slot. Same MemArg-irrelevant note.
            op(
                "I64_STORE",
                Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            op(
                "F32_STORE",
                Instruction::F32Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            op(
                "F64_STORE",
                Instruction::F64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            op(
                "I32_STORE16",
                Instruction::I32Store16(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            // Memory loads — the reducer byte-ABI `apply(event: list<u8>)` wrapper (B3) reads the incoming
            // event bytes out of linear memory (the canonical ABI delivers a `list<u8>` param as a
            // `(ptr, len)` pair with the bytes at `ptr`) to copy them into a heap `Bytes` before
            // `value-decode`. `I32_LOAD8_U` reads one unsigned byte; `I32_LOAD` reads the 4-byte
            // `(ptr, len)` fields where needed. Same `MemArg`-irrelevant-to-opcode-byte note as the stores.
            op(
                "I32_LOAD",
                Instruction::I32Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            op(
                "I32_LOAD8_U",
                Instruction::I32Load8U(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            // Width-specific loads for reading a scalar leaf a spilled compound result stored at its natural
            // width (the general result-lift's scalar-leaf boxing). MemArg is opcode-byte-irrelevant here too.
            op(
                "I64_LOAD",
                Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            op(
                "F32_LOAD",
                Instruction::F32Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            op(
                "F64_LOAD",
                Instruction::F64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            op(
                "I32_LOAD8_S",
                Instruction::I32Load8S(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            op(
                "I32_LOAD16_S",
                Instruction::I32Load16S(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            op(
                "I32_LOAD16_U",
                Instruction::I32Load16U(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
        ];

        // The named single bytes: core valtypes, the empty block type, the component primitive
        // valtypes, the core + component section ids the envelope uses, the func export kind, and the
        // two functype form bytes.
        let singles = vec![
            valtype(
                "CORE_I32",
                "core valtype `i32` — a ≤32-bit scalar's machine slot.",
                ValType::I32,
            ),
            valtype(
                "CORE_I64",
                "core valtype `i64` — a 64-bit scalar's machine slot.",
                ValType::I64,
            ),
            valtype("CORE_F32", "core valtype `f32`.", ValType::F32),
            valtype("CORE_F64", "core valtype `f64`.", ValType::F64),
            Single {
                ident: "BLOCK_EMPTY",
                byte: one_byte("empty block type", wasm_encoder::BlockType::Empty),
                doc: "empty block type — a structured block (`if`/`block`) that leaves no value.",
            },
            prim("COMP_BOOL", "component `bool`.", PrimitiveValType::Bool),
            prim("COMP_S8", "component `s8`.", PrimitiveValType::S8),
            prim("COMP_U8", "component `u8`.", PrimitiveValType::U8),
            prim("COMP_S16", "component `s16`.", PrimitiveValType::S16),
            prim("COMP_U16", "component `u16`.", PrimitiveValType::U16),
            prim("COMP_S32", "component `s32`.", PrimitiveValType::S32),
            prim("COMP_U32", "component `u32`.", PrimitiveValType::U32),
            prim("COMP_S64", "component `s64`.", PrimitiveValType::S64),
            prim("COMP_U64", "component `u64`.", PrimitiveValType::U64),
            prim("COMP_F32", "component `f32`.", PrimitiveValType::F32),
            prim("COMP_F64", "component `f64`.", PrimitiveValType::F64),
            prim(
                "COMP_STRING",
                "component `string`.",
                PrimitiveValType::String,
            ),
            core_sec("CORE_SEC_TYPE", "core TYPE section id.", SectionId::Type),
            core_sec(
                "CORE_SEC_FUNCTION",
                "core FUNCTION section id.",
                SectionId::Function,
            ),
            core_sec(
                "CORE_SEC_TABLE",
                "core TABLE section id (the funcref table a closure's `call_indirect` dispatches through — one entry per lambda-lifted closure function).",
                SectionId::Table,
            ),
            core_sec(
                "CORE_SEC_ELEMENT",
                "core ELEMENT section id (the active segment filling the funcref table with the lifted closure functions' indices, so a closure's stored table slot names its code).",
                SectionId::Element,
            ),
            core_sec(
                "CORE_SEC_MEMORY",
                "core MEMORY section id (the linear memory a `list<u8>`/`string` return lifts through).",
                SectionId::Memory,
            ),
            core_sec(
                "CORE_SEC_GLOBAL",
                "core GLOBAL section id (a build-once static compound's handle global; the bump-allocator cursor).",
                SectionId::Global,
            ),
            core_sec(
                "CORE_SEC_EXPORT",
                "core EXPORT section id.",
                SectionId::Export,
            ),
            core_sec(
                "CORE_SEC_START",
                "core START section id (the init function that builds each static compound once).",
                SectionId::Start,
            ),
            core_sec("CORE_SEC_CODE", "core CODE section id.", SectionId::Code),
            core_sec(
                "CORE_SEC_DATA",
                "core DATA section id (an active segment initializing linear memory — the constant value form + return area of a resource escape).",
                SectionId::Data,
            ),
            comp_sec(
                "COMP_SEC_CORE_MODULE",
                "component CORE-MODULE section id.",
                ComponentSectionId::CoreModule,
            ),
            comp_sec(
                "COMP_SEC_CORE_INSTANCE",
                "component CORE-INSTANCE section id.",
                ComponentSectionId::CoreInstance,
            ),
            comp_sec(
                "COMP_SEC_ALIAS",
                "component ALIAS section id.",
                ComponentSectionId::Alias,
            ),
            comp_sec(
                "COMP_SEC_TYPE",
                "component TYPE section id.",
                ComponentSectionId::Type,
            ),
            comp_sec(
                "COMP_SEC_CANONICAL",
                "component CANONICAL-FUNCTION section id.",
                ComponentSectionId::CanonicalFunction,
            ),
            comp_sec(
                "COMP_SEC_IMPORT",
                "component IMPORT section id.",
                ComponentSectionId::Import,
            ),
            comp_sec(
                "COMP_SEC_EXPORT",
                "component EXPORT section id.",
                ComponentSectionId::Export,
            ),
            comp_sec(
                "COMP_SEC_COMPONENT",
                "component COMPONENT section id (a nested component definition).",
                ComponentSectionId::Component,
            ),
            comp_sec(
                "COMP_SEC_INSTANCE",
                "component INSTANCE section id (instantiate a component).",
                ComponentSectionId::Instance,
            ),
            Single {
                ident: "EXPORT_KIND_FUNC",
                byte: one_byte("export kind func", ExportKind::Func),
                doc: "core export kind `func` — the export-descriptor byte for a function export.",
            },
            Single {
                ident: "EXPORT_KIND_MEMORY",
                byte: one_byte("export kind memory", ExportKind::Memory),
                doc: "core export kind `memory` — the export-descriptor byte for a memory export (the resource escape's `memory`).",
            },
            Single {
                ident: "CORE_FUNCTYPE_FORM",
                byte: core_functype_form(),
                doc: "core functype form tag — opens a core `func` type before its param/result vecs.",
            },
            Single {
                ident: "COMP_FUNCTYPE_FORM",
                byte: component_functype_form(),
                doc: "component functype form tag — opens a component function type.",
            },
        ];

        let magics = vec![
            Magic {
                ident: "CORE_MAGIC",
                bytes: Module::HEADER,
                doc: "the `\\0asm` version-1 core-module preamble.",
            },
            Magic {
                ident: "COMPONENT_MAGIC",
                bytes: Component::HEADER,
                doc: "the `\\0asm` component-layer preamble (component-model version).",
            },
        ];

        Tables {
            opcodes,
            singles,
            magics,
        }
    }

    /// An opcode row: extract the byte from encoding `insn`.
    fn op(ident: &'static str, insn: Instruction) -> Opcode {
        Opcode {
            ident,
            byte: opcode_of(ident, insn),
        }
    }

    /// A core-valtype single: extract the byte from encoding the `ValType`.
    fn valtype(ident: &'static str, doc: &'static str, vt: ValType) -> Single {
        Single {
            ident,
            byte: one_byte(ident, vt),
            doc,
        }
    }

    /// A component-primitive single: extract the byte from encoding the `PrimitiveValType`.
    fn prim(ident: &'static str, doc: &'static str, p: PrimitiveValType) -> Single {
        Single {
            ident,
            byte: one_byte(ident, p),
            doc,
        }
    }

    /// A core-section-id single (the id is the enum's `u8` repr).
    fn core_sec(ident: &'static str, doc: &'static str, id: SectionId) -> Single {
        Single {
            ident,
            byte: u8::from(id),
            doc,
        }
    }

    /// A component-section-id single.
    fn comp_sec(ident: &'static str, doc: &'static str, id: ComponentSectionId) -> Single {
        Single {
            ident,
            byte: u8::from(id),
            doc,
        }
    }

    /// Render the extracted tables as the generated `wasm_abi.rs` body: the `op` opcode module, then
    /// the named single-byte constants, then the magic-header slices. Hex literals so the file reads
    /// like the spec (`pub const I32_ADD: u8 = 0x6a;`).
    pub fn render(t: &Tables) -> TokenStream {
        let op_consts = t.opcodes.iter().map(|o| {
            let ident = ident(o.ident);
            let byte = hex(o.byte);
            quote!(pub const #ident: u8 = #byte;)
        });

        let single_consts = t.singles.iter().map(|s| {
            let ident = ident(s.ident);
            let byte = hex(s.byte);
            let doc = format!(" {}", s.doc);
            quote! {
                #[doc = #doc]
                pub const #ident: u8 = #byte;
            }
        });

        let magic_consts = t.magics.iter().map(|m| {
            let ident = ident(m.ident);
            let bytes = m.bytes.iter().map(|b| hex(*b));
            let doc = format!(" {}", m.doc);
            quote! {
                #[doc = #doc]
                pub const #ident: &[u8] = &[#(#bytes),*];
            }
        });

        quote! {
            #[doc = " The core-wasm opcode bytes the serializer emits — one `pub const` per Lir"]
            #[doc = " instruction, the byte `wasm-encoder` encodes that instruction to. A one-byte opcode"]
            #[doc = " each (its operands follow, emitted by the serializer); the extraction asserts that."]
            pub mod op {
                #(#op_consts)*
            }

            #(#single_consts)*

            #(#magic_consts)*
        }
    }

    /// A hex `u8` literal token (`0x6a`) — matches the spec/wasm convention the frozen source used, so
    /// the generated file reads like an opcode table rather than decimal noise. The const's own `: u8`
    /// / `&[u8]` annotation types it, so the literal is unsuffixed; a raw token keeps the `0x` form
    /// through prettyplease/rustfmt (which would rewrite a `Literal`'s decimal print).
    fn hex(b: u8) -> TokenStream {
        format!("0x{b:02x}")
            .parse()
            .expect("a `0x..` hex literal parses as a token")
    }

    /// A `SCREAMING_SNAKE` const identifier token.
    fn ident(name: &str) -> proc_macro2::Ident {
        proc_macro2::Ident::new(name, Span::call_site())
    }
}

/// The `//!` banner prepended to `runtime_abi.rs` — the "do-not-edit / regenerate" notice. A module
/// doc is awkward to carry as a token attribute, so it is plain leading text (rustfmt leaves a `//!`
/// block alone).
fn runtime_abi_banner() -> String {
    "//! @generated by `cargo xtask codegen` from cdz-runtime/wit/runtime.wit — DO NOT hand-edit.\n\
     //!\n\
     //! The value-heap runtime's ABI as STRUCTURED data: each op's name + core signature, a typed\n\
     //! `OPS` accessor, the interface name, and the runtime's content hash. The wasm backend builds a\n\
     //! program's per-program import section from this (importing only the ops it uses) — no opaque\n\
     //! envelope blob, no baked-in op index (imports resolve by name). Regenerate with `cargo xtask\n\
     //! codegen`; `cargo xtask codegen --check` (a hard gate in `xtask check`) fails if it drifts from\n\
     //! the runtime WIT or the built runtime's bytes. Plain data — no dependency, so it ships.\n\n"
        .to_string()
}

/// The `//!` banner prepended to `wasm_abi.rs` — the "do-not-edit / regenerate" notice, plus what the
/// file is and where its bytes come from.
fn wasm_abi_banner() -> String {
    "//! @generated by `cargo xtask codegen` from the `wasm-encoder` crate — DO NOT hand-edit.\n\
     //!\n\
     //! Every wasm / component-model byte the backend emits, EXTRACTED from `wasm-encoder` (the spec\n\
     //! byte encoder): the core opcode table, core + component valtype bytes, core + component section\n\
     //! ids, the two magic headers, and the functype form bytes. `codegen` encodes a one-off value\n\
     //! with `wasm-encoder` and reads the byte back, so nothing here is hand-transcribed. Regenerate\n\
     //! with `cargo xtask codegen`; `cargo xtask codegen --check` (a hard gate in `xtask check`) fails\n\
     //! if it drifts from the encoder. Plain data — no dependency, so it ships in the portable\n\
     //! compiler (the `wasm-encoder` oracle stays in xtask). The backend's serializer reads these\n\
     //! rather than baking a raw byte into the emit path.\n\n"
        .to_string()
}
