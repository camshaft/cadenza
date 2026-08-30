//! `codegen` — generate the wasm backend's runtime ABI table from its authoritative oracle.
//!
//! The wasm backend emits bytes against the value-heap runtime's ABI. Rather than hand-transcribing it
//! (a hard-coded list that could silently drift from its source), it is DERIVED from the runtime WIT
//! and written as a plain-data Rust file the backend consumes. The generated file is PLAIN DATA — no
//! external dependency — so it ships in the portable compiler; the oracle crate (`wit-parser`) lives
//! ONLY here in xtask (a dev desk). Re-run `cargo xtask codegen` after changing the runtime WIT;
//! `cargo xtask codegen --check` (a hard gate in `xtask check`) fails if the committed file has
//! drifted from its oracle or the built runtime's bytes.
//!
//!  - `runtime_abi.rs` — the VALUE-HEAP RUNTIME interface, declared once in the runtime crate's
//!    `wit/runtime.wit` (the ABI's source of truth). Read with `wit-parser` into one `RtOp { name,
//!    params, result }` per declared op, so the compiler builds a program's per-program import
//!    section from structured signature data (importing only the ops it uses) rather than pasting
//!    opaque envelope blobs. Also carries the runtime's content hash, so a runtime-code change (not
//!    only a WIT change) is caught by the staleness gate.
//!
//! (Two sibling generators USED to live here but were carved out into standalone bins + nix
//! derivations — the operator codegen→build-time-nix directive, each byte-identity-guarded: the
//! WASM/component byte table `wasm_abi.rs` → `xtask-codegen-wasm-abi` + `cdzWasmAbi`, and the built-in
//! CONTRACT schemas `cdz-platform/src/contracts/*.rs` → `xtask-codegen-contracts` + `cdzPlatformContracts`.
//! So `run` now generates only `runtime_abi.rs`; its runtime-HASH constants are next to move build-time.)

use crate::{Paths, build_component_with_features, content_address};
use proc_macro2::TokenStream;
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

/// Generate the backend's `runtime_abi.rs` from the runtime WIT. In `check` mode, regenerate it in
/// memory and compare to the committed file WITHOUT writing — the STALENESS GATE: exit non-zero if it
/// is out of date, so a forgotten regeneration fails `xtask check` rather than silently drifting from
/// its oracle. (The `wasm_abi.rs` + contract-schema generators moved to standalone bins + nix
/// derivations — see the module doc.)
pub fn run(paths: &Paths, check: bool) {
    generate_runtime_abi(paths, check);
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
    // The three runtime CONTENT-HASH consts live in their OWN generated file in `cadenza-compile-abi`
    // (a boundary crate in both the `rcdzc` and the thin `!standalone` `cdz` closures), so `cdz doctor`
    // reads `REQUIRED_RUNTIME_HASH` without linking `rcdzc` (the rcdzc-optional flip). `runtime_abi.rs`
    // re-exports them for byte-stability. Both files are emitted/checked by this one command.
    let hash_out = paths
        .seed
        .join("crates/cadenza-compile-abi/src/runtime_hash.rs");

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
    let body = format_tokens(render(&ops, iface, imm_unit));
    let source = format!("{}{body}", runtime_abi_banner());

    // The relocated hash consts (their own file in cadenza-compile-abi) — same derived values.
    let hash_body = format_tokens(render_runtime_hash(
        &runtime_hash,
        &debug_runtime_hash,
        &nfc_hash,
    ));
    let hash_source = format!("{}{hash_body}", runtime_hash_banner());

    let summary = format!(
        "{} ops, {} lowerable, from {}",
        ops.len(),
        ops.iter().filter(|o| o.lowerable).count(),
        wit.display()
    );
    emit_or_check(&out, &source, check, "the runtime WIT", &summary);
    emit_or_check(
        &hash_out,
        &hash_source,
        check,
        "the built runtime/nfc component bytes",
        "REQUIRED_RUNTIME_HASH / DEBUG_RUNTIME_HASH / REQUIRED_NFC_HASH",
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

/// The `//!` banner for the generated `cadenza-compile-abi/src/runtime_hash.rs` — the relocated home of
/// the three runtime CONTENT-HASH consts. They live in `cadenza-compile-abi` (not `rcdzc`'s
/// `runtime_abi.rs`) so the thin `!standalone` `cdz` dispatcher — which does NOT link `rcdzc` — can still
/// read `REQUIRED_RUNTIME_HASH` for `cdz doctor` (the rcdzc-optional flip); `rcdzc::backend::wasm::runtime_abi`
/// re-exports them so every existing `rcdzc::…::REQUIRED_RUNTIME_HASH` reference stays byte-stable.
fn runtime_hash_banner() -> String {
    "//! @generated by `cargo xtask codegen` from the built runtime/nfc component bytes — DO NOT hand-edit.\n\
     //!\n\
     //! The three runtime CONTENT-HASH consts (`REQUIRED_RUNTIME_HASH` / `DEBUG_RUNTIME_HASH` /\n\
     //! `REQUIRED_NFC_HASH`), regenerated from the built component bytes so they track a runtime/nfc-code\n\
     //! change automatically. They live HERE in `cadenza-compile-abi` (a boundary crate in both the `rcdzc`\n\
     //! and the thin `!standalone` `cdz` closures) so `cdz doctor` reads them WITHOUT linking `rcdzc`;\n\
     //! `rcdzc::backend::wasm::runtime_abi` `pub use`s them for byte-stability. Each is a compile-time\n\
     //! `CDZ_*_HASH` env override with the committed literal as the default (a nix build bakes the hash of\n\
     //! the component it built in the same closure). Regenerate with `cargo xtask codegen`; `--check` gates it.\n\n"
        .to_string()
}

/// Render the three runtime CONTENT-HASH consts as their own module body (for
/// `cadenza-compile-abi/src/runtime_hash.rs`). Split out of [`render`] (which now re-exports these from
/// `runtime_abi.rs`) so the thin `!standalone` `cdz` can read them without linking `rcdzc`. Same
/// `option_env!`-override-with-committed-default shape as before the move; the values are identical.
fn render_runtime_hash(
    runtime_hash: &str,
    debug_runtime_hash: &str,
    nfc_hash: &str,
) -> TokenStream {
    let runtime_hash_expr = env_or_default_hash("CDZ_RUNTIME_HASH", runtime_hash);
    let debug_runtime_hash_expr = env_or_default_hash("CDZ_DEBUG_RUNTIME_HASH", debug_runtime_hash);
    let nfc_hash_expr = env_or_default_hash("CDZ_NFC_HASH", nfc_hash);
    quote! {
        #[doc = " The BLAKE3 content address of the value-heap runtime component this compiler was generated"]
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

        #[doc = " The BLAKE3 content address of the NFC component (`cdz-nfc`) the RUNTIME imports. Regenerated"]
        #[doc = " from the built NFC-component bytes like `REQUIRED_RUNTIME_HASH`, so it tracks an NFC-code"]
        #[doc = " change automatically. The host resolves + composes the NFC component from the CAS by this hash"]
        #[doc = " (the store records `nfc = \"<hash>\"`; cdz-run/main.rs verify the loaded bytes against it). The"]
        #[doc = " NFC dep lives on the RUNTIME's world, so the NFC-code hash feeds `REQUIRED_RUNTIME_HASH`"]
        #[doc = " (the runtime that imports NFC hashes differently); it is not a separate program-import hash."]
        #[doc = " Overridable at compile time via the `CDZ_NFC_HASH` env (see `REQUIRED_RUNTIME_HASH`)."]
        pub const REQUIRED_NFC_HASH: &str = #nfc_hash_expr;
    }
}

fn render(ops: &[Op], iface: &str, imm_unit: u32) -> TokenStream {
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

        #[doc = " The three runtime CONTENT-HASH consts (`REQUIRED_RUNTIME_HASH` / `DEBUG_RUNTIME_HASH` /"]
        #[doc = " `REQUIRED_NFC_HASH`) live in `cadenza-compile-abi::runtime_hash` (generated by the same"]
        #[doc = " `cargo xtask codegen`) so the thin `!standalone` `cdz` dispatcher reads them WITHOUT linking"]
        #[doc = " `rcdzc`; re-exported here so every `rcdzc::backend::wasm::runtime_abi::REQUIRED_RUNTIME_HASH`"]
        #[doc = " reference (incl. the compiler's own runtime-stamp path) stays byte-stable."]
        pub use cadenza_compile_abi::runtime_hash::{
            DEBUG_RUNTIME_HASH, REQUIRED_NFC_HASH, REQUIRED_RUNTIME_HASH,
        };

        #[doc = " The NFC-normalization interface — the plain WIT name the value-heap RUNTIME imports for"]
        #[doc = " Unicode Normalization Form C. FINDING#23 (operator ruling d): NFC lives in a SEPARATE"]
        #[doc = " component (the heavy `unicode-normalization` tables); the runtime's WIT `world` declares"]
        #[doc = " `import cadenza:nfc/normalize` (a runtime-world dep under this PLAIN iface name — NOT a"]
        #[doc = " program-emitted versioned `@…+<hash>` import), and the host composes the NFC component into"]
        #[doc = " the runtime by content hash. The compiler emits NO program-side NFC import; this const is"]
        #[doc = " the interface name the host matches when composing (see cdz-run compose_nfc_into_runtime_linker)."]
        pub const NFC_IFACE: &str = "cadenza:nfc/normalize";

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
