//! Hand-rolled DWARF for the embedded core module — D2 (stepping) + D3 (scalar vars) of
//! `DESIGN-debug-info-rcdzc.md`.
//!
//! Emits the four `.debug_*` custom sections a debugger needs to STEP through Cadenza source and
//! inspect scalar locals:
//! - `.debug_str` — the string table the DIEs reference by offset (names, producer, dir).
//! - `.debug_abbrev` — the abbreviation table (the DIE "shapes": compile_unit, subprogram, formal
//!   parameter, base type, variable, lexical block).
//! - `.debug_info` — a `DW_TAG_compile_unit` DIE whose children are one `DW_TAG_base_type` per distinct
//!   scalar type (D3) then one `DW_TAG_subprogram` per function (its code-offset range
//!   `DW_AT_low_pc`/`DW_AT_high_pc`, plus, per scalar local: a `DW_TAG_formal_parameter` for a param and
//!   a `DW_TAG_variable` for a `let`-binding — each with a `DW_OP_WASM_location` local slot + a
//!   `DW_AT_type` ref, so a debugger can `print` it; a scalar match binder becomes a `DW_TAG_variable`
//!   inside a PC-ranged `DW_TAG_lexical_block` so it is in scope only within its match).
//! - `.debug_line` — the line-number program mapping each function's per-construct code offsets →
//!   (file, line, column).
//!
//! These are CUSTOM sections (wasm section id 0) appended to the embedded CORE MODULE — the standard
//! place wasm tools/debuggers look ("DWARF for WebAssembly"). Code addresses are byte offsets into the
//! core module's CODE section (`code_offset_base + FuncCodeRange.code_start`, computed by the caller).
//! A custom section is inert data the runtime never executes and no instruction can address, and it is
//! not an import — so the running component can neither read its own DWARF nor gain a manifest entry
//! from carrying it:
//!
//= spec/capabilities/debug-information.md#a-running-component-cannot-observe-its-own-debug-information
//# A component MUST NOT be able to read its own debug information while it runs, so that debug information is metadata for an external tool rather than runtime type reflection the erasure guarantee removes (type-system.md §Types Are Erased From The Component).
//!
//= spec/capabilities/debug-information.md#a-running-component-cannot-observe-its-own-debug-information
//# Debug information MUST NOT add a host operation to a component's manifest, so that a component carrying debug information imports exactly the operations the same component without it imports and its capability manifest is unchanged.
//!
//! **The byte discipline** matches the rest of the backend: everything is hand-emitted LEB128 / fixed
//! little-endian, no encoder in the compile path. The oracle is `llvm-dwarfdump` / `wasm-tools` parsing
//! the result (the `wasm-encoder` byte-oracle does not model DWARF), plus real debugger consumption.
//!
//! **Reproducibility** (design §4): `DW_AT_comp_dir` is a fixed sentinel (never the build dir),
//! `DW_AT_producer` is a fixed string (never the live toolchain banner), `DW_AT_name` is the
//! tree-relative module path from the `spans` sidecar. No wall-clock. Everything is emitted in
//! source-determined (emission) order, so two derivations byte-match.
//!
//! Scope: PER-POSITION line rows — one row per distinct `(line, column)` the function's code visits
//! (from the `stmt_lines` markers; `DwarfFunc.rows`), so a debugger steps construct-by-construct and
//! highlights the exact sub-expression on a line (the payoff for s-expression Cadenza). A body with no
//! sub-construct markers falls back to a single row at the function entry (function granularity). This
//! is DWARF v4 (address size 4 — a wasm32 code offset is 32-bit), the version GDB/LLDB/`wasmtime -D
//! debug-info` and the Chrome DWARF extension all accept.
//!
//= spec/capabilities/debug-information.md#debug-information-uses-an-interchange-format
//# Debug information MUST be emitted in an interchange debug-information format that an external debugging tool consumes, rather than a form only Cadenza's own tooling can read, so that an existing debugger relates the artifact to its source without bespoke Cadenza support.

use crate::backend::wasm::encode::{section, sleb128, uleb128};

/// A fixed, non-provenance-leaking producer string (design §4 — never the live toolchain banner).
//= spec/capabilities/debug-information.md#debug-information-carries-no-provenance
//# The compiler MUST NOT embed into debug information a producer or build-environment string that would otherwise vary between builds of the same source.
//= spec/contracts/reproducible-derivation.md#provenance-is-stripped-or-normalized
//# The compiler MUST remove or normalize any embedded producer string, build path, or timestamp that would otherwise vary between builds of the same source.
const PRODUCER: &str = "cadenza-rcdzc";
/// A fixed sentinel compilation directory — never the build directory (design §4). This is the debug
/// section's face of the constitution's reproducibility floor: the compiler embeds no build-host path
/// (nor wall-clock, nor host id) anywhere in its output, so the same source derives byte-identically.
//= spec/capabilities/debug-information.md#debug-information-carries-no-provenance
//# The compiler MUST NOT embed into debug information a wall-clock time, an absolute filesystem path, or a build-host identifier.
//= constitution.md#ii-compilation-is-reproducible
//# The compiler MUST NOT embed a wall-clock time, an absolute filesystem path, or a build-host identifier into its output.
//= spec/contracts/reproducible-derivation.md#provenance-is-stripped-or-normalized
//# The compiler MUST NOT embed into its output any value derived from the build host's environment that is not a function of the source and the pinned toolchain.
const COMP_DIR: &str = "/";

// ── DWARF constants (from the DWARF 4 spec; hand-transcribed — the values are stable and standardized,
//    unlike the wasm opcodes which come from the generated table). ──────────────────────────────────
mod dw {
    // Tags.
    pub const TAG_COMPILE_UNIT: u64 = 0x11;
    pub const TAG_SUBPROGRAM: u64 = 0x2e;
    pub const TAG_FORMAL_PARAMETER: u64 = 0x05;
    pub const TAG_VARIABLE: u64 = 0x34;
    pub const TAG_BASE_TYPE: u64 = 0x24;
    pub const TAG_LEXICAL_BLOCK: u64 = 0x0b;
    // Attributes.
    pub const AT_NAME: u64 = 0x03;
    pub const AT_LOW_PC: u64 = 0x11;
    pub const AT_HIGH_PC: u64 = 0x12;
    pub const AT_STMT_LIST: u64 = 0x10;
    pub const AT_COMP_DIR: u64 = 0x1b;
    pub const AT_PRODUCER: u64 = 0x25;
    pub const AT_LANGUAGE: u64 = 0x13;
    pub const AT_DECL_FILE: u64 = 0x3a;
    pub const AT_DECL_LINE: u64 = 0x3b;
    pub const AT_LOCATION: u64 = 0x02;
    pub const AT_TYPE: u64 = 0x49;
    pub const AT_BYTE_SIZE: u64 = 0x0b;
    pub const AT_ENCODING: u64 = 0x3e;
    // Forms.
    pub const FORM_ADDR: u64 = 0x01;
    pub const FORM_DATA4: u64 = 0x06;
    pub const FORM_DATA2: u64 = 0x05;
    pub const FORM_DATA1: u64 = 0x0b;
    pub const FORM_STRP: u64 = 0x0e; // offset into .debug_str
    pub const FORM_SEC_OFFSET: u64 = 0x17;
    pub const FORM_UDATA: u64 = 0x0f;
    pub const FORM_REF4: u64 = 0x13; // 4-byte offset into .debug_info (a DIE reference)
    pub const FORM_EXPRLOC: u64 = 0x18; // a uleb-length-prefixed DWARF expression
    // Source language (`DW_AT_language`). Cadenza has no assigned DWARF language code, so we declare
    // `DW_LANG_C` (0x0002) — the conventional "generic compiled scalar language" a debugger recognizes:
    // it selects a C-like expression evaluator + integer/bool value formatting, which matches Cadenza's
    // scalar surface (the only values DWARF describes; compounds are opaque handles, §3). A CU with NO
    // language reads as "<not loaded>" in lldb, which then declines to format values.
    pub const LANG_C: u16 = 0x0002;
    // Base-type encodings (`DW_ATE_*`).
    pub const ATE_BOOLEAN: u8 = 0x02;
    pub const ATE_FLOAT: u8 = 0x04; // IEEE floating point (`f32`/`f64`)
    pub const ATE_SIGNED: u8 = 0x05;
    pub const ATE_UNSIGNED: u8 = 0x07;
    // A location expression: a value in a wasm LOCAL. `DW_OP_WASM_location 0x00 <local-idx-uleb>`
    // (the WebAssembly-DWARF vendor extension — the way a debugger finds a wasm local's slot).
    pub const OP_WASM_LOCATION: u8 = 0xed;
    // Children flag.
    pub const CHILDREN_YES: u8 = 0x01;
    pub const CHILDREN_NO: u8 = 0x00;
    // Line-number standard/extended opcodes.
    pub const LNS_COPY: u8 = 0x01;
    pub const LNS_ADVANCE_LINE: u8 = 0x03;
    pub const LNS_SET_FILE: u8 = 0x04;
    pub const LNS_SET_COLUMN: u8 = 0x05;
    pub const LNE_END_SEQUENCE: u8 = 0x01;
    pub const LNE_SET_ADDRESS: u8 = 0x02;
    // Our abbreviation codes (arbitrary, must match between .debug_abbrev and .debug_info).
    pub const ABBREV_COMPILE_UNIT: u64 = 1;
    pub const ABBREV_SUBPROGRAM: u64 = 2; // subprogram WITHOUT children (no scalar locals)
    pub const ABBREV_SUBPROGRAM_KIDS: u64 = 3; // subprogram WITH children (has scalar locals)
    pub const ABBREV_FORMAL_PARAMETER: u64 = 4;
    pub const ABBREV_BASE_TYPE: u64 = 5;
    pub const ABBREV_VARIABLE: u64 = 6; // a `let`-binding local (a DW_TAG_variable, not a parameter)
    pub const ABBREV_LEXICAL_BLOCK: u64 = 7; // a match-binder scope (has children: its variables)
}

/// A string table (`.debug_str`) built incrementally: `intern(s)` returns the byte offset of `s`,
/// appending it (NUL-terminated) on first use. Offsets are what `DW_FORM_strp` attributes reference.
#[derive(Default)]
struct StrTab {
    bytes: Vec<u8>,
}

impl StrTab {
    fn intern(&mut self, s: &str) -> u32 {
        let off = self.bytes.len() as u32;
        self.bytes.extend_from_slice(s.as_bytes());
        self.bytes.push(0);
        off
    }
}

/// A resolved function to describe: its name, code-offset range (ABSOLUTE in the core module's code
/// section), source line (1-based; 0 = unknown), and its named scalar locals (D3 — a `DW_TAG_variable`
/// each, so a debugger reads the value). Built by the caller from `FuncCodeRange` + the `spans`
/// side-table + a newline index over the source (or 1 when only byte spans are known).
pub struct DwarfFunc {
    pub name: String,
    pub low_pc: u32,
    pub high_pc: u32,
    pub line: u32,
    pub vars: Vec<DwarfVar>,
    /// Per-construct `(absolute code offset, 1-based source line, 1-based source column)` rows,
    /// ascending by offset — one per source POSITION the function's code visits
    /// (`DESIGN-debug-line-granularity-rcdzc.md`). The line program emits a row at each, so a debugger
    /// steps position-by-position; the column lets a debugger highlight the exact sub-expression on a
    /// line (the payoff for s-expression Cadenza, where several constructs share a line). Empty → one
    /// row at `low_pc`/`line`/column 0 (the function-granularity fallback for a single-construct body).
    pub rows: Vec<(u32, u32, u32)>,
    /// Scalar MATCH-BINDER lexical scopes (D3 locals for `(match e (x body)…)`). Each becomes a
    /// `DW_TAG_lexical_block` child of the subprogram, with a `DW_AT_low_pc`/`high_pc` PC range and a
    /// `DW_TAG_variable` per binder — so a match binder is `print`-able ONLY inside its match (its slot
    /// is reused elsewhere, so a function-scoped variable would misreport it). Empty for a function with
    /// no scalar match binder (the common case) → the subprogram DIE is unchanged.
    pub scopes: Vec<DwarfScope>,
}

/// A lexical-block scope inside a function: an ABSOLUTE `[low_pc, high_pc)` code range and the scalar
/// variables live within it (a scalar match's binders). Emitted as a `DW_TAG_lexical_block` DIE.
pub struct DwarfScope {
    pub low_pc: u32,
    pub high_pc: u32,
    pub vars: Vec<DwarfVar>,
}

/// A named scalar local to describe (D3): its source name, wasm local slot, base type, and whether it
/// is a function PARAMETER (`DW_TAG_formal_parameter`) or a `let`-binding local (`DW_TAG_variable`).
/// Either emits a `DW_OP_WASM_location` pointing at the local slot + a `DW_AT_type` referencing the
/// matching `DW_TAG_base_type` — the tag is the only difference, so a debugger shows args and locals
/// distinctly.
///
//= spec/capabilities/debug-information.md#debug-information-may-carry-source-level-names-and-types
//# Debug information MAY carry the source-level name of a definition or binding, so that an external tool can present a value under the name its source gives it.
pub struct DwarfVar {
    pub name: String,
    pub slot: u32,
    pub base: BaseType,
    pub is_param: bool,
}

/// A scalar base type — the (encoding, byte size) a `DW_TAG_base_type` describes. Distinct values are
/// deduplicated into one base-type DIE each, referenced by variables via `DW_AT_type`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BaseType {
    /// A `DW_ATE_*` encoding (signed / unsigned / boolean).
    pub encoding: u8,
    /// The size in bytes (1/2/4/8 for the integer widths; 1 for bool).
    pub byte_size: u8,
    /// A stable display name for the type (`i64`, `u8`, `bool`, …) — the base-type DIE's `DW_AT_name`.
    pub name: &'static str,
}

/// The DWARF base type for a scalar Cadenza type — an integer width (signed/unsigned, 1/2/4/8 bytes)
/// or `Bool`. `None` for a non-scalar (a heap-handle compound), which gets no `DW_TAG_variable` (DWARF
/// cannot walk the tagless heap, §3). The name is a stable DWARF-facing spelling (`i64`, `u8`, `bool`).
///
//= spec/capabilities/debug-information.md#debug-information-may-carry-source-level-names-and-types
//# Debug information MAY carry the source-level type of a binding as descriptive information, so that an external tool can present a value's type even though the executable form carries no runtime type.
pub fn base_type_of(ty: &crate::ty::Ty) -> Option<BaseType> {
    use crate::ty::Ty;
    match ty {
        Ty::Bool => Some(BaseType {
            encoding: dw::ATE_BOOLEAN,
            byte_size: 1,
            name: "bool",
        }),
        Ty::Int(it) => {
            let signed = it.ground_signed();
            let bits = it.ground_width();
            let byte_size = (bits / 8).max(1) as u8;
            let encoding = if signed {
                dw::ATE_SIGNED
            } else {
                dw::ATE_UNSIGNED
            };
            // A stable name per (signedness, width). Unknown widths fall back to the 64-bit spelling.
            let name = match (signed, bits) {
                (true, 8) => "i8",
                (true, 16) => "i16",
                (true, 32) => "i32",
                (true, _) => "i64",
                (false, 8) => "u8",
                (false, 16) => "u16",
                (false, 32) => "u32",
                (false, _) => "u64",
            };
            Some(BaseType {
                encoding,
                byte_size,
                name,
            })
        }
        // A float is a scalar the runtime holds in a wasm `f32`/`f64` local, so it earns a base type
        // (`DW_ATE_float`) exactly like an integer — a debugger can then `print` a float argument. The
        // width is an IEEE format (32/64); an unresolved width grounds to `Float64` (`ground_width`).
        Ty::Float(ft) => {
            let bits = ft.ground_width();
            let (byte_size, name) = if bits <= 32 { (4, "f32") } else { (8, "f64") };
            Some(BaseType {
                encoding: dw::ATE_FLOAT,
                byte_size,
                name,
            })
        }
        // A nominal newtype is ERASED to its underlying value (`(type UserId (Mk Int64))` is a runtime
        // i64), so it is describable exactly when its underlying scalar is — recurse through the tag to
        // the inner type's base type. A nominal over a compound recurses to `None` (the tagless heap is
        // not describable, §3). The DIE uses the UNDERLYING scalar's spelling (`i64`, not `UserId`); a
        // debugger prints the erased value, which is what the runtime actually holds.
        Ty::Nominal { inner, .. } => base_type_of(inner),
        _ => None,
    }
}

/// Build the four `.debug_*` custom sections for `funcs`, concatenated in the order a core module wants
/// them appended. `module_path` is the tree-relative source path (the DWARF file-table + CU name).
/// Returns the bytes to append to the core module (after the code section). Empty `funcs` still yields
/// a valid one-file, zero-subprogram CU (a program with no emitted function is degenerate but valid).
///
/// Every string, DIE, base type, and line row is interned/emitted in `funcs` EMISSION order (which the
/// layout fixes from the source), and no wall-clock, host path, or nondeterministic collection iteration
/// enters the bytes — so two derivations of the same source with the same toolchain byte-match:
///
//= spec/capabilities/debug-information.md#debug-information-is-a-deterministic-function-of-source-and-toolchain
//# The debug information the compiler emits MUST be a deterministic function of the canonical source and the pinned toolchain, so that two derivations of the same source with the same toolchain emit byte-identical debug information.
///
//= spec/capabilities/debug-information.md#debug-information-is-a-deterministic-function-of-source-and-toolchain
//# The order in which debug information records its entries MUST be a deterministic function of the source, independent of filesystem enumeration order or nondeterministic collection iteration.
pub fn debug_sections(module_path: &str, funcs: &[DwarfFunc]) -> Vec<u8> {
    let mut str_tab = StrTab::default();
    // Intern the CU-level strings first (stable offsets, source-determined order).
    let producer_off = str_tab.intern(PRODUCER);
    let name_off = str_tab.intern(module_path);
    let comp_dir_off = str_tab.intern(COMP_DIR);
    // Each function name interned; keep the offsets in order.
    let fn_name_offs: Vec<u32> = funcs.iter().map(|f| str_tab.intern(&f.name)).collect();

    // Collect the DISTINCT scalar base types across every function's vars, in first-seen order (so a
    // single `DW_TAG_base_type` DIE serves every variable of that type), and intern each var's name +
    // each base type's name. `var_name_off` is keyed by (func index, var index).
    let mut base_types: Vec<(BaseType, u32)> = Vec::new();
    let mut seen_bt: std::collections::HashSet<BaseType> = std::collections::HashSet::new();
    let mut var_name_off: std::collections::HashMap<(usize, usize), u32> =
        std::collections::HashMap::new();
    // Scope-var name offsets, keyed by (func index, scope index, var index) — the match-binder locals.
    let mut scope_name_off: std::collections::HashMap<(usize, usize, usize), u32> =
        std::collections::HashMap::new();
    let intern_bt = |str_tab: &mut StrTab,
                     base_types: &mut Vec<(BaseType, u32)>,
                     seen_bt: &mut std::collections::HashSet<BaseType>,
                     bt: BaseType| {
        if seen_bt.insert(bt) {
            let bt_name_off = str_tab.intern(bt.name);
            base_types.push((bt, bt_name_off));
        }
    };
    for (fi, f) in funcs.iter().enumerate() {
        for (vi, v) in f.vars.iter().enumerate() {
            var_name_off.insert((fi, vi), str_tab.intern(&v.name));
            intern_bt(&mut str_tab, &mut base_types, &mut seen_bt, v.base);
        }
        // A scope's binder vars intern their names + base types too (same DIE pool), so the
        // lexical-block `DW_TAG_variable`s resolve their `DW_AT_type` ref4 to an emitted base type.
        for (si, sc) in f.scopes.iter().enumerate() {
            for (vi, v) in sc.vars.iter().enumerate() {
                scope_name_off.insert((fi, si, vi), str_tab.intern(&v.name));
                intern_bt(&mut str_tab, &mut base_types, &mut seen_bt, v.base);
            }
        }
    }

    // The whole-CU code range: low = min low_pc, high = max high_pc (0..0 when no functions).
    let cu_low = funcs.iter().map(|f| f.low_pc).min().unwrap_or(0);
    let cu_high = funcs.iter().map(|f| f.high_pc).max().unwrap_or(0);

    let debug_line = build_line_program(module_path, funcs);
    let debug_abbrev = build_abbrev();
    let debug_info = build_info(
        &fn_name_offs,
        funcs,
        producer_off,
        name_off,
        comp_dir_off,
        cu_low,
        cu_high,
        &base_types,
        &var_name_off,
        &scope_name_off,
    );

    // Append as custom sections (id 0): each is `<id=0> <uleb total-len> <name-uleb-len><name><payload>`.
    let mut out = Vec::new();
    out.extend_from_slice(&custom_section(".debug_abbrev", &debug_abbrev));
    out.extend_from_slice(&custom_section(".debug_info", &debug_info));
    out.extend_from_slice(&custom_section(".debug_str", &str_tab.bytes));
    out.extend_from_slice(&custom_section(".debug_line", &debug_line));
    out
}

/// The byte offset, within a core module, where the CODE section's PAYLOAD begins — i.e. the first
/// byte after the code section's id byte + length prefix, which is where [`FuncCodeRange`]'s offsets
/// (`code_start`/`code_end`) are measured from. The "DWARF for WebAssembly" convention makes a code
/// address a module-relative byte offset, so an absolute DWARF address is `this_base + range.start`.
/// Walks the well-defined section framing (`<id byte> <uleb length> <payload>`) from just past the
/// 8-byte core magic, returning the code section's (id 10) payload offset. `None` if there is no code
/// section (a degenerate module) or the bytes are malformed (total — never panics).
///
/// [`FuncCodeRange`]: crate::backend::wasm::serialize::FuncCodeRange
pub fn code_section_payload_base(core: &[u8]) -> Option<u32> {
    const CODE_SECTION_ID: u8 = 10;
    let mut pos = 8usize; // past `\0asm` + version (the 8-byte core header)
    while pos < core.len() {
        let id = core[pos];
        pos += 1;
        // Read the section's uleb length, tracking how many bytes it consumed.
        let (len, len_bytes) = read_uleb(core, pos)?;
        pos += len_bytes;
        if id == CODE_SECTION_ID {
            return u32::try_from(pos).ok();
        }
        pos = pos.checked_add(len as usize)?;
    }
    None
}

/// Read an unsigned LEB128 at `pos`, returning `(value, bytes_consumed)`. Total — `None` on truncation.
fn read_uleb(bytes: &[u8], mut pos: usize) -> Option<(u64, usize)> {
    let start = pos;
    let mut result: u64 = 0;
    let mut shift = 0u32;
    loop {
        let byte = *bytes.get(pos)?;
        pos += 1;
        result |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some((result, pos - start));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

/// Wrap the `.debug_*` custom `sections` in a STANDALONE bare core wasm module (Mode S — the sidecar
/// `dwarf` artifact). Just the 8-byte core header (`\0asm` + version 1) followed by the custom sections
/// — no type/func/code sections, so it defines nothing executable; it exists only to carry the DWARF a
/// debugger loads alongside the runnable component (the `external_debug_info` target). `wasm-tools` and
/// `llvm-dwarfdump` parse it exactly as they parse the embedded core's sections.
pub fn standalone_dwarf_module(sections: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + sections.len());
    out.extend_from_slice(crate::backend::wasm::wasm_abi::CORE_MAGIC);
    out.extend_from_slice(sections);
    out
}

/// The `external_debug_info` custom section — the "DWARF for WebAssembly" convention for a runnable
/// module that carries NO embedded DWARF but points a debugger at a DETACHED sidecar file (Mode S).
/// The section's payload is a single length-prefixed UTF-8 name (the sidecar's path/URL, relative to
/// the module). A debugger reading the runnable finds this and loads the sidecar's `.debug_*` sections
/// automatically, so the code addresses resolve without a manual `-s`/`--symbols` flag. Appended to the
/// embedded core module like the other debug sections — inert (moves no executed byte) and strippable.
///
//= spec/capabilities/debug-information.md#debug-information-may-be-embedded-or-emitted-as-a-sidecar
//# The compiler MUST emit a reference, reachable from the runnable artifact, that identifies the separately emitted debug artifact describing it, so that a tool holding the runnable artifact can locate the debug information for it.
pub fn external_debug_info_section(sidecar_path: &str) -> Vec<u8> {
    let mut payload = Vec::new();
    uleb128(sidecar_path.len() as u64, &mut payload);
    payload.extend_from_slice(sidecar_path.as_bytes());
    custom_section("external_debug_info", &payload)
}

/// A wasm CUSTOM section (id 0): its contents are `<name-len-uleb><name-bytes><payload>`. Reused by
/// `compile` to append the component-level `cdz-result-type` run-wiring section (bytes-second).
pub(crate) fn custom_section(name: &str, payload: &[u8]) -> Vec<u8> {
    let mut contents = Vec::new();
    uleb128(name.len() as u64, &mut contents);
    contents.extend_from_slice(name.as_bytes());
    contents.extend_from_slice(payload);
    section(0, &contents)
}

/// `.debug_abbrev` — the abbreviation table (the DIE "shapes"). Each entry:
/// `<code-uleb> <tag-uleb> <children-byte> ( <attr-uleb> <form-uleb> )* 0 0`; the table ends with a
/// single 0 abbrev code. Seven abbrevs: compile_unit, subprogram (leaf), subprogram-with-children,
/// formal_parameter, base_type, variable (a `let`-binding local — D3), and lexical_block (a scalar
/// match-binder scope — D3).
fn build_abbrev() -> Vec<u8> {
    let mut b = Vec::new();
    let entry = |b: &mut Vec<u8>, code: u64, tag: u64, children: u8, attrs: &[(u64, u64)]| {
        uleb128(code, b);
        uleb128(tag, b);
        b.push(children);
        for &(at, form) in attrs {
            uleb128(at, b);
            uleb128(form, b);
        }
        uleb128(0, b);
        uleb128(0, b);
    };
    // 1: compile_unit, has children (base types + subprograms).
    entry(
        &mut b,
        dw::ABBREV_COMPILE_UNIT,
        dw::TAG_COMPILE_UNIT,
        dw::CHILDREN_YES,
        &[
            (dw::AT_PRODUCER, dw::FORM_STRP),
            (dw::AT_NAME, dw::FORM_STRP),
            (dw::AT_COMP_DIR, dw::FORM_STRP),
            (dw::AT_LANGUAGE, dw::FORM_DATA2),
            (dw::AT_LOW_PC, dw::FORM_ADDR),
            (dw::AT_HIGH_PC, dw::FORM_DATA4),
            (dw::AT_STMT_LIST, dw::FORM_SEC_OFFSET),
        ],
    );
    let sp_attrs = [
        (dw::AT_NAME, dw::FORM_STRP),
        (dw::AT_DECL_FILE, dw::FORM_DATA1),
        (dw::AT_DECL_LINE, dw::FORM_UDATA),
        (dw::AT_LOW_PC, dw::FORM_ADDR),
        (dw::AT_HIGH_PC, dw::FORM_DATA4),
    ];
    // 2: subprogram WITHOUT children (a function with no scalar locals).
    entry(
        &mut b,
        dw::ABBREV_SUBPROGRAM,
        dw::TAG_SUBPROGRAM,
        dw::CHILDREN_NO,
        &sp_attrs,
    );
    // 3: subprogram WITH children (a function whose scalar params/locals are described).
    entry(
        &mut b,
        dw::ABBREV_SUBPROGRAM_KIDS,
        dw::TAG_SUBPROGRAM,
        dw::CHILDREN_YES,
        &sp_attrs,
    );
    // 4: formal_parameter — name + type (a DIE ref) + a location expression (the wasm local slot).
    entry(
        &mut b,
        dw::ABBREV_FORMAL_PARAMETER,
        dw::TAG_FORMAL_PARAMETER,
        dw::CHILDREN_NO,
        &[
            (dw::AT_NAME, dw::FORM_STRP),
            (dw::AT_TYPE, dw::FORM_REF4),
            (dw::AT_LOCATION, dw::FORM_EXPRLOC),
        ],
    );
    // 5: base_type — name + encoding + byte size.
    entry(
        &mut b,
        dw::ABBREV_BASE_TYPE,
        dw::TAG_BASE_TYPE,
        dw::CHILDREN_NO,
        &[
            (dw::AT_NAME, dw::FORM_STRP),
            (dw::AT_ENCODING, dw::FORM_DATA1),
            (dw::AT_BYTE_SIZE, dw::FORM_DATA1),
        ],
    );
    // 6: variable — a `let`-binding local. SAME attributes as a formal_parameter (name/type/location);
    // only the tag differs, so a debugger lists it as a local, not an argument.
    entry(
        &mut b,
        dw::ABBREV_VARIABLE,
        dw::TAG_VARIABLE,
        dw::CHILDREN_NO,
        &[
            (dw::AT_NAME, dw::FORM_STRP),
            (dw::AT_TYPE, dw::FORM_REF4),
            (dw::AT_LOCATION, dw::FORM_EXPRLOC),
        ],
    );
    // 7: lexical_block — a scalar match-binder scope. HAS children (its `DW_TAG_variable`s) + a PC range
    // (low_pc addr + high_pc as a size, matching the subprogram encoding) so a debugger scopes the
    // binder to exactly the match's instructions.
    entry(
        &mut b,
        dw::ABBREV_LEXICAL_BLOCK,
        dw::TAG_LEXICAL_BLOCK,
        dw::CHILDREN_YES,
        &[
            (dw::AT_LOW_PC, dw::FORM_ADDR),
            (dw::AT_HIGH_PC, dw::FORM_DATA4),
        ],
    );
    // End of the abbreviation table.
    uleb128(0, &mut b);
    b
}

/// The DWARF-4 32-bit CU header size in bytes: `unit_length(4) version(2) abbrev_offset(4) addr_size(1)`.
/// DIE offsets (`DW_FORM_ref4`) are measured from the CU START (the `.debug_info` section start, since
/// there is one CU), so the first DIE sits at this offset.
const CU_HEADER_LEN: usize = 4 + 2 + 4 + 1;

/// `.debug_info` — the CU header + the compile_unit DIE, whose children are: one `DW_TAG_base_type` per
/// distinct scalar type (D3), then one subprogram DIE per function. Each subprogram's children are its
/// scalar params (`DW_TAG_formal_parameter`) and `let`-binding locals (`DW_TAG_variable`), each
/// referencing a base type via `DW_AT_type`, plus a `DW_TAG_lexical_block` per scalar match-binder scope
/// (its own PC range fencing its `DW_TAG_variable` binders). `fn_name_offs` and the `base_types` name
/// offsets carry the `.debug_str` offsets; a local `base_die_off` map (built as the base-type DIEs are
/// emitted) resolves each variable's `DW_AT_type` ref4. CU header (DWARF 4, 32-bit); the tree is closed
/// by 0 abbrev codes.
///
/// The source names and type spellings written here land in the inert `.debug_info`/`.debug_str`
/// custom sections, which no emitted instruction addresses — so carrying them for an external tool does
/// not make them reachable by the running component (erasure is preserved):
///
//= spec/capabilities/debug-information.md#debug-information-may-carry-source-level-names-and-types
//# A source-level name or type carried in debug information MUST NOT be reachable by the running component, so that carrying it for an external tool does not reintroduce the runtime type reflection erasure removes.
#[allow(clippy::too_many_arguments)]
fn build_info(
    fn_name_offs: &[u32],
    funcs: &[DwarfFunc],
    producer_off: u32,
    name_off: u32,
    comp_dir_off: u32,
    cu_low: u32,
    cu_high: u32,
    base_types: &[(BaseType, u32)], // (type, its .debug_str name offset), in emission order
    var_name_off: &std::collections::HashMap<(usize, usize), u32>, // (func_ix, var_ix) → name strp
    scope_name_off: &std::collections::HashMap<(usize, usize, usize), u32>, // (func_ix, scope_ix, var_ix) → name strp
) -> Vec<u8> {
    // DIE bytes accumulate AFTER the CU header; a `DW_FORM_ref4` is `CU_HEADER_LEN + die.len()` at the
    // point a DIE begins. Record each base type's DIE offset so variables can reference it.
    let mut die = Vec::new();
    let off_at = |die: &Vec<u8>| (CU_HEADER_LEN + die.len()) as u32;

    // compile_unit DIE (abbrev 1).
    uleb128(dw::ABBREV_COMPILE_UNIT, &mut die);
    die.extend_from_slice(&producer_off.to_le_bytes()); // DW_AT_producer (strp)
    die.extend_from_slice(&name_off.to_le_bytes()); // DW_AT_name (strp)
    die.extend_from_slice(&comp_dir_off.to_le_bytes()); // DW_AT_comp_dir (strp)
    die.extend_from_slice(&dw::LANG_C.to_le_bytes()); // DW_AT_language (data2) — DW_LANG_C
    die.extend_from_slice(&cu_low.to_le_bytes()); // DW_AT_low_pc (addr, 4 bytes)
    die.extend_from_slice(&cu_high.saturating_sub(cu_low).to_le_bytes()); // DW_AT_high_pc (data4 = size)
    die.extend_from_slice(&0u32.to_le_bytes()); // DW_AT_stmt_list (sec_offset → .debug_line start)

    // Base-type DIEs first (abbrev 5), recording each one's offset for variable `DW_AT_type` refs.
    let mut base_die_off: std::collections::HashMap<BaseType, u32> =
        std::collections::HashMap::new();
    for &(bt, name_off) in base_types {
        base_die_off.insert(bt, off_at(&die));
        uleb128(dw::ABBREV_BASE_TYPE, &mut die);
        die.extend_from_slice(&name_off.to_le_bytes()); // DW_AT_name (strp)
        die.push(bt.encoding); // DW_AT_encoding (data1)
        die.push(bt.byte_size); // DW_AT_byte_size (data1)
    }

    // Emit one scalar-variable DIE (a formal_parameter for a param, else a variable — a `let` local or a
    // match binder) with a name/type-ref/location. Shared by the subprogram's direct vars and a lexical
    // block's binders, so both encode identically.
    let emit_var = |die: &mut Vec<u8>, v: &DwarfVar, name_off: u32| {
        uleb128(
            if v.is_param {
                dw::ABBREV_FORMAL_PARAMETER
            } else {
                dw::ABBREV_VARIABLE
            },
            die,
        );
        die.extend_from_slice(&name_off.to_le_bytes()); // DW_AT_name (strp)
        die.extend_from_slice(&base_die_off[&v.base].to_le_bytes()); // DW_AT_type (ref4 → base type DIE)
        // DW_AT_location (exprloc): `DW_OP_WASM_location 0x00 <local-idx-uleb>`. Length-prefixed.
        let mut loc = vec![dw::OP_WASM_LOCATION, 0x00];
        uleb128(v.slot as u64, &mut loc);
        uleb128(loc.len() as u64, die); // exprloc length
        die.extend_from_slice(&loc);
    };

    // One subprogram DIE per function; abbrev 3 (has children) when it has scalar vars OR match-binder
    // scopes (lexical-block children), else abbrev 2.
    for (fi, (f, &fn_name_off)) in funcs.iter().zip(fn_name_offs).enumerate() {
        let has_kids = !f.vars.is_empty() || !f.scopes.is_empty();
        uleb128(
            if has_kids {
                dw::ABBREV_SUBPROGRAM_KIDS
            } else {
                dw::ABBREV_SUBPROGRAM
            },
            &mut die,
        );
        die.extend_from_slice(&fn_name_off.to_le_bytes()); // DW_AT_name (strp)
        die.push(1u8); // DW_AT_decl_file (data1) — file 1 (our single file)
        uleb128(f.line.max(1) as u64, &mut die); // DW_AT_decl_line (udata)
        die.extend_from_slice(&f.low_pc.to_le_bytes()); // DW_AT_low_pc (addr)
        die.extend_from_slice(&f.high_pc.saturating_sub(f.low_pc).to_le_bytes()); // DW_AT_high_pc (data4)
        // A formal_parameter (a param) / variable (a `let`-binding local) DIE per scalar var — the tag
        // differs so a debugger shows args and locals distinctly; the attributes are identical.
        for (vi, v) in f.vars.iter().enumerate() {
            emit_var(&mut die, v, var_name_off[&(fi, vi)]);
        }
        // A `DW_TAG_lexical_block` per scalar match: its PC range fences the binder vars to the match's
        // instructions (the binder slot is reused elsewhere, so a function-scoped var would misreport).
        for (si, sc) in f.scopes.iter().enumerate() {
            uleb128(dw::ABBREV_LEXICAL_BLOCK, &mut die);
            die.extend_from_slice(&sc.low_pc.to_le_bytes()); // DW_AT_low_pc (addr)
            die.extend_from_slice(&sc.high_pc.saturating_sub(sc.low_pc).to_le_bytes()); // DW_AT_high_pc (data4)
            for (vi, v) in sc.vars.iter().enumerate() {
                emit_var(&mut die, v, scope_name_off[&(fi, si, vi)]);
            }
            uleb128(0, &mut die); // terminate the lexical block's children
        }
        if has_kids {
            uleb128(0, &mut die); // terminate the subprogram's children
        }
    }
    // Terminate the compile_unit's children.
    uleb128(0, &mut die);

    // CU header — unit_length is the byte count AFTER the length field itself.
    let mut out = Vec::new();
    let unit_len = (2 + 4 + 1 + die.len()) as u32; // version + abbrev_off + addr_size + DIEs
    out.extend_from_slice(&unit_len.to_le_bytes());
    // The concrete format is DWARF v4, fixed here (not a build knob), so every debug build emits the
    // same interchange format a standard debugger reads.
    //= spec/capabilities/debug-information.md#debug-information-uses-an-interchange-format
    //# The concrete debug-information format MUST be pinned at the declared-default location, so that two builds that emit debug information emit it in the same format.
    out.extend_from_slice(&4u16.to_le_bytes()); // DWARF version 4
    out.extend_from_slice(&0u32.to_le_bytes()); // .debug_abbrev offset
    out.push(4u8); // address size (wasm32 code offset)
    out.extend_from_slice(&die);
    out
}

/// `.debug_line` — a DWARF 4 line-number program. Header + program. For each function in ascending code
/// order it emits one row per source POSITION the body visits: each of `DwarfFunc.rows`
/// (`(offset, line, col)`, per-construct) sets the address to that offset, advances the line register,
/// sets the column register, and `copy`s a row — so a debugger steps construct-by-construct and can
/// highlight the exact sub-expression on a line. A function with no per-construct rows falls back to a
/// single row at its `low_pc`/`line` (function granularity). The program ends with `end_sequence` at the
/// highest `high_pc`.
///
//= spec/capabilities/debug-information.md#debug-information-relates-an-execution-position-to-its-source
//# Debug information MUST relate a position in the executable artifact to the source construct of the canonical representation it derives from, so that an external tool can present an execution position as a location in the program's source.
fn build_line_program(module_path: &str, funcs: &[DwarfFunc]) -> Vec<u8> {
    // ── The line-number program body ──
    let mut prog = Vec::new();
    // The line register starts at 1 and the column register at 0 (DWARF initial state); track both to
    // emit minimal advances/sets.
    let mut cur_line: i64 = 1;
    let mut cur_col: u32 = 0;
    let mut ordered: Vec<&DwarfFunc> = funcs.iter().collect();
    ordered.sort_by_key(|f| f.low_pc);
    // Ensure file register is 1 (our single file) — explicit for clarity.
    prog.push(dw::LNS_SET_FILE);
    uleb128(1, &mut prog);
    for f in &ordered {
        // The rows for this function: its per-construct `(offset, line, col)` rows if present, else a
        // single row at the function entry, column 0 (function-granularity fallback for a
        // single-construct body). Each row sets the address (an absolute code offset — simplest, no
        // `advance_pc` arithmetic), advances the line register + sets the column register, then `copy`
        // emits the row.
        let fallback = [(f.low_pc, f.line.max(1), 0u32)];
        let rows: &[(u32, u32, u32)] = if f.rows.is_empty() {
            &fallback
        } else {
            &f.rows
        };
        for &(offset, line, col) in rows {
            // DW_LNE_set_address <addr> — an extended opcode: 0x00 <len-uleb> <sub-opcode> <operand>.
            prog.push(0x00);
            uleb128(1 + 4, &mut prog); // 1 (sub-opcode) + 4 (a 4-byte address)
            prog.push(dw::LNE_SET_ADDRESS);
            prog.extend_from_slice(&offset.to_le_bytes());
            // Advance the line register to this row's line.
            let target = line.max(1) as i64;
            if target != cur_line {
                prog.push(dw::LNS_ADVANCE_LINE);
                sleb128(target - cur_line, &mut prog);
                cur_line = target;
            }
            // Set the column register to this row's column (a debugger highlights the sub-expression).
            if col != cur_col {
                prog.push(dw::LNS_SET_COLUMN);
                uleb128(col as u64, &mut prog);
                cur_col = col;
            }
            // Emit a row (DW_LNS_copy).
            prog.push(dw::LNS_COPY);
        }
    }
    // Close the sequence at the highest high_pc: set the address there, then end_sequence.
    let end_addr = ordered.iter().map(|f| f.high_pc).max().unwrap_or(0);
    prog.push(0x00);
    uleb128(1 + 4, &mut prog);
    prog.push(dw::LNE_SET_ADDRESS);
    prog.extend_from_slice(&end_addr.to_le_bytes());
    prog.push(0x00);
    uleb128(1, &mut prog);
    prog.push(dw::LNE_END_SEQUENCE);

    // ── The line-program header (DWARF 4) ──
    // Layout after unit_length: version(u16) header_length(u32) min_inst_len(u8) max_ops_per_inst(u8)
    // default_is_stmt(u8) line_base(i8) line_range(u8) opcode_base(u8) std_opcode_lengths[opcode_base-1]
    // include_directories(sequence, NUL-terminated, ends with empty) file_names(sequence, ends empty).
    // everything AFTER header_length, up to & including file_names:
    //   min_inst_len=1, max_ops_per_inst=1, default_is_stmt=1, line_base=-5, line_range=14,
    //   opcode_base=13, then standard_opcode_lengths for opcodes 1..=12 (DWARF 4 canonical values).
    let mut header_rest: Vec<u8> = vec![
        1,
        1,
        1,
        (-5i8) as u8,
        14,
        13,
        0,
        1,
        1,
        1,
        1,
        0,
        0,
        0,
        1,
        0,
        0,
        1,
    ];
    // include_directories: none → a single terminating NUL.
    header_rest.push(0);
    // file_names: one file = the module path, dir index 0, mtime 0, size 0; then a terminating NUL.
    header_rest.extend_from_slice(module_path.as_bytes());
    header_rest.push(0); // NUL-terminate the name
    uleb128(0, &mut header_rest); // directory index
    uleb128(0, &mut header_rest); // mtime
    uleb128(0, &mut header_rest); // size
    header_rest.push(0); // end of file_names

    // header_length = bytes from AFTER header_length field up to the start of the program (= header_rest).
    let header_length = header_rest.len() as u32;

    let mut unit = Vec::new();
    unit.extend_from_slice(&4u16.to_le_bytes()); // version 4
    unit.extend_from_slice(&header_length.to_le_bytes()); // header_length
    unit.extend_from_slice(&header_rest);
    unit.extend_from_slice(&prog);

    // unit_length prefix (bytes after the length field).
    let mut out = Vec::new();
    out.extend_from_slice(&(unit.len() as u32).to_le_bytes());
    out.extend_from_slice(&unit);
    out
}
