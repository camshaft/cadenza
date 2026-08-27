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
//!  - `cdz-platform/src/contracts/<name>.rs` — each built-in contract's schema, projected from its
//!    Cadenza source `cdz-platform/contracts/<name>.cdz`. Validation and parsing are delegated to the
//!    `cdz` BINARY (`cdz test` typechecks + runs the source's `@test` conformance proofs; `cdz convert
//!    --to binary` yields the AST), so xtask depends only on `cadenza-ast` (the value model), NOT the
//!    compiler. The generated files are plain Cadenza-AST builder calls, so `cdz-platform` never links
//!    the compiler. Regenerated per-source by mtime. See its own section banner.

use crate::{Paths, build_component_with_features, content_address};
use cadenza_ast::ast::{Arenas, Struct, StructId};
use proc_macro2::{Span, TokenStream};
use quote::quote;
use std::path::{Path, PathBuf};
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
    generate_contracts(paths, check);
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

// ================================================================================================
// contracts — generate each built-in contract's schema from its Cadenza source in the contracts/ dir.
//
// A contract's identity is the hash of its declared schema (`design/cadenza-platform.md` section 1), so
// that schema must be VALID Cadenza and a value the runtime marshals against it must type-ascribe to the
// schema type. A hand-authored schema can silently be neither. So each contract's source of truth is a
// real Cadenza file under `cdz-platform/contracts/*.cdz`, and codegen projects its `type` declarations
// into `cdz-platform/src/contracts/<name>.rs` as plain Cadenza-AST builder calls the platform feeds to
// `Contract::new`.
//
// Validation and parsing are delegated to the `cdz` BINARY, so xtask does NOT depend on the compiler
// (only on `cadenza-ast`, the language's value model + codec, which `cdz-platform` itself depends on):
//   - `cdz test <src>` typechecks the source AND runs its `@test` conformance proofs — fully-literal
//     envelope values whose `-> Envelope` helper ascribes them against the schema type, so a value of the
//     shape `Deliver::encode` marshals is proven to be a value of the schema. Non-zero exit fails codegen.
//   - `cdz convert <src> --to binary` yields the canonical AST, which `cadenza_ast::codec` decodes; the
//     `type` declarations are extracted from it and re-emitted as builder calls.
//
// A source is only revalidated + regenerated when its generated file is out of date by MTIME (so a clean
// tree neither rebuilds `cdz` nor re-runs the suite). `cargo xtask codegen --check` (a hard gate in
// `xtask check`) fails if a committed file is stale.
// ================================================================================================

/// Generate a schema module for every `cdz-platform/contracts/*.cdz`, plus the `contracts/mod.rs` that
/// lists them (so adding a contract file wires it in with no hand-editing). See the section banner.
fn generate_contracts(paths: &Paths, check: bool) {
    let contracts_dir = paths.seed.join("crates/cdz-platform/contracts");
    let out_dir = paths.seed.join("crates/cdz-platform/src/contracts");
    // The DIRECTORY is the classification (operator ruling, no hardcoded xtask list): `contracts/kernel/`
    // contracts emit a Rust binding (the platform host uses them); `contracts/userspace/` contracts are
    // CADENZA-ONLY (a guest/reducer consumes them via self-reflection, the host never does) — validated but
    // NO Rust binding + no `contracts/mod.rs` entry. A source carries a `cadenza_only` flag = it lives under
    // `userspace/`.
    let read_cdz = |dir: &std::path::Path| -> Vec<PathBuf> {
        match std::fs::read_dir(dir) {
            Ok(rd) => {
                let mut v: Vec<PathBuf> = rd
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|x| x == "cdz"))
                    .collect();
                v.sort();
                v
            }
            // A missing subdir is empty, not an error (e.g. no userspace contracts yet).
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                eprintln!("xtask codegen: read contracts dir {}: {e}", dir.display());
                std::process::exit(1);
            }
        }
    };
    // (path, cadenza_only): kernel first (emit Rust), then userspace (validate-only).
    let mut sources: Vec<(PathBuf, bool)> = read_cdz(&contracts_dir.join("kernel"))
        .into_iter()
        .map(|p| (p, false))
        .collect();
    sources.extend(
        read_cdz(&contracts_dir.join("userspace"))
            .into_iter()
            .map(|p| (p, true)),
    );
    if !check && let Err(e) = std::fs::create_dir_all(&out_dir) {
        eprintln!("xtask codegen: create {}: {e}", out_dir.display());
        std::process::exit(1);
    }

    // A contract source may `import { contract-id } from "contract-id"` to export its own self-reflecting
    // contract-id (P4 self-reflection): `def <c>-id() = contract-id(Ast.module); export { <c>-id }`, so a
    // reducer can import another contract's id to route on. Per-file `cdz test <contract>.cdz` cannot resolve
    // that import — the lib lives in `guests/`, not beside the contract — so validate each contract from a
    // STAGING dir holding the contract sources ALONGSIDE a copy of `guests/contract-id.cdz`, where `cdz test`'s
    // same-directory module resolution finds it. `guests/contract-id.cdz` stays the single source (only copied
    // here); `cdz convert` reads the real source (it needs no import resolution). The staging dir is under
    // `target/` (gitignored) and is rebuilt each run so a renamed/removed contract leaves no stale sibling.
    let stage = paths.repo.join("target/codegen-contract-stage");
    let lib = paths
        .seed
        .join("crates/cdz-platform/guests/contract-id.cdz");
    let _ = std::fs::remove_dir_all(&stage);
    if let Err(e) = std::fs::create_dir_all(&stage) {
        eprintln!("xtask codegen: create staging dir {}: {e}", stage.display());
        std::process::exit(1);
    }
    let stage_copy = |from: &std::path::Path, to: PathBuf| {
        if let Err(e) = std::fs::copy(from, &to) {
            eprintln!(
                "xtask codegen: stage {} -> {}: {e}",
                from.display(),
                to.display()
            );
            std::process::exit(1);
        }
    };
    stage_copy(&lib, stage.join("contract-id.cdz"));
    for (src, _) in &sources {
        let file = src.file_name().expect("a contract file name");
        stage_copy(src, stage.join(file));
    }

    let mut names: Vec<String> = Vec::with_capacity(sources.len());
    for (src, cadenza_only) in &sources {
        let name = src
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_else(|| {
                eprintln!(
                    "xtask codegen: contract file has no usable name: {}",
                    src.display()
                );
                std::process::exit(1);
            })
            .to_string();
        let out = out_dir.join(format!("{name}.rs"));
        // Only a KERNEL contract emits Rust + is declared in `contracts/mod.rs`; a userspace one is
        // validated but has no Rust module.
        if !cadenza_only {
            names.push(name.clone());
        }

        // MTIME short-circuit (inner loop ONLY): when the generated file is newer than its source, neither
        // revalidate (which builds + runs `cdz`) nor regenerate — the committed file is current. This is a
        // SPEED optimization keyed on the `.cdz` SOURCE, so it is blind to a change in the RENDERING LOGIC
        // (this function): a codegen change leaves the committed `.rs` stale but mtime-fresh. That is a
        // footgun, so `--check` (the shared `xtask check` gate) does NOT take the short-circuit — it always
        // re-renders and content-compares, exactly like the ABI checks, so codegen-logic drift is a hard
        // failure rather than a silent skip. (Regression: the FIX B single-ctor elision left the kernel
        // contracts stale-but-mtime-fresh, and this gate reported them "up to date" until forced.)
        if !check && !cadenza_only && up_to_date(&out, src) {
            println!("xtask codegen: {} is up to date (mtime).", out.display());
            continue;
        }

        // Validate + run the source's `@test` conformance proofs (`cdz test`), then read its canonical AST
        // (`cdz convert`) — both via the `cdz` binary, so xtask carries no compiler dependency. `cdz test`
        // runs in BOTH regen and `--check` mode: the operator's intent is that the contracts execute their
        // own `@test`s in the gate ("the contracts could actually execute tests on themselves"), and this is
        // the only place they run. `cdz test`'s heap-value proofs need the value-heap RUNTIME + NFC in the
        // CAS store; `generate_runtime_abi` (which runs first, and already builds those components to hash
        // them) seeds them into the store (`seed_store_component`) — so `--check` in a bare CI job (which does
        // not run `cargo xtask build`) still resolves the runtime for the contract self-tests.
        let src_str = src.to_str().expect("a UTF-8 contract path");
        // Validate from the STAGED copy (beside contract-id.cdz) so a contract that imports the self-reflection
        // lib resolves it; `cdz convert` below still reads the real source (no import resolution needed).
        let staged = stage.join(src.file_name().expect("a contract file name"));
        let staged_str = staged.to_str().expect("a UTF-8 staged contract path");
        run_cdz(
            paths,
            &["test", staged_str],
            &format!("validate {}", src.display()),
        );
        // Userspace contract: validated above (its `@test`s ran), but emit NO Rust — a Cadenza guest consumes
        // it via self-reflection, the host never does.
        if *cadenza_only {
            println!(
                "xtask codegen: {} is userspace (contracts/userspace/) — validated, no Rust binding.",
                src.display()
            );
            continue;
        }
        let ast_bytes = run_cdz_capture(
            paths,
            &["convert", src_str, "--to", "binary"],
            &format!("read the AST of {}", src.display()),
        );
        let arenas = cadenza_ast::codec::decode(&ast_bytes).unwrap_or_else(|| {
            eprintln!(
                "xtask codegen: `cdz convert {} --to binary` did not produce a decodable AST",
                src.display()
            );
            std::process::exit(1);
        });

        let decls = type_decls(&arenas);
        if decls.is_empty() {
            eprintln!(
                "xtask codegen: {} declares no `type` — a contract schema needs at least one type",
                src.display()
            );
            std::process::exit(1);
        }

        let identity = contract_identity(paths, &stage, staged_str, &name);
        let body = format_tokens(render_schema(&arenas, &decls, &name, identity.as_ref()));
        let source = format!("{}{body}", contract_banner(&name));
        let summary = format!("{} type declarations, from {}", decls.len(), src.display());
        emit_or_check(
            &out,
            &source,
            check,
            "the contract Cadenza source",
            &summary,
        );
    }

    // The module file listing every generated contract, projected from the directory so a new contract
    // file wires itself in. Tiny and needs no `cdz`, so it is always (re)checked — no mtime short-circuit.
    let mod_rs = out_dir.join("mod.rs");
    let mod_src = format!(
        "{}{}",
        contracts_mod_banner(),
        format_tokens(render_contracts_mod(&names))
    );
    emit_or_check(
        &mod_rs,
        &mod_src,
        check,
        "the contracts directory listing",
        &format!("{} contract module(s)", names.len()),
    );
}

/// Whether `generated` exists and is at least as new as `source` — the mtime freshness test that lets
/// codegen skip revalidation + regeneration of an unchanged contract. Missing/unreadable → not fresh.
fn up_to_date(generated: &std::path::Path, source: &std::path::Path) -> bool {
    let (Ok(g), Ok(s)) = (std::fs::metadata(generated), std::fs::metadata(source)) else {
        return false;
    };
    match (g.modified(), s.modified()) {
        (Ok(gm), Ok(sm)) => gm >= sm,
        _ => false,
    }
}

/// Run `cdz <args>` (via `cargo run -p cdz`), inheriting stdio; exit non-zero if it fails. `what` names
/// the step for the failure message. Delegating to the binary keeps the compiler out of xtask's deps.
fn run_cdz(paths: &Paths, args: &[&str], what: &str) {
    let status = cdz_command(paths, args).status().unwrap_or_else(|e| {
        panic!(
            "xtask codegen: could not run `cdz {}` ({what}): {e}",
            args.join(" ")
        )
    });
    if !status.success() {
        eprintln!("xtask codegen: `cdz {}` failed ({what})", args.join(" "));
        std::process::exit(1);
    }
}

/// Run `cdz <args>` and return its stdout bytes (stderr inherited); exit non-zero if it fails. Used to
/// capture `cdz convert --to binary` (the canonical AST).
fn run_cdz_capture(paths: &Paths, args: &[&str], what: &str) -> Vec<u8> {
    let out = cdz_command(paths, args)
        .stderr(std::process::Stdio::inherit())
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "xtask codegen: could not run `cdz {}` ({what}): {e}",
                args.join(" ")
            )
        });
    if !out.status.success() {
        eprintln!("xtask codegen: `cdz {}` failed ({what})", args.join(" "));
        std::process::exit(1);
    }
    out.stdout
}

/// A `cargo run -p cdz -- <args>` command rooted at the repo, so codegen always drives a freshly built
/// `cdz` rather than a stale binary on the PATH. `--quiet` keeps cargo's own chatter off stdout (so a
/// captured `--to binary` payload is clean); build progress still goes to stderr.
fn cdz_command(paths: &Paths, args: &[&str]) -> std::process::Command {
    let mut cmd = std::process::Command::new("cargo");
    cmd.current_dir(&paths.repo)
        .args(["run", "--quiet", "--release", "-p", "cdz", "--"])
        .args(args);
    cmd
}

/// The contract's `type` declaration occurrences, in source order. A bare `.cdz` source canonicalizes to
/// a root `(do <form>…)`, and a source comment wraps the form after it as `(comment <text>… <form>)`, so a
/// declaration can sit under a comment chain rather than directly under the `do`. Walk the `do`'s children,
/// unwrapping any comment chain to the form it carries, and collect the `type`-headed ones. Type bodies
/// never contain a `type`, so this needs no deeper descent.
fn type_decls(arenas: &Arenas) -> Vec<StructId> {
    let mut out = Vec::new();
    let Struct::List(items) = arenas.get(arenas.root) else {
        return out;
    };
    // Skip the `do` head; each remaining child is a top-level form (possibly comment-wrapped).
    for &child in items.iter().skip(1) {
        let form = unwrap_comment(arenas, child);
        if arenas.head_name(form) == Some("type") {
            out.push(form);
        }
    }
    out
}

/// Unwrap a comment chain `(comment <text>… <form>)` to the form it carries (the last child), following
/// nested comments to the innermost wrapped form. A non-comment id is returned unchanged.
fn unwrap_comment(arenas: &Arenas, id: StructId) -> StructId {
    let mut id = id;
    while arenas.head_name(id) == Some("comment") {
        match arenas.get(id) {
            Struct::List(items) => match items.last() {
                Some(&last) if last != id => id = last,
                _ => break,
            },
            Struct::Atom(_) => break,
        }
    }
    id
}

/// Render a contract's generated schema module: the `schema(b)` function that reconstructs the type
/// declarations for `Contract::new`, plus — for every declared constructor — a value BUILDER and a
/// matching READER. The builders/readers name the constructor and its fields and defer the canonical value
/// SHAPE to `crate::contract_value`, so both the schema and the value marshalling are generated from the
/// one source and cannot drift from each other or from the compiler's canonical encoding.
fn render_schema(
    arenas: &Arenas,
    decls: &[StructId],
    name: &str,
    identity: Option<&(String, String, String)>,
) -> TokenStream {
    let mut stmts: Vec<TokenStream> = Vec::new();
    let mut counter = 0usize;
    let decl_idents: Vec<syn::Ident> = decls
        .iter()
        .map(|&d| emit_node(arenas, d, &mut stmts, &mut counter))
        .collect();

    let bindings = decls.iter().flat_map(|&d| emit_value_bindings(arenas, d));

    // When the source declares its identity (the `@!contract`/`@!input`/`@!output` pragmas), generate the
    // `contract()` constructor from them — so a contract's name and its input/output type references live
    // ONLY in the `.cdz`, and the platform's `*_contract()` calls this instead of restating the strings.
    // A `.cdz` with no `@!contract` pragma (a non-contract schema, if one ever exists) generates only the
    // schema.
    let contract_fn = match identity {
        Some((contract_name, input, output)) => quote! {
            #[doc = " The contract this module declares — built from its `@!contract` / `@!input` /"]
            #[doc = " `@!output` pragmas and its schema. The one place the contract's name and input/output"]
            #[doc = " type references live is the `.cdz` source; `*_contract()` calls this."]
            pub fn contract() -> crate::Contract {
                crate::Contract::new(crate::Str::from_static(#contract_name), schema, #input, #output)
            }
        },
        None => quote! {},
    };

    let doc = format!(
        " The `{name}` contract's schema: its named Cadenza type declarations, in source order, ready"
    );
    quote! {
        // A generated bindings surface: a builder + reader for every constructor. Not every consumer uses
        // every one (e.g. the output type's constructors until the kernel produces that outcome), so the
        // unused-code lint is allowed for the whole generated module rather than per item.
        #![allow(dead_code)]

        // A contract every one of whose constructors is a single-constructor sum ELIDED to its bare scalar
        // payload (e.g. `timer`, whose `Envelope`/`Event` are both `| C(UInt64)`) never names `v` — the
        // builders return the payload directly — so this import is genuinely unused there. Allow it rather
        // than emit the import conditionally.
        #[allow(unused_imports)]
        use crate::contract_value as v;
        use cadenza_ast::ast::{Arenas, Builder, StructId};

        #[doc = #doc]
        #[doc = " to hand to `Contract::new`. Generated from the contract's Cadenza source, which `cdz`"]
        #[doc = " typechecks and runs the conformance tests of at codegen time, so this is provably a"]
        #[doc = " valid Cadenza schema."]
        pub fn schema(b: &mut Builder) -> Vec<StructId> {
            #(#stmts)*
            vec![#(#decl_idents),*]
        }

        #contract_fn

        #(#bindings)*
    }
}

/// Read a contract's identity — its NAME and its INPUT / OUTPUT type-name references — by COMPILING and
/// EXECUTING the contract's `descriptor()` and reading the folded descriptor record (operator 2026-08-27:
/// "the codegen should call the compiled module, get the descriptor, and then codegen rust based on the
/// descriptor that calls the `Contract::new`"). This REPLACES the former `@!contract`/`@!input`/`@!output`
/// pragma read (those module pragmas are deprecated and removed — the identity now flows through the guest's
/// own `descriptor()` self-reflection, the single source of truth). The `staged` contract is compiled together
/// with the staged `contract-id` lib it imports into a component exporting `descriptor`, run with `cdz run
/// --format binary-ast` (which emits the descriptor record as the canonical binary AST — the universal
/// `cadenza-ast` exchange form), and decoded; `cdz_contract::identity_from_descriptor` reads the name +
/// input/output type names out (the descriptor's `input`/`output` fields are `Ast.encode(Ast.Name(<type>))`,
/// decoded back to the type-name symbol). The generated `contract()` still calls `Contract::new(name, types,
/// input, output)` — the Rust runtime `Contract` — from these, so the id it computes is byte-identical to what
/// the pragma read produced. Every kernel contract exports `descriptor()`, so a compile/run failure here is a
/// hard error (a kernel contract must have a runnable descriptor), via `run_cdz`/`run_cdz_capture`.
fn contract_identity(
    paths: &Paths,
    stage: &Path,
    staged_str: &str,
    name: &str,
) -> Option<(String, String, String)> {
    let wasm = stage.join(format!("{name}.wasm"));
    let wasm_str = wasm.to_str().expect("a UTF-8 staged wasm path");
    let lib = stage.join("contract-id.cdz");
    let lib_str = lib.to_str().expect("a UTF-8 staged lib path");
    // Compile the contract + the staged `contract-id` lib it imports into a component exporting `descriptor`.
    run_cdz(
        paths,
        &[
            "compile", staged_str, lib_str, "--entry", name, "-o", wasm_str,
        ],
        &format!("compile {name} to execute its descriptor()"),
    );
    // Run it, emitting the descriptor record as canonical binary AST, then decode + read (name, input, output).
    let doc = run_cdz_capture(
        paths,
        &["run", wasm_str, "--format", "binary-ast"],
        &format!("execute the descriptor() of {name}"),
    );
    let value = cadenza_ast::codec::decode(&doc)?;
    cdz_contract::identity_from_descriptor(&value)
}

/// A declared constructor's shape, as introspected from a `(type T …)` declaration.
enum Ctor {
    /// A variant with no payload — `C`.
    Nullary,
    /// A variant carrying a single non-record payload — `C(SomeType)`.
    Single,
    /// A variant carrying a record — `C(Record(f0: T0, …))` — with its field names in declared order.
    Record(Vec<String>),
}

/// Emit a value builder + reader for every constructor of the type declaration `decl` (a `(type T …)`).
/// The builder constructs a canonical value of that constructor; the reader is its exact inverse. Both are
/// thin wrappers over `crate::contract_value` (aliased `v`), which owns the canonical forms.
fn emit_value_bindings(arenas: &Arenas, decl: StructId) -> Vec<TokenStream> {
    let Struct::List(items) = arenas.get(decl) else {
        return Vec::new();
    };
    // (type <name> <variant>…) — the type name is the child after the `type` head.
    let Some(ty) = items.get(1).and_then(|&n| arenas.as_name(n)) else {
        return Vec::new();
    };
    // A SINGLE-constructor sum elides its constructor in the canonical Value form (the payload directly,
    // framed only by the root ascription), matching the compiler's `Value.encode`; a multi-constructor sum
    // keeps its bare-name constructor `(Ctor …)`. Count the variants so `emit_ctor` picks the right shape.
    let variants: Vec<StructId> = items.iter().skip(2).copied().collect();
    let single = variants.len() == 1;
    variants
        .iter()
        .filter_map(|&var| emit_ctor(arenas, ty, var, single))
        .collect()
}

/// Emit the builder + reader for one variant occurrence `var` of type `ty`. `None` if the variant is not a
/// name or `(Ctor …)` form (a parse invariant changed).
fn emit_ctor(arenas: &Arenas, ty: &str, var: StructId, single: bool) -> Option<TokenStream> {
    // A nullary variant is a bare name; a payload-carrying one is `(Ctor <payload>)`.
    let (ctor, shape) = if let Some(ctor) = arenas.as_name(var) {
        (ctor.to_string(), Ctor::Nullary)
    } else {
        let ctor = arenas.head_name(var)?.to_string();
        let Struct::List(items) = arenas.get(var) else {
            return None;
        };
        match items.get(1) {
            None => (ctor, Ctor::Nullary),
            Some(&payload) if arenas.head_name(payload) == Some("Record") => {
                (ctor, Ctor::Record(record_field_names(arenas, payload)))
            }
            Some(_) => (ctor, Ctor::Single),
        }
    };

    let build = syn::Ident::new(
        &format!("{}_{}", to_snake(ty), to_snake(&ctor)),
        Span::call_site(),
    );
    let doc_build = format!(" Build a canonical `{ty}.{ctor}` value.");
    Some(match shape {
        Ctor::Nullary => {
            let is = syn::Ident::new(&format!("is_{build}"), Span::call_site());
            let doc_read = format!(" Whether `id` is a `{ty}.{ctor}` value.");
            if single {
                // Single-constructor nullary sum: the constructor is ELIDED — the value IS the bare `unit`
                // atom (the compiler's `Value.encode` of the erased Unit payload), framed only by the root
                // ascription `(: unit T)`, matching the scalar/record single-ctor elision above.
                quote! {
                    #[doc = #doc_build]
                    pub fn #build(b: &mut Builder) -> StructId {
                        v::unit(b)
                    }
                    #[doc = #doc_read]
                    pub fn #is(arenas: &Arenas, id: StructId) -> bool {
                        v::is_unit(arenas, id)
                    }
                }
            } else {
                quote! {
                    #[doc = #doc_build]
                    pub fn #build(b: &mut Builder) -> StructId {
                        v::qctor(b, #ty, #ctor, vec![])
                    }
                    #[doc = #doc_read]
                    pub fn #is(arenas: &Arenas, id: StructId) -> bool {
                        v::as_qctor(arenas, id, #ty, #ctor).is_some_and(|t| t.is_empty())
                    }
                }
            }
        }
        Ctor::Single => {
            let as_ = syn::Ident::new(&format!("as_{build}"), Span::call_site());
            let doc_read = format!(" Read the payload of a `{ty}.{ctor}` value, or `None`.");
            if single {
                // Single-constructor sum: the constructor is ELIDED — the value IS the payload directly (the
                // canonical Value form frames it by the root ascription, not a `(Ctor …)` wrapper).
                quote! {
                    #[doc = #doc_build]
                    pub fn #build(_b: &mut Builder, x: StructId) -> StructId {
                        x
                    }
                    #[doc = #doc_read]
                    pub fn #as_(_arenas: &Arenas, id: StructId) -> Option<StructId> {
                        Some(id)
                    }
                }
            } else {
                quote! {
                    #[doc = #doc_build]
                    pub fn #build(b: &mut Builder, x: StructId) -> StructId {
                        v::qctor(b, #ty, #ctor, vec![x])
                    }
                    #[doc = #doc_read]
                    pub fn #as_(arenas: &Arenas, id: StructId) -> Option<StructId> {
                        let t = v::as_qctor(arenas, id, #ty, #ctor)?;
                        let [x] = <[StructId; 1]>::try_from(t).ok()?;
                        Some(x)
                    }
                }
            }
        }
        Ctor::Record(fields) => {
            // A named-fields struct so both the builder and reader name the record's fields rather than
            // being positional (review: "generate rust structs so we can name the expected fields").
            let rec_struct = syn::Ident::new(
                &format!("{}{}", to_pascal(ty), to_pascal(&ctor)),
                Span::call_site(),
            );
            let field_idents: Vec<syn::Ident> = fields
                .iter()
                .map(|f| syn::Ident::new(&to_snake(f), Span::call_site()))
                .collect();
            let field_names: Vec<&str> = fields.iter().map(String::as_str).collect();
            let as_ = syn::Ident::new(&format!("as_{build}"), Span::call_site());
            let struct_doc =
                format!(" The fields of a `{ty}.{ctor}` value — each a built value occurrence.");
            let doc_read = format!(" Read a `{ty}.{ctor}` value's fields by name, or `None`.");
            // Single-constructor sum: the constructor is ELIDED — the value IS the record directly (framed by
            // the root ascription). Multi-constructor: the record is wrapped in the bare-name `(Ctor <record>)`.
            let (build_body, read_bind) = if single {
                (
                    quote! { v::record(b, vec![#((#field_names, fields.#field_idents)),*]) },
                    quote! { let rec = id; },
                )
            } else {
                (
                    quote! {
                        let rec = v::record(b, vec![#((#field_names, fields.#field_idents)),*]);
                        v::qctor(b, #ty, #ctor, vec![rec])
                    },
                    quote! {
                        let t = v::as_qctor(arenas, id, #ty, #ctor)?;
                        let [rec] = <[StructId; 1]>::try_from(t).ok()?;
                    },
                )
            };
            quote! {
                #[doc = #struct_doc]
                pub struct #rec_struct {
                    #(pub #field_idents: StructId,)*
                }
                #[doc = #doc_build]
                pub fn #build(b: &mut Builder, fields: #rec_struct) -> StructId {
                    #build_body
                }
                #[doc = #doc_read]
                pub fn #as_(arenas: &Arenas, id: StructId) -> Option<#rec_struct> {
                    #read_bind
                    Some(#rec_struct {
                        #(#field_idents: v::record_field(arenas, rec, #field_names)?,)*
                    })
                }
            }
        }
    })
}

/// The field names of a record type `(Record (: f0 T0) (: f1 T1) …)`, in declared order.
fn record_field_names(arenas: &Arenas, record: StructId) -> Vec<String> {
    let Struct::List(items) = arenas.get(record) else {
        return Vec::new();
    };
    items
        .iter()
        .skip(1) // the `Record` head
        .filter_map(|&f| {
            let kv = arenas.as_form(f, ":")?; // `(: <name> <type>)`
            arenas.as_name(*kv.first()?).map(str::to_string)
        })
        .collect()
}

/// A Cadenza name to a PascalCase Rust type identifier: drop `-`/`_` separators and capitalize the first
/// letter of each segment (`Envelope` → `Envelope`, `deliver-envelope` → `DeliverEnvelope`). A type and
/// constructor name are concatenated to name a record's field-struct (`Event` + `Message` → `EventMessage`).
fn to_pascal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut cap = true;
    for c in s.chars() {
        if c == '-' || c == '_' {
            cap = true;
        } else if cap {
            out.extend(c.to_uppercase());
            cap = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// A Cadenza name to a snake_case Rust identifier: `-` → `_`, and an underscore before each interior
/// capital (`MissingHandler` → `missing_handler`, `deliver-envelope` → `deliver_envelope`).
fn to_snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, c) in s.chars().enumerate() {
        if c == '-' {
            out.push('_');
        } else if c.is_ascii_uppercase() {
            if i != 0 && !out.ends_with('_') {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Emit the builder statements that reconstruct the node at `id`, returning the identifier bound to it.
/// Post-order: children first (so their identifiers exist), then the parent's `b.list`. A type
/// declaration is names and lists only; any other atom is a bug in the source classification.
fn emit_node(
    arenas: &Arenas,
    id: StructId,
    stmts: &mut Vec<TokenStream>,
    counter: &mut usize,
) -> syn::Ident {
    if let Some(name) = arenas.as_name(id) {
        let ident = fresh_ident(counter);
        let name = name.to_string();
        stmts.push(quote!(let #ident = b.name(#name);));
        return ident;
    }
    match arenas.get(id) {
        Struct::List(items) => {
            let items = items.clone();
            let children: Vec<syn::Ident> = items
                .iter()
                .map(|&c| emit_node(arenas, c, stmts, counter))
                .collect();
            let ident = fresh_ident(counter);
            stmts.push(quote!(let #ident = b.list(vec![#(#children),*]);));
            ident
        }
        Struct::Atom(_) => panic!(
            "xtask codegen: a contract `type` declaration holds a non-name atom — a type declaration is \
             built from names and lists only (a compiler/parse invariant changed)"
        ),
    }
}

/// Render `contracts/mod.rs`: one `pub mod <name>;` per contract, in sorted order.
fn render_contracts_mod(names: &[String]) -> TokenStream {
    let mods = names.iter().map(|n| {
        let ident = syn::Ident::new(&n.replace('-', "_"), Span::call_site());
        quote!(pub mod #ident;)
    });
    quote! { #(#mods)* }
}

/// A fresh temporary identifier `v0`, `v1`, … for the builder statements.
fn fresh_ident(counter: &mut usize) -> syn::Ident {
    let ident = syn::Ident::new(&format!("v{counter}"), Span::call_site());
    *counter += 1;
    ident
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
    rustfmt_stdin(&pretty).unwrap_or(pretty)
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

/// The `//!` banner prepended to a generated `contracts/<name>.rs` — the "do-not-edit / regenerate"
/// notice, plus what the file is and where it comes from.
fn contract_banner(name: &str) -> String {
    format!(
        "//! @generated by `cargo xtask codegen` from cdz-platform/contracts/{name}.cdz — DO NOT hand-edit.\n\
         //!\n\
         //! The `{name}` contract's schema as Cadenza-AST builder calls: `schema(b)` reconstructs the\n\
         //! contract's named type declarations, which the platform hands to `Contract::new`. The source is\n\
         //! real Cadenza that `cargo xtask codegen` validates + runs the conformance tests of (via the\n\
         //! `cdz` binary) before generating this file, so the schema is provably valid Cadenza and a\n\
         //! marshalled value type-ascribes against it. Edit the schema in `{name}.cdz`, then regenerate\n\
         //! with `cargo xtask codegen`; `cargo xtask codegen --check` (a hard gate in `xtask check`) fails\n\
         //! if this file is stale. Plain builder calls — no dependency on the compiler, so it ships in\n\
         //! `cdz-platform`.\n\n"
    )
}

/// The `//!` banner prepended to the generated `contracts/mod.rs` — the module listing every contract.
fn contracts_mod_banner() -> String {
    "//! @generated by `cargo xtask codegen` from the cdz-platform/contracts/ directory — DO NOT hand-edit.\n\
     //!\n\
     //! One `pub mod` per built-in contract, projected from the contract sources so a new\n\
     //! `contracts/<name>.cdz` wires itself in on the next `cargo xtask codegen`.\n\n"
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
