//! `xtask-codegen-wasm-abi` — the backend's wasm / component-model byte table (`wasm_abi.rs`): the core
//! opcode table, core+component valtype bytes, section ids, magic headers, functype form bytes. Carved out
//! of `xtask/src/codegen.rs` (v-xtask-decompose, the codegen→build-time-nix directive).
//!
//! The operator's CODEGEN-SEXPR model (ruled 2026-08-29): the flow is SEXPR → RUST, never rust → sexpr.
//! `data/wasm-abi.sexp` (top-level, OUTSIDE the rust compiler tree — language-independent, operator seq-173)
//! is the HAND-AUTHORED, committed, human-editable SOURCE OF TRUTH — NOTHING generates it.
//! `wasm-encoder` is ONLY the cross-check ORACLE the derived rust asserts against (a transcription typo in
//! the authored sexpr is caught by that assertion, NOT by re-extracting bytes from the encoder).
//! Modes:
//!   - `--from-sexpr` — produce `wasm_abi.rs` from the authoritative `wasm-abi.sexp`: `cdz convert` it to
//!     cadenza-ast BINARY (dogfoods cadenza-ast as the codegen IR, no-json), decode + walk (`read_sexpr_tables`),
//!     render. This IS the operator's sexpr → rust direction. Needs `cdz` from `CDZ_SEED_BIN_DIR`.
//!   - (default) — produce `wasm_abi.rs` by EXTRACTING each byte from `wasm-encoder`. This is a TEMPORARY
//!     PRE-FLIP BRIDGE ONLY — it is NOT a source of truth, and is kept solely so v-nix's `cdzWasmAbi`
//!     derivation stays green until it flips to `--from-sexpr` (their derivation window). It is
//!     byte-identical to the `--from-sexpr` output, which is why the flip is safe. To be removed on the flip.
//!   - `--oracle-check` — assert every opcode/valtype/section/magic byte in the AUTHORED `wasm-abi.sexp`
//!     matches the wasm-encoder oracle (catches a hand-authored transcription typo). v-nix wires it as a
//!     required nix check. (The derived-crate baked-in unit-test form of this is the redo in progress.)
//!
//! A `cdzWasmAbi` nix derivation runs this bin to produce `wasm_abi.rs` at build time (a build-phase overlay
//! copies it into rcdzc's src, so nothing generated is committed). Repo root from `CDZ_REPO_ROOT` (else cwd);
//! the first non-flag arg is the output path (the derivation passes it; default = the committed wasm_abi.rs).

use proc_macro2::{Span, TokenStream};
use quote::quote;
use std::path::PathBuf;
use xtask_codegen_support::{cdz_bin, format_tokens, rustfmt_stdin, sexpr_to_arenas};

fn main() {
    let repo = std::env::var_os("CDZ_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("current dir"));
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The authored source of truth lives OUTSIDE the rust compiler tree — a language-independent
    // top-level `data/` location (operator seq-173): the sexpr is the single source, the derived rust is
    // just one consumer, so it must not live under `implementation/seed/crates/rcdzc/…`.
    let sexpr = repo.join("data/wasm-abi.sexp");

    // ORACLE-CHECK (`--oracle-check`): assert the committed sexpr's BYTES match the wasm-encoder oracle
    // (the operator's inverted guarantee — a derived test catches a transcription typo). v-nix wires this
    // as a required nix check.
    if args.iter().any(|a| a == "--oracle-check") {
        let sexpr_tables = wasm_abi::read_sexpr_tables(&sexpr_to_arenas(&cdz_bin(&repo), &sexpr));
        let mismatches = wasm_abi::oracle_mismatches(&wasm_abi::collect(), &sexpr_tables);
        if !mismatches.is_empty() {
            eprintln!(
                "wasm-abi oracle-check FAILED — {} of the committed wasm-abi.sexp bytes do not match the \
                 wasm-encoder spec oracle (fix the sexpr):\n  {}",
                mismatches.len(),
                mismatches.join("\n  ")
            );
            std::process::exit(1);
        }
        println!(
            "wasm-abi oracle-check: ok — every opcode/valtype/section/magic byte in wasm-abi.sexp matches \
             the wasm-encoder oracle ({} opcodes, {} singles, {} magics).",
            sexpr_tables.opcodes.len(),
            sexpr_tables.singles.len(),
            sexpr_tables.magics.len()
        );
        return;
    }

    // CRATE-LIB (`--crate-lib`): emit the DERIVED CRATE's lib.rs — the operator's option-B target (the
    // `wasm-abi-table` seed crate). Same const table as `wasm_abi.rs`, but as a crate-root `lib.rs` (a
    // crate-level banner instead of the module banner) that rcdzc will DEP for the constants. Read from the
    // authoritative sexpr (SEXPR → RUST). The baked-in `#[cfg(test)]` oracle module (each const asserted
    // vs wasm-encoder, wasm-encoder isolated to crate B's dev-deps) is appended in a follow-on slice.
    if let Some(i) = args.iter().position(|a| a == "--crate-lib") {
        let out = args
            .get(i + 1)
            .map(PathBuf::from)
            .expect("--crate-lib needs an output path");
        // Constants from the AUTHORITATIVE sexpr (source of truth); the baked-in oracle tests re-derive
        // each byte from `wasm-encoder` (`collect`, which carries the constructor expressions) and assert
        // the sexpr-derived constants match. Both keyed by name, so the asserts reference the emitted consts.
        let tables = wasm_abi::read_sexpr_tables(&sexpr_to_arenas(&cdz_bin(&repo), &sexpr));
        let body = format!(
            "{}{}",
            format_tokens(wasm_abi::render(&tables)),
            wasm_abi::render_oracle_tests(&wasm_abi::collect())
        );
        // Canonicalize the whole body (constants + hand-emitted oracle module) through the pinned rustfmt
        // so the generated crate is fmt-clean; format_tokens already required rustfmt for the constants.
        let body = rustfmt_stdin(&body).unwrap_or(body);
        let source = format!("{}{}", crate_lib_banner(), body);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if let Err(e) = std::fs::write(&out, source) {
            eprintln!("xtask codegen: writing {}: {e}", out.display());
            std::process::exit(1);
        }
        println!("xtask codegen: wrote crate lib {}", out.display());
        return;
    }

    // Produce wasm_abi.rs (the byte table the backend consumes). The operator's flow is SEXPR → RUST:
    //   `--from-sexpr` — read the AUTHORITATIVE, hand-authored wasm-abi.sexp → cadenza-ast binary → walk →
    //                     render. THIS is the source-of-truth direction.
    //   default        — extract from the wasm-encoder crate. A TEMPORARY PRE-FLIP BRIDGE ONLY (not a source
    //                     of truth): byte-identical to `--from-sexpr`, kept green until v-nix flips cdzWasmAbi
    //                     to `--from-sexpr` + cdz in their derivation window, then this default is removed.
    let tables = if args.iter().any(|a| a == "--from-sexpr") {
        wasm_abi::read_sexpr_tables(&sexpr_to_arenas(&cdz_bin(&repo), &sexpr))
    } else {
        wasm_abi::collect()
    };
    let out = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            repo.join("implementation/seed/crates/rcdzc/src/backend/wasm/wasm_abi.rs")
        });
    let source = format!(
        "{}{}",
        wasm_abi_banner(),
        format_tokens(wasm_abi::render(&tables))
    );
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    if let Err(e) = std::fs::write(&out, source) {
        eprintln!("xtask codegen: writing {}: {e}", out.display());
        std::process::exit(1);
    }
    println!("xtask codegen: wrote {}", out.display());
}

/// The `//!` banner prepended to `wasm_abi.rs` — the "do-not-edit / regenerate" notice.
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

/// The `//!` banner for the DERIVED CRATE's `lib.rs` (the `wasm-abi-table` seed crate, operator option-B).
/// Same const table as `wasm_abi.rs`, but as a crate root that rcdzc DEPS for the constants (so rcdzc
/// carries no wasm-encoder dep). Generated from the authoritative `data/wasm-abi.sexp` (SEXPR → RUST).
fn crate_lib_banner() -> String {
    "//! @generated by `xtask-codegen-wasm-abi --crate-lib` from `data/wasm-abi.sexp` — DO NOT hand-edit.\n\
     //!\n\
     //! The backend's wasm / component-model byte table as a standalone crate: the core opcode table,\n\
     //! core + component valtype bytes, section ids, magic headers, and functype form bytes. Projected\n\
     //! from the AUTHORITATIVE, hand-authored `data/wasm-abi.sexp` (the language-independent source of\n\
     //! truth); `rcdzc` depends on this crate for the constants, so the compiler carries no `wasm-encoder`\n\
     //! dependency. A `#[cfg(test)]` oracle module (each byte asserted against the `wasm-encoder` spec\n\
     //! encoder, isolated to this crate's dev-dependencies) guards the authored table against a\n\
     //! transcription typo.\n\n"
        .to_string()
}

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
        /// The Rust EXPRESSION (as source tokens) that re-derives this byte from `wasm-encoder`, for the
        /// derived crate's baked-in oracle test (e.g. `byte0(Instruction::I32Const(0))`). Empty for a
        /// table read back from the sexpr (no constructor) — those aren't the oracle-emission source.
        pub oracle_src: &'static str,
    }

    /// One generated named single-byte constant — a valtype, section id, export-kind, or functype
    /// form byte. `doc` is the `///` line explaining what the byte is.
    pub struct Single {
        pub ident: &'static str,
        pub byte: u8,
        pub doc: &'static str,
        /// The Rust expression re-deriving this byte from `wasm-encoder` for the oracle test (e.g.
        /// `byte0(ValType::I32)` or `u8::from(SectionId::Type)`). Empty = not oracle-emitted (a sexpr
        /// read-back, or a special like the functype-form tags covered by the nix oracle check).
        pub oracle_src: &'static str,
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
    // The table constructors are MACROS (not fns) so each row captures, alongside its extracted byte, the
    // Rust EXPRESSION that re-derives that byte from `wasm-encoder` (`stringify!`) — the `oracle_src` the
    // derived crate's baked-in test emits (`assert_eq!(CONST, <oracle_src>)`). The byte is still computed by
    // the same `opcode_of`/`one_byte`/`u8::from` as before, so `collect()`'s output is byte-identical.
    /// An opcode row: byte from encoding `$insn`; oracle re-derives via `byte0($insn)`.
    macro_rules! op {
        ($id:expr, $insn:expr $(,)?) => {
            Opcode {
                ident: $id,
                byte: opcode_of($id, $insn),
                oracle_src: stringify!(byte0($insn)),
            }
        };
    }
    /// A core-valtype single: byte from encoding the `ValType`; oracle via `byte0($vt)`.
    macro_rules! valtype {
        ($id:expr, $doc:expr, $vt:expr $(,)?) => {
            Single {
                ident: $id,
                byte: one_byte($id, $vt),
                doc: $doc,
                oracle_src: stringify!(byte0($vt)),
            }
        };
    }
    /// A component-primitive single: byte from encoding the `PrimitiveValType`; oracle via `byte0($p)`.
    macro_rules! prim {
        ($id:expr, $doc:expr, $p:expr $(,)?) => {
            Single {
                ident: $id,
                byte: one_byte($id, $p),
                doc: $doc,
                oracle_src: stringify!(byte0($p)),
            }
        };
    }
    /// A core-section-id single (the id is the enum's `u8` repr); oracle via `u8::from($id)`.
    macro_rules! core_sec {
        ($id:expr, $doc:expr, $sid:expr $(,)?) => {
            Single {
                ident: $id,
                byte: u8::from($sid),
                doc: $doc,
                oracle_src: stringify!(u8::from($sid)),
            }
        };
    }
    /// A component-section-id single; oracle via `u8::from($id)`.
    macro_rules! comp_sec {
        ($id:expr, $doc:expr, $sid:expr $(,)?) => {
            Single {
                ident: $id,
                byte: u8::from($sid),
                doc: $doc,
                oracle_src: stringify!(u8::from($sid)),
            }
        };
    }

    pub fn collect() -> Tables {
        // The opcode table: each backend `op::` constant paired with the `Instruction` whose encoding
        // yields its byte. One row per instruction the serializer emits (Lir → bytes).
        let opcodes = vec![
            op!("I32_CONST", Instruction::I32Const(0)),
            op!("I64_CONST", Instruction::I64Const(0)),
            op!("F64_CONST", Instruction::F64Const(0.0f64.into())),
            op!("F32_CONST", Instruction::F32Const(0.0f32.into())),
            // Float ARITHMETIC (f64/f32) — the machine ops a runtime `+.`/`-.`/`*.`/`/.` selects.
            // IEEE, never trapping (overflow → inf, x/0 → ±inf/NaN), so no overflow guard (unlike the
            // integer arith). Round-to-nearest-even is the hardware default the determinism contract pins.
            op!("F64_ADD", Instruction::F64Add),
            op!("F64_SUB", Instruction::F64Sub),
            op!("F64_MUL", Instruction::F64Mul),
            op!("F64_DIV", Instruction::F64Div),
            op!("F32_ADD", Instruction::F32Add),
            op!("F32_SUB", Instruction::F32Sub),
            op!("F32_MUL", Instruction::F32Mul),
            op!("F32_DIV", Instruction::F32Div),
            // Float EQUALITY (f64/f32) — a runtime float `=` (IEEE compare: -0.0 == 0.0, NaN ≠ NaN).
            op!("F64_EQ", Instruction::F64Eq),
            op!("F64_NE", Instruction::F64Ne),
            op!("F32_EQ", Instruction::F32Eq),
            op!("F32_NE", Instruction::F32Ne),
            // Float ORDERING (f64/f32) — runtime `< <= > >=` under IEEE PARTIAL order (operator ruling):
            // a NaN operand yields false (unordered — NaN is neither <, >, nor = anything), and -0.0
            // compares equal-under-ordering to +0.0 (`f64.le -0.0 0.0` = true). These are the RAW IEEE
            // compares, DISTINCT from the canonical-byte equality above (which the two relations disagree
            // with on NaN + signed zero — inherent to float).
            op!("F64_LT", Instruction::F64Lt),
            op!("F64_GT", Instruction::F64Gt),
            op!("F64_LE", Instruction::F64Le),
            op!("F64_GE", Instruction::F64Ge),
            op!("F32_LT", Instruction::F32Lt),
            op!("F32_GT", Instruction::F32Gt),
            op!("F32_LE", Instruction::F32Le),
            op!("F32_GE", Instruction::F32Ge),
            // Float width conversion (F5): `f32.demote_f64` narrows Float64→Float32 (rounds),
            // `f64.promote_f32` widens Float32→Float64 (exact). Int↔float conversions land with `of-int`.
            op!("F32_DEMOTE_F64", Instruction::F32DemoteF64),
            op!("F64_PROMOTE_F32", Instruction::F64PromoteF32),
            // INT→FLOAT conversion (`Float N.of-int`): signed i64 → f64/f32, round-to-nearest-even.
            op!("F64_CONVERT_I64_S", Instruction::F64ConvertI64S),
            op!("F32_CONVERT_I64_S", Instruction::F32ConvertI64S),
            // FLOAT→INT bit reinterpret (no value change) — the canonical-byte float `=` reinterprets
            // the bits to an integer to compare (NaN-canonicalizing bit compare, IEEE `f*.eq` won't do).
            op!("I32_REINTERPRET_F32", Instruction::I32ReinterpretF32),
            op!("I64_REINTERPRET_F64", Instruction::I64ReinterpretF64),
            op!("IF", Instruction::If(wasm_encoder::BlockType::Empty)),
            op!("ELSE", Instruction::Else),
            op!("END", Instruction::End),
            op!("SELECT", Instruction::Select),
            op!("DROP", Instruction::Drop),
            op!("BLOCK", Instruction::Block(wasm_encoder::BlockType::Empty)),
            op!("LOOP", Instruction::Loop(wasm_encoder::BlockType::Empty)),
            op!("BR", Instruction::Br(0)),
            op!("BR_IF", Instruction::BrIf(0)),
            op!(
                "BR_TABLE",
                Instruction::BrTable(std::borrow::Cow::Borrowed(&[]), 0),
            ),
            op!("LOCAL_GET", Instruction::LocalGet(0)),
            op!("LOCAL_SET", Instruction::LocalSet(0)),
            op!("LOCAL_TEE", Instruction::LocalTee(0)),
            op!("GLOBAL_GET", Instruction::GlobalGet(0)),
            op!("GLOBAL_SET", Instruction::GlobalSet(0)),
            op!("CALL", Instruction::Call(0)),
            op!(
                "CALL_INDIRECT",
                Instruction::CallIndirect {
                    type_index: 0,
                    table_index: 0,
                },
            ),
            op!("RETURN_CALL", Instruction::ReturnCall(0)),
            op!("RETURN", Instruction::Return),
            op!("UNREACHABLE", Instruction::Unreachable),
            op!("I32_ADD", Instruction::I32Add),
            op!("I32_SUB", Instruction::I32Sub),
            op!("I32_MUL", Instruction::I32Mul),
            op!("I32_DIV_S", Instruction::I32DivS),
            op!("I32_DIV_U", Instruction::I32DivU),
            op!("I32_REM_S", Instruction::I32RemS),
            op!("I32_REM_U", Instruction::I32RemU),
            op!("I32_AND", Instruction::I32And),
            op!("I32_OR", Instruction::I32Or),
            op!("I32_XOR", Instruction::I32Xor),
            op!("I32_SHL", Instruction::I32Shl),
            op!("I32_SHR_S", Instruction::I32ShrS),
            op!("I32_SHR_U", Instruction::I32ShrU),
            op!("I32_EQ", Instruction::I32Eq),
            op!("I32_EQZ", Instruction::I32Eqz),
            op!("I32_NE", Instruction::I32Ne),
            op!("I32_LT_S", Instruction::I32LtS),
            op!("I32_LT_U", Instruction::I32LtU),
            op!("I32_GT_S", Instruction::I32GtS),
            op!("I32_GT_U", Instruction::I32GtU),
            op!("I32_LE_S", Instruction::I32LeS),
            op!("I32_LE_U", Instruction::I32LeU),
            op!("I32_GE_S", Instruction::I32GeS),
            op!("I32_GE_U", Instruction::I32GeU),
            op!("I64_ADD", Instruction::I64Add),
            op!("I64_SUB", Instruction::I64Sub),
            op!("I64_MUL", Instruction::I64Mul),
            op!("I64_DIV_S", Instruction::I64DivS),
            op!("I64_DIV_U", Instruction::I64DivU),
            op!("I64_REM_S", Instruction::I64RemS),
            op!("I64_REM_U", Instruction::I64RemU),
            op!("I64_AND", Instruction::I64And),
            op!("I64_OR", Instruction::I64Or),
            op!("I64_XOR", Instruction::I64Xor),
            op!("I64_SHL", Instruction::I64Shl),
            op!("I64_SHR_S", Instruction::I64ShrS),
            op!("I64_SHR_U", Instruction::I64ShrU),
            op!("I64_EQ", Instruction::I64Eq),
            op!("I64_EQZ", Instruction::I64Eqz),
            op!("I64_NE", Instruction::I64Ne),
            op!("I64_LT_S", Instruction::I64LtS),
            op!("I64_LT_U", Instruction::I64LtU),
            op!("I64_GT_S", Instruction::I64GtS),
            op!("I64_GT_U", Instruction::I64GtU),
            op!("I64_LE_S", Instruction::I64LeS),
            op!("I64_LE_U", Instruction::I64LeU),
            op!("I64_GE_S", Instruction::I64GeS),
            op!("I64_GE_U", Instruction::I64GeU),
            op!("I32_WRAP_I64", Instruction::I32WrapI64),
            op!("I64_EXTEND_I32_S", Instruction::I64ExtendI32S),
            op!("I64_EXTEND_I32_U", Instruction::I64ExtendI32U),
            // Memory stores — the `list<u8>`/`string` return path writes the payload bytes and the
            // canonical-ABI return area (`[data-ptr, data-len]`) into linear memory. `MemArg` is
            // irrelevant to the opcode byte (the extraction reads byte 0; the serializer emits the
            // align/offset LEB operands itself), so a zero memarg suffices to recover the opcode.
            op!(
                "I32_STORE",
                Instruction::I32Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            op!(
                "I32_STORE8",
                Instruction::I32Store8(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            // The wider stores the reducer RESULT-lower (W4c-b canonical writer) needs: an `i64`/`f64`/`f32`
            // scalar field, and the 2-byte `i32.store16` for a `u16`/`s16` slot. Same MemArg-irrelevant note.
            op!(
                "I64_STORE",
                Instruction::I64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            op!(
                "F32_STORE",
                Instruction::F32Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            op!(
                "F64_STORE",
                Instruction::F64Store(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            op!(
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
            op!(
                "I32_LOAD",
                Instruction::I32Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            op!(
                "I32_LOAD8_U",
                Instruction::I32Load8U(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            // Width-specific loads for reading a scalar leaf a spilled compound result stored at its natural
            // width (the general result-lift's scalar-leaf boxing). MemArg is opcode-byte-irrelevant here too.
            op!(
                "I64_LOAD",
                Instruction::I64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            op!(
                "F32_LOAD",
                Instruction::F32Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            op!(
                "F64_LOAD",
                Instruction::F64Load(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            op!(
                "I32_LOAD8_S",
                Instruction::I32Load8S(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            op!(
                "I32_LOAD16_S",
                Instruction::I32Load16S(wasm_encoder::MemArg {
                    offset: 0,
                    align: 0,
                    memory_index: 0,
                }),
            ),
            op!(
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
            valtype!(
                "CORE_I32",
                "core valtype `i32` — a ≤32-bit scalar's machine slot.",
                ValType::I32,
            ),
            valtype!(
                "CORE_I64",
                "core valtype `i64` — a 64-bit scalar's machine slot.",
                ValType::I64,
            ),
            valtype!("CORE_F32", "core valtype `f32`.", ValType::F32),
            valtype!("CORE_F64", "core valtype `f64`.", ValType::F64),
            Single {
                ident: "BLOCK_EMPTY",
                byte: one_byte("empty block type", wasm_encoder::BlockType::Empty),
                doc: "empty block type — a structured block (`if`/`block`) that leaves no value.",
                oracle_src: stringify!(byte0(wasm_encoder::BlockType::Empty)),
            },
            prim!("COMP_BOOL", "component `bool`.", PrimitiveValType::Bool),
            prim!("COMP_S8", "component `s8`.", PrimitiveValType::S8),
            prim!("COMP_U8", "component `u8`.", PrimitiveValType::U8),
            prim!("COMP_S16", "component `s16`.", PrimitiveValType::S16),
            prim!("COMP_U16", "component `u16`.", PrimitiveValType::U16),
            prim!("COMP_S32", "component `s32`.", PrimitiveValType::S32),
            prim!("COMP_U32", "component `u32`.", PrimitiveValType::U32),
            prim!("COMP_S64", "component `s64`.", PrimitiveValType::S64),
            prim!("COMP_U64", "component `u64`.", PrimitiveValType::U64),
            prim!("COMP_F32", "component `f32`.", PrimitiveValType::F32),
            prim!("COMP_F64", "component `f64`.", PrimitiveValType::F64),
            prim!(
                "COMP_STRING",
                "component `string`.",
                PrimitiveValType::String,
            ),
            core_sec!("CORE_SEC_TYPE", "core TYPE section id.", SectionId::Type),
            core_sec!(
                "CORE_SEC_FUNCTION",
                "core FUNCTION section id.",
                SectionId::Function,
            ),
            core_sec!(
                "CORE_SEC_TABLE",
                "core TABLE section id (the funcref table a closure's `call_indirect` dispatches through — one entry per lambda-lifted closure function).",
                SectionId::Table,
            ),
            core_sec!(
                "CORE_SEC_ELEMENT",
                "core ELEMENT section id (the active segment filling the funcref table with the lifted closure functions' indices, so a closure's stored table slot names its code).",
                SectionId::Element,
            ),
            core_sec!(
                "CORE_SEC_MEMORY",
                "core MEMORY section id (the linear memory a `list<u8>`/`string` return lifts through).",
                SectionId::Memory,
            ),
            core_sec!(
                "CORE_SEC_GLOBAL",
                "core GLOBAL section id (a build-once static compound's handle global; the bump-allocator cursor).",
                SectionId::Global,
            ),
            core_sec!(
                "CORE_SEC_EXPORT",
                "core EXPORT section id.",
                SectionId::Export,
            ),
            core_sec!(
                "CORE_SEC_START",
                "core START section id (the init function that builds each static compound once).",
                SectionId::Start,
            ),
            core_sec!("CORE_SEC_CODE", "core CODE section id.", SectionId::Code),
            core_sec!(
                "CORE_SEC_DATA",
                "core DATA section id (an active segment initializing linear memory — the constant value form + return area of a resource escape).",
                SectionId::Data,
            ),
            comp_sec!(
                "COMP_SEC_CORE_MODULE",
                "component CORE-MODULE section id.",
                ComponentSectionId::CoreModule,
            ),
            comp_sec!(
                "COMP_SEC_CORE_INSTANCE",
                "component CORE-INSTANCE section id.",
                ComponentSectionId::CoreInstance,
            ),
            comp_sec!(
                "COMP_SEC_ALIAS",
                "component ALIAS section id.",
                ComponentSectionId::Alias,
            ),
            comp_sec!(
                "COMP_SEC_TYPE",
                "component TYPE section id.",
                ComponentSectionId::Type,
            ),
            comp_sec!(
                "COMP_SEC_CANONICAL",
                "component CANONICAL-FUNCTION section id.",
                ComponentSectionId::CanonicalFunction,
            ),
            comp_sec!(
                "COMP_SEC_IMPORT",
                "component IMPORT section id.",
                ComponentSectionId::Import,
            ),
            comp_sec!(
                "COMP_SEC_EXPORT",
                "component EXPORT section id.",
                ComponentSectionId::Export,
            ),
            comp_sec!(
                "COMP_SEC_COMPONENT",
                "component COMPONENT section id (a nested component definition).",
                ComponentSectionId::Component,
            ),
            comp_sec!(
                "COMP_SEC_INSTANCE",
                "component INSTANCE section id (instantiate a component).",
                ComponentSectionId::Instance,
            ),
            Single {
                ident: "EXPORT_KIND_FUNC",
                byte: one_byte("export kind func", ExportKind::Func),
                doc: "core export kind `func` — the export-descriptor byte for a function export.",
                oracle_src: stringify!(byte0(wasm_encoder::ExportKind::Func)),
            },
            Single {
                ident: "EXPORT_KIND_MEMORY",
                byte: one_byte("export kind memory", ExportKind::Memory),
                doc: "core export kind `memory` — the export-descriptor byte for a memory export (the resource escape's `memory`).",
                oracle_src: stringify!(byte0(wasm_encoder::ExportKind::Memory)),
            },
            // The two functype FORM tags are read off a one-entry type section (see `core_functype_form`),
            // not a plain `.encode()` byte, so they carry no uniform `oracle_src` — the emitted oracle
            // module asserts them with a bespoke helper; the nix `wasm-abi-oracle` check also covers them.
            Single {
                ident: "CORE_FUNCTYPE_FORM",
                byte: core_functype_form(),
                doc: "core functype form tag — opens a core `func` type before its param/result vecs.",
                oracle_src: "",
            },
            Single {
                ident: "COMP_FUNCTYPE_FORM",
                byte: component_functype_form(),
                doc: "component functype form tag — opens a component function type.",
                oracle_src: "",
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

    /// Read the tables back from the AUTHORITATIVE wasm-abi.sexp's decoded cadenza-ast (the producer path,
    /// operator's model). Walks the root `(do …)`: each child is `(opcode NAME byte)` /
    /// `(single NAME byte "doc")` / `(magic NAME b0 … b7 "doc")` — head via `head_name`, NAME via `as_name`,
    /// bytes via `as_int_usize`, doc via `as_str`. Panics on a malformed entry (the sexpr is the committed
    /// source of truth + the `--oracle-check` guards its bytes, so a shape break is a hard bug). Strings are
    /// interned to `'static` (the bin runs once + exits) so the `Tables` shape matches `collect`'s.
    pub fn read_sexpr_tables(a: &cadenza_ast::ast::Arenas) -> Tables {
        use cadenza_ast::ast::Struct;
        fn intern(s: &str) -> &'static str {
            Box::leak(s.to_owned().into_boxed_str())
        }
        let name = |a: &cadenza_ast::ast::Arenas, id| {
            intern(a.as_name(id).expect("wasm-abi entry: expected a NAME"))
        };
        let byte = |a: &cadenza_ast::ast::Arenas, id| {
            u8::try_from(
                a.as_int_usize(id)
                    .expect("wasm-abi entry: expected an integer byte"),
            )
            .expect("wasm-abi byte does not fit in u8")
        };
        let doc = |a: &cadenza_ast::ast::Arenas, id| {
            intern(a.as_str(id).expect("wasm-abi entry: expected a doc string"))
        };

        let Struct::List(items) = a.get(a.root) else {
            panic!("wasm-abi.sexp root is not a `(do …)` list");
        };
        let (mut opcodes, mut singles, mut magics) = (Vec::new(), Vec::new(), Vec::new());
        // Skip the `do` head; each remaining child is one table entry.
        for &child in items.iter().skip(1) {
            let head = a.head_name(child).expect("wasm-abi entry has no head name");
            let Struct::List(f) = a.get(child) else {
                panic!("wasm-abi entry `{head}` is not a list");
            };
            match head {
                // A sexpr-read table carries no wasm-encoder constructor → `oracle_src` is empty (it is the
                // CONSTANTS source, not the oracle-emission source; the oracle module emits from `collect`).
                "opcode" => opcodes.push(Opcode {
                    ident: name(a, f[1]),
                    byte: byte(a, f[2]),
                    oracle_src: "",
                }),
                "single" => singles.push(Single {
                    ident: name(a, f[1]),
                    byte: byte(a, f[2]),
                    doc: doc(a, f[3]),
                    oracle_src: "",
                }),
                "magic" => {
                    let mut bytes = [0u8; 8];
                    for (i, b) in bytes.iter_mut().enumerate() {
                        *b = byte(a, f[2 + i]);
                    }
                    magics.push(Magic {
                        ident: name(a, f[1]),
                        bytes,
                        doc: doc(a, f[10]),
                    });
                }
                other => {
                    panic!("unknown wasm-abi entry head `{other}` (expected opcode/single/magic)")
                }
            }
        }
        Tables {
            opcodes,
            singles,
            magics,
        }
    }

    /// Cross-check the sexpr's BYTES against the wasm-encoder ORACLE (the operator's inverted guarantee):
    /// every named opcode/single byte + magic byte-seq in `sexpr` must equal what `wasm-encoder` emits in
    /// `oracle`. Returns human-readable mismatch lines (empty = the sexpr matches the spec encoder). Docs are
    /// NOT checked — they are human-authored prose the encoder has no opinion on. Catches a transcription typo
    /// in the committed sexpr at build time.
    pub fn oracle_mismatches(oracle: &Tables, sexpr: &Tables) -> Vec<String> {
        use std::collections::BTreeMap;
        let mut out = Vec::new();
        let o_ops: BTreeMap<&str, u8> = oracle.opcodes.iter().map(|o| (o.ident, o.byte)).collect();
        let s_ops: BTreeMap<&str, u8> = sexpr.opcodes.iter().map(|o| (o.ident, o.byte)).collect();
        let o_singles: BTreeMap<&str, u8> =
            oracle.singles.iter().map(|s| (s.ident, s.byte)).collect();
        let s_singles: BTreeMap<&str, u8> =
            sexpr.singles.iter().map(|s| (s.ident, s.byte)).collect();
        let o_magics: BTreeMap<&str, [u8; 8]> =
            oracle.magics.iter().map(|m| (m.ident, m.bytes)).collect();
        let s_magics: BTreeMap<&str, [u8; 8]> =
            sexpr.magics.iter().map(|m| (m.ident, m.bytes)).collect();
        let cmp_u8 =
            |kind: &str, o: &BTreeMap<&str, u8>, s: &BTreeMap<&str, u8>, out: &mut Vec<String>| {
                for (name, ob) in o {
                    match s.get(name) {
                        Some(sb) if sb == ob => {}
                        Some(sb) => out.push(format!(
                            "{kind} {name}: sexpr 0x{sb:02x} != wasm-encoder 0x{ob:02x}"
                        )),
                        None => out.push(format!(
                            "{kind} {name}: MISSING from sexpr (wasm-encoder 0x{ob:02x})"
                        )),
                    }
                }
                for name in s.keys() {
                    if !o.contains_key(name) {
                        out.push(format!(
                            "{kind} {name}: in sexpr but NOT in the wasm-encoder oracle"
                        ));
                    }
                }
            };
        cmp_u8("opcode", &o_ops, &s_ops, &mut out);
        cmp_u8("single", &o_singles, &s_singles, &mut out);
        for (name, ob) in &o_magics {
            match s_magics.get(name) {
                Some(sb) if sb == ob => {}
                Some(sb) => out.push(format!("magic {name}: sexpr {sb:?} != wasm-encoder {ob:?}")),
                None => out.push(format!("magic {name}: MISSING from sexpr")),
            }
        }
        for name in s_magics.keys() {
            if !o_magics.contains_key(name) {
                out.push(format!(
                    "magic {name}: in sexpr but NOT in the wasm-encoder oracle"
                ));
            }
        }
        out
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

    /// Render the DERIVED CRATE's baked-in `#[cfg(test)] mod oracle` — the operator's inverted guarantee
    /// as UNIT TESTS INSIDE the generated crate: each generated constant is asserted against the byte
    /// `wasm-encoder` (the spec oracle) produces for its named constructor. `wasm-encoder` is referenced
    /// only here, under `#[cfg(test)]`, so the derived crate carries it as a DEV-dependency only and its
    /// consumers (rcdzc) stay `wasm-encoder`-free. Emitted as source text (the per-entry `oracle_src`
    /// re-derivation expression is a token string). Entries with an empty `oracle_src` (the two functype
    /// form tags, read off a type section rather than a plain `.encode()`) are covered by the nix
    /// `wasm-abi-oracle` check instead of a per-entry assert here.
    pub fn render_oracle_tests(t: &Tables) -> String {
        let mut s = String::new();
        s.push_str(
            "\n#[cfg(test)]\nmod oracle {\n    //! @generated — the operator's inverted guarantee: every \
             constant above matches the `wasm-encoder` spec\n    //! oracle. A hand-edit to `wasm-abi.sexp` \
             that transcribes a wrong byte fails these tests. `wasm-encoder`\n    //! is a DEV-dependency of \
             this crate ONLY (its consumers do not carry it).\n    #![allow(unused_imports)]\n    use \
             wasm_encoder::{\n        BlockType, Component, ComponentSectionId, Encode, ExportKind, \
             Instruction, Module, PrimitiveValType,\n        SectionId, ValType,\n    };\n\n    /// The first \
             byte `wasm-encoder` emits for a value — the opcode / tag byte the backend constant should equal.\n    \
             fn byte0<E: Encode>(v: E) -> u8 {\n        let mut b = Vec::new();\n        v.encode(&mut b);\n        \
             assert!(!b.is_empty(), \"wasm-encoder emitted no bytes\");\n        b[0]\n    }\n",
        );
        // Opcodes: `super::op::NAME == byte0(Instruction::…)`.
        s.push_str("\n    #[test]\n    fn opcodes_match_wasm_encoder() {\n");
        for o in &t.opcodes {
            if o.oracle_src.is_empty() {
                continue;
            }
            s.push_str(&format!(
                "        assert_eq!(super::op::{}, {}, \"{}\");\n",
                o.ident, o.oracle_src, o.ident
            ));
        }
        s.push_str("    }\n");
        // Singles: `super::NAME == byte0(ValType::…)` or `== u8::from(SectionId::…)`.
        s.push_str("\n    #[test]\n    fn singles_match_wasm_encoder() {\n");
        for si in &t.singles {
            if si.oracle_src.is_empty() {
                continue;
            }
            s.push_str(&format!(
                "        assert_eq!(super::{}, {}, \"{}\");\n",
                si.ident, si.oracle_src, si.ident
            ));
        }
        s.push_str("    }\n");
        // Magics: the 8-byte core / component preamble headers.
        s.push_str("\n    #[test]\n    fn magics_match_wasm_encoder() {\n");
        for m in &t.magics {
            let header = match m.ident {
                "CORE_MAGIC" => "Module::HEADER",
                "COMPONENT_MAGIC" => "Component::HEADER",
                other => panic!(
                    "unknown magic `{other}` — add its wasm-encoder header to render_oracle_tests"
                ),
            };
            s.push_str(&format!(
                "        assert_eq!(super::{}, &{}, \"{}\");\n",
                m.ident, header, m.ident
            ));
        }
        s.push_str("    }\n}\n");
        s
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

#[cfg(test)]
mod tests {
    use crate::wasm_abi::{self, Magic, Opcode, Single, Tables};

    /// Clone the tables by hand (the entry structs are deliberately non-`Clone` value types) so a test
    /// can mutate a copy while leaving the oracle intact — the stand-in for "someone hand-edits the sexpr".
    fn rebuild(t: &Tables) -> Tables {
        Tables {
            opcodes: t
                .opcodes
                .iter()
                .map(|o| Opcode {
                    ident: o.ident,
                    byte: o.byte,
                    oracle_src: o.oracle_src,
                })
                .collect(),
            singles: t
                .singles
                .iter()
                .map(|s| Single {
                    ident: s.ident,
                    byte: s.byte,
                    doc: s.doc,
                    oracle_src: s.oracle_src,
                })
                .collect(),
            magics: t
                .magics
                .iter()
                .map(|m| Magic {
                    ident: m.ident,
                    bytes: m.bytes,
                    doc: m.doc,
                })
                .collect(),
        }
    }

    /// The wasm-encoder oracle table must agree with itself — non-empty in each category, and
    /// `oracle_mismatches` finds zero disagreements. Guards the extraction (a byte read back that
    /// doesn't round-trip, or an empty category, is a broken oracle).
    #[test]
    fn oracle_table_is_self_consistent() {
        let t = wasm_abi::collect();
        assert!(
            !t.opcodes.is_empty() && !t.singles.is_empty() && !t.magics.is_empty(),
            "an oracle category is empty: {} opcodes, {} singles, {} magics",
            t.opcodes.len(),
            t.singles.len(),
            t.magics.len(),
        );
        let mismatches = wasm_abi::oracle_mismatches(&t, &wasm_abi::collect());
        assert!(
            mismatches.is_empty(),
            "the oracle table disagrees with itself: {mismatches:?}"
        );
    }

    /// A fat-fingered byte in the sexpr (the exact transcription typo the operator's inverted guarantee
    /// exists to catch) must be reported by `--oracle-check`. This is the invariant that makes the sexpr
    /// safe as the authoritative source: a wrong `0x41` can't ship silently.
    #[test]
    fn oracle_check_catches_a_transcription_typo() {
        let oracle = wasm_abi::collect();
        let mut sexpr = rebuild(&oracle);
        let ident = sexpr.opcodes[0].ident;
        sexpr.opcodes[0].byte = sexpr.opcodes[0].byte.wrapping_add(1);
        let mismatches = wasm_abi::oracle_mismatches(&oracle, &sexpr);
        assert!(
            mismatches
                .iter()
                .any(|m| m.contains(ident) && m.contains("!=")),
            "expected a byte-mismatch report for {ident}, got {mismatches:?}"
        );
    }

    /// A structural drift — an entry dropped from or invented in the sexpr — must also be reported, not
    /// just a wrong byte on a present entry. Pins the MISSING / not-in-oracle arms of the cross-check.
    #[test]
    fn oracle_check_catches_missing_and_extra_entries() {
        let oracle = wasm_abi::collect();

        let mut dropped = rebuild(&oracle);
        let dropped_ident = dropped.opcodes[0].ident;
        dropped.opcodes.remove(0);
        let m = wasm_abi::oracle_mismatches(&oracle, &dropped);
        assert!(
            m.iter()
                .any(|s| s.contains(dropped_ident) && s.contains("MISSING")),
            "expected a MISSING report for dropped {dropped_ident}, got {m:?}"
        );

        let mut extra = rebuild(&oracle);
        extra.opcodes.push(Opcode {
            ident: "NOT_A_REAL_OPCODE",
            byte: 0xff,
            oracle_src: "",
        });
        let m = wasm_abi::oracle_mismatches(&oracle, &extra);
        assert!(
            m.iter()
                .any(|s| s.contains("NOT_A_REAL_OPCODE")
                    && s.contains("NOT in the wasm-encoder oracle")),
            "expected an extra-entry report, got {m:?}"
        );
    }

    /// Pin two spec anchors end-to-end: the extracted bytes for `i32.const`/`i32.add` (0x41/0x6a — the
    /// file's own banner example) and that `render` emits them into a hex `op` module. A render change
    /// that stopped emitting hex, dropped the module, or mis-mapped an opcode flips this.
    #[test]
    fn render_pins_known_spec_bytes() {
        let t = wasm_abi::collect();
        let op_byte = |ident: &str| t.opcodes.iter().find(|o| o.ident == ident).map(|o| o.byte);
        assert_eq!(op_byte("I32_CONST"), Some(0x41), "i32.const opcode drifted");
        assert_eq!(op_byte("I32_ADD"), Some(0x6a), "i32.add opcode drifted");

        let rust = wasm_abi::render(&t).to_string();
        assert!(
            rust.contains("pub mod op"),
            "render dropped the op module: {rust}"
        );
        assert!(
            rust.contains("0x41"),
            "render is not emitting hex opcode bytes"
        );
    }

    /// `render_oracle_tests` emits a self-contained `#[cfg(test)] mod oracle` that asserts each generated
    /// constant against its `wasm-encoder` re-derivation (the operator's inverted guarantee, baked into the
    /// derived crate). Pin the module shape + the `byte0`/`u8::from`/magic assert forms for known entries.
    #[test]
    fn render_oracle_tests_emits_encoder_asserts() {
        let src = wasm_abi::render_oracle_tests(&wasm_abi::collect());
        assert!(src.contains("mod oracle"), "no oracle module: {src}");
        assert!(src.contains("fn byte0"), "no byte0 helper");
        // Flatten whitespace so the checks are robust to `stringify!`'s `::`/paren spacing (rustfmt
        // normalizes it in the emitted file, but this is the raw pre-format output).
        let flat: String = src.chars().filter(|c| !c.is_whitespace()).collect();
        // opcode: encode-byte0 form; a valtype single: encode-byte0; a section: u8::from; a magic: HEADER.
        assert!(
            flat.contains("assert_eq!(super::op::I32_CONST,byte0(Instruction::I32Const(0)),"),
            "{src}"
        );
        assert!(
            flat.contains("assert_eq!(super::CORE_I32,byte0(ValType::I32),"),
            "{src}"
        );
        assert!(
            flat.contains("assert_eq!(super::CORE_SEC_TYPE,u8::from(SectionId::Type),"),
            "{src}"
        );
        assert!(
            flat.contains("assert_eq!(super::CORE_MAGIC,&Module::HEADER,"),
            "{src}"
        );
        // the two functype-form tags carry no uniform oracle_src → not per-entry asserted here.
        assert!(
            !src.contains("CORE_FUNCTYPE_FORM"),
            "functype form should be skipped, not emitted"
        );
    }

    /// The `--crate-lib` banner opens the DERIVED crate's `lib.rs` as a crate-root doc comment and names
    /// its authored source + generated status (so a hand-edit is discouraged + the provenance is legible).
    #[test]
    fn crate_lib_banner_is_a_generated_crate_doc() {
        let banner = crate::crate_lib_banner();
        assert!(
            banner.starts_with("//!"),
            "crate-lib banner is not a `//!` crate doc: {banner}"
        );
        assert!(
            banner.contains("@generated"),
            "banner omits the generated marker"
        );
        assert!(
            banner.contains("data/wasm-abi.sexp"),
            "banner omits the authored source path"
        );
        assert!(
            banner.contains("DO NOT hand-edit"),
            "banner omits the do-not-edit notice"
        );
    }
}
