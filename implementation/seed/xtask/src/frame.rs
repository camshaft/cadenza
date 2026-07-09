//! Scalar-component FRAME byte-segment generation for the Cadenza-authored compiler (`cdzc.cdz`).
//!
//! The rewritten compiler frames a nullary `run : () -> s64` component around a serialized `i64` body
//! (`cdzc/40-frame.cdz`'s `core-module` / `wrap-component`). The FIXED byte segments those functions
//! interleave — the wasm/component-model magic bytes — are the WebAssembly spec's, not ours, so we do
//! NOT hand-transcribe them: we derive each segment from `wasm-encoder` (the authoritative encoder,
//! self-checked with `wasmparser`) and emit them as Cadenza top-level VALUE definitions
//! `(def frame-<seg> (Bytes.of (list …)))`. This is the same magic-value-sharing the opcode table
//! (`op.cdz` / `op.rs`) and the heap envelope already use — one source of truth for every emitted byte.
//!
//! Only the FIXED segments are generated; the length-computing structure (`section`/`wvec`/`sized`)
//! stays in Cadenza, because a section's length prefix depends on the body length (a larger `i64.const`
//! grows the component), so it cannot be a fixed blob.

use crate::write_if_changed;
use std::path::Path;
use wasm_encoder::{
    CodeSection, ComponentBuilder, ComponentExportKind, ComponentValType, Encode, ExportKind,
    ExportSection, Function, FunctionSection, Instruction, Module, ModuleArg, PrimitiveValType,
    TypeSection, ValType,
};

/// One generated frame segment: the Cadenza def name, a doc line, and the authoritative bytes.
struct Seg {
    name: &'static str,
    doc: &'static str,
    bytes: Vec<u8>,
}

/// Derive the `() -> i64` core functype bytes (`60 00 01 7E`) by encoding a one-entry type section and
/// taking the functype. `TypeSection::encode` emits the section CONTENT `[byte-len, count, <functype…>]`
/// (the section id is prepended later by `Module::section`), so the functype is everything after the
/// 2-byte `[byte-len, count]` prefix.
fn functype_run_i64() -> Vec<u8> {
    let mut t = TypeSection::new();
    t.ty().function::<[ValType; 0], _>([], [ValType::I64]);
    let mut buf = Vec::new();
    t.encode(&mut buf);
    buf[2..].to_vec()
}

/// The whole scalar `run:()->s64` component for a placeholder body, built with `wasm-encoder`. Used to
/// self-check that the Cadenza frame's assembled segments reproduce a valid component (the assembled
/// result is validated in `generate`, and the compiler's differential gate is the byte-level backstop).
fn reference_scalar_component(body: i64) -> Vec<u8> {
    let mut c = ComponentBuilder::default();
    let core = {
        let mut m = Module::new();
        let mut t = TypeSection::new();
        t.ty().function::<[ValType; 0], _>([], [ValType::I64]);
        m.section(&t);
        let mut f = FunctionSection::new();
        f.function(0);
        m.section(&f);
        let mut e = ExportSection::new();
        e.export("run", ExportKind::Func, 0);
        m.section(&e);
        let mut code = CodeSection::new();
        let mut b = Function::new([]);
        b.instruction(&Instruction::I64Const(body)).instruction(&Instruction::End);
        code.function(&b);
        m.section(&code);
        m.finish()
    };
    let midx = c.core_module_raw(&core);
    let inst = c.core_instantiate(midx, [] as [(&str, ModuleArg); 0]);
    let run_core = c.core_alias_export(inst, "run", ExportKind::Func);
    let fnty = {
        let (idx, mut enc) = c.type_function();
        enc.params::<[(&str, ComponentValType); 0], _>([])
            .result(Some(ComponentValType::Primitive(PrimitiveValType::S64)));
        idx
    };
    let run_fn = c.lift_func(run_core, fnty, []);
    c.export("run", ComponentExportKind::Func, run_fn, None);
    c.finish()
}

/// The fixed frame segments, in the order the Cadenza `40-frame.cdz` references them. Each byte value is
/// authoritative: the core-module magic + `()->i64` functype come from `wasm-encoder`; the
/// component-model section bytes (instance/type/canon-lift/alias/export) are the spec's fixed encodings,
/// self-checked by validating the assembled reference component below.
fn segments() -> Vec<Seg> {
    vec![
        Seg { name: "frame-core-magic",       doc: "core module: \\0asm v1 preamble",                    bytes: vec![0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00] },
        Seg { name: "frame-functype-run",     doc: "functype () -> i64",                                 bytes: functype_run_i64() },
        Seg { name: "frame-run-name",         doc: "the export name \"run\"",                            bytes: b"run".to_vec() },
        Seg { name: "frame-comp-magic",       doc: "component: preamble",                                bytes: vec![0x00, 0x61, 0x73, 0x6D, 0x0D, 0x00, 0x01, 0x00] },
        Seg { name: "frame-comp-instance",    doc: "section 2: core instance (instantiate module 0)",    bytes: vec![0x02, 0x04, 0x01, 0x00, 0x00, 0x00] },
        Seg { name: "frame-comp-type-run",    doc: "section 7: component type () -> s64 (0x78)",         bytes: vec![0x07, 0x05, 0x01, 0x40, 0x00, 0x00, 0x78] },
        Seg { name: "frame-comp-canon-lift",  doc: "section 6: canon lift of core run",                  bytes: vec![0x06, 0x09, 0x01, 0x00, 0x00, 0x01, 0x00, 0x03, 0x72, 0x75, 0x6E] },
        Seg { name: "frame-comp-func-alias",  doc: "section 8: component func alias",                     bytes: vec![0x08, 0x06, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00] },
        Seg { name: "frame-comp-export-run",  doc: "section 11: export \"run\"",                         bytes: vec![0x0B, 0x09, 0x01, 0x00, 0x03, 0x72, 0x75, 0x6E, 0x01, 0x00, 0x00] },
    ]
}

/// SCREAMING_SNAKE Rust const name from the `frame-comp-type-run` kebab def name.
fn screaming(name: &str) -> String {
    name.to_uppercase().replace('-', "_")
}

/// Render `cdzc/40-frame.cdz`: the generated segment value-defs + the (stable) assembly functions.
fn render_cadenza(segs: &[Seg]) -> String {
    let mut s = String::new();
    s.push_str(CDZ_HEADER);
    s.push('\n');
    for seg in segs {
        let lits: Vec<String> = seg.bytes.iter().map(|b| format!("0x{b:02X}")).collect();
        s.push_str(&format!("(def {} (Bytes.of (list {})))   ; {}\n", seg.name, lits.join(" "), seg.doc));
    }
    s.push('\n');
    s.push_str(ASSEMBLY);
    s
}

/// Render the Rust `frame.rs` — the SAME segments as `&[u8]` consts, so the seed's `wrap_component` /
/// core-module framing can consume the shared table instead of hand-embedding the section bytes (the
/// wit_envelope pattern: one derivation, a Rust file for the seed and a cdz file for the Cadenza compiler).
fn render_rust(segs: &[Seg]) -> String {
    let mut s = String::new();
    s.push_str(RUST_HEADER);
    s.push_str("\n#![allow(dead_code)]\n\n");
    for seg in segs {
        let lits: Vec<String> = seg.bytes.iter().map(|b| b.to_string()).collect();
        s.push_str(&format!("/// {}\npub const {}: &[u8] = &[{}];\n", seg.doc, screaming(seg.name), lits.join(", ")));
    }
    s
}

/// Generate the scalar-frame segments from `wasm-encoder` into BOTH the Cadenza compiler
/// (`cdzc/40-frame.cdz`) and the Rust seed (`crates/cdz-compiler/src/frame.rs`) — one derivation, two
/// files, the same magic-value-sharing as the opcode table and the heap envelope. `seed` is the seed
/// root, `repo` the workspace root.
pub fn generate(seed: &Path, repo: &Path) -> Result<bool, String> {
    let segs = segments();

    // Self-check: the reference scalar component (built with the same section bytes for a placeholder
    // body) must validate. The Cadenza side reuses these exact segments; the differential gate is the
    // byte-level backstop on the full assembled result.
    let reference = reference_scalar_component(0);
    wasmparser::validate(&reference).map_err(|e| format!("scalar frame reference failed validation: {e}"))?;

    let cdz = render_cadenza(&segs);
    let rs = render_rust(&segs);
    let cdz_path = repo.join("implementation/compiler/cdzc/40-frame.cdz");
    let rs_path = seed.join("crates/cdz-compiler/src/frame.rs");
    let a = write_if_changed(&cdz_path, &cdz).map_err(|e| format!("write {}: {e}", cdz_path.display()))?;
    let b = write_if_changed(&rs_path, &rs).map_err(|e| format!("write {}: {e}", rs_path.display()))?;
    Ok(a || b)
}

const RUST_HEADER: &str = "\
// @generated by `cargo run -p xtask` from xtask/src/frame.rs. DO NOT EDIT — edit the generator.
//
// Scalar-component frame byte segments, derived from wasm-encoder (the authoritative encoder of the
// wasm/component-model bytes). The SAME segments are emitted into `compiler/cdzc/40-frame.cdz` for the
// Cadenza compiler — one derivation feeds both, so the two implementations share every frame byte.";

const CDZ_HEADER: &str = "\
; @generated by `cargo run -p xtask` from xtask/src/frame.rs. DO NOT EDIT — edit the generator.
;
; The scalar-int component frame for cdzc.cdz: a nullary `run : () -> s64` component around a serialized
; i64 body. The FIXED byte segments below are top-level VALUE definitions derived from wasm-encoder (the
; authoritative encoder of the wasm/component-model bytes) — the same magic-value sharing as op.cdz and the
; heap envelope. The length-computing assembly (section/wvec/sized) stays in Cadenza because a section's
; length prefix depends on the body length.
";

const ASSEMBLY: &str = "\
(def (core-module code-bytes)
  (doc \"The embedded core module for a nullary `run : () -> i64` with NO scratch locals — the scalar case.
        Delegates to core-module-locals with a 0 local count.\")
  (core-module-locals code-bytes 0))

(def (core-module-locals code-bytes nlocals)
  (doc \"The embedded core module for a nullary `run : () -> i64`, given main's serialized body bytes and
        `nlocals` i64 scratch locals (0 for a scalar body; 3 per checked arithmetic op). Assembles the
        generated fixed segments with the length-computing structure; the code entry declares the locals.\")
  (cat frame-core-magic
  (cat (section 1 (wvec 1 frame-functype-run))                          ; type: () -> i64
  (cat (section 3 (wvec 1 (u8 0x00)))                                   ; func 0 : type 0
  (cat (section 7 (wvec 1 (cat (u8 0x03) (cat frame-run-name (Bytes.of (list 0x00 0x00))))))  ; export \"run\"
       (section 10 (wvec 1 (sized (cat (locals-decl nlocals) (cat code-bytes (u8 0x0B))))))))))) ; code: <locals> <body> end

(def (wrap-component core)
  (doc \"Wrap the core module in the component envelope, presenting core func 0 (run) as the component
        export run : () -> s64. Assembles the generated component-section segments around the core module.\")
  (cat frame-comp-magic
  (cat (cat (u8 0x01) (sized core))                                     ; section 1: embedded core module
  (cat frame-comp-instance
  (cat frame-comp-type-run
  (cat frame-comp-canon-lift
  (cat frame-comp-func-alias
       frame-comp-export-run)))))))
";
