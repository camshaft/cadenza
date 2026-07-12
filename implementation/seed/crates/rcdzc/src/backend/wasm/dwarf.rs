//! Hand-rolled DWARF for the embedded core module — D2 of `DESIGN-debug-info-rcdzc.md`.
//!
//! Emits the four `.debug_*` custom sections a debugger needs to STEP through Cadenza source:
//! - `.debug_str` — the string table the DIEs reference by offset (names, producer, dir).
//! - `.debug_abbrev` — the abbreviation table (the DIE "shapes": compile_unit, subprogram).
//! - `.debug_info` — one `DW_TAG_compile_unit` DIE + one `DW_TAG_subprogram` per function, each with
//!   its code-offset range (`DW_AT_low_pc`/`DW_AT_high_pc`).
//! - `.debug_line` — the line-number program mapping each function's code offset → (file, line).
//!
//! These are CUSTOM sections (wasm section id 0) appended to the embedded CORE MODULE — the standard
//! place wasm tools/debuggers look ("DWARF for WebAssembly"). Code addresses are byte offsets into the
//! core module's CODE section (`code_offset_base + FuncCodeRange.code_start`, computed by the caller).
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
//! Scope: FUNCTION-granularity line rows (one row per function at its entry, from D1b's
//! `FuncCodeRange`). Per-statement rows are a later refinement. This is DWARF v4 (address size 4 — a
//! wasm32 code offset is 32-bit), the version GDB/LLDB/`wasmtime -D debug-info` and the Chrome DWARF
//! extension all accept.

use crate::backend::wasm::encode::{section, sleb128, uleb128};

/// A fixed, non-provenance-leaking producer string (design §4 — never the live toolchain banner).
const PRODUCER: &str = "cadenza-rcdzc";
/// A fixed sentinel compilation directory — never the build directory (design §4).
const COMP_DIR: &str = "/";

// ── DWARF constants (from the DWARF 4 spec; hand-transcribed — the values are stable and standardized,
//    unlike the wasm opcodes which come from the generated table). ──────────────────────────────────
mod dw {
    // Tags.
    pub const TAG_COMPILE_UNIT: u64 = 0x11;
    pub const TAG_SUBPROGRAM: u64 = 0x2e;
    // Attributes.
    pub const AT_NAME: u64 = 0x03;
    pub const AT_LOW_PC: u64 = 0x11;
    pub const AT_HIGH_PC: u64 = 0x12;
    pub const AT_STMT_LIST: u64 = 0x10;
    pub const AT_COMP_DIR: u64 = 0x1b;
    pub const AT_PRODUCER: u64 = 0x25;
    pub const AT_DECL_FILE: u64 = 0x3a;
    pub const AT_DECL_LINE: u64 = 0x3b;
    // Forms.
    pub const FORM_ADDR: u64 = 0x01;
    pub const FORM_DATA4: u64 = 0x06;
    pub const FORM_DATA1: u64 = 0x0b;
    pub const FORM_STRP: u64 = 0x0e; // offset into .debug_str
    pub const FORM_SEC_OFFSET: u64 = 0x17;
    pub const FORM_UDATA: u64 = 0x0f;
    // Children flag.
    pub const CHILDREN_YES: u8 = 0x01;
    pub const CHILDREN_NO: u8 = 0x00;
    // Line-number standard/extended opcodes.
    pub const LNS_COPY: u8 = 0x01;
    pub const LNS_ADVANCE_LINE: u8 = 0x03;
    pub const LNS_SET_FILE: u8 = 0x04;
    pub const LNE_END_SEQUENCE: u8 = 0x01;
    pub const LNE_SET_ADDRESS: u8 = 0x02;
    // Our abbreviation codes (arbitrary, must match between .debug_abbrev and .debug_info).
    pub const ABBREV_COMPILE_UNIT: u64 = 1;
    pub const ABBREV_SUBPROGRAM: u64 = 2;
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
/// section), and source line (1-based; 0 = unknown). Built by the caller from `FuncCodeRange` + the
/// `spans` side-table + a newline index over the source (or 1 when only byte spans are known).
pub struct DwarfFunc {
    pub name: String,
    pub low_pc: u32,
    pub high_pc: u32,
    pub line: u32,
}

/// Build the four `.debug_*` custom sections for `funcs`, concatenated in the order a core module wants
/// them appended. `module_path` is the tree-relative source path (the DWARF file-table + CU name).
/// Returns the bytes to append to the core module (after the code section). Empty `funcs` still yields
/// a valid one-file, zero-subprogram CU (a program with no emitted function is degenerate but valid).
pub fn debug_sections(module_path: &str, funcs: &[DwarfFunc]) -> Vec<u8> {
    let mut str_tab = StrTab::default();
    // Intern the CU-level strings first (stable offsets, source-determined order).
    let producer_off = str_tab.intern(PRODUCER);
    let name_off = str_tab.intern(module_path);
    let comp_dir_off = str_tab.intern(COMP_DIR);
    // Each function name interned; keep the offsets in order.
    let fn_name_offs: Vec<u32> = funcs.iter().map(|f| str_tab.intern(&f.name)).collect();

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

/// A wasm CUSTOM section (id 0): its contents are `<name-len-uleb><name-bytes><payload>`.
fn custom_section(name: &str, payload: &[u8]) -> Vec<u8> {
    let mut contents = Vec::new();
    uleb128(name.len() as u64, &mut contents);
    contents.extend_from_slice(name.as_bytes());
    contents.extend_from_slice(payload);
    section(0, &contents)
}

/// `.debug_abbrev` — two abbreviations: 1 = compile_unit (has children), 2 = subprogram (no children).
/// Each entry: `<code-uleb> <tag-uleb> <children-byte> ( <attr-uleb> <form-uleb> )* 0 0`. The table ends
/// with a single 0 abbrev code.
fn build_abbrev() -> Vec<u8> {
    let mut b = Vec::new();
    // Abbrev 1: DW_TAG_compile_unit, has children (the subprograms).
    uleb128(dw::ABBREV_COMPILE_UNIT, &mut b);
    uleb128(dw::TAG_COMPILE_UNIT, &mut b);
    b.push(dw::CHILDREN_YES);
    let cu_attrs = [
        (dw::AT_PRODUCER, dw::FORM_STRP),
        (dw::AT_NAME, dw::FORM_STRP),
        (dw::AT_COMP_DIR, dw::FORM_STRP),
        (dw::AT_LOW_PC, dw::FORM_ADDR),
        (dw::AT_HIGH_PC, dw::FORM_DATA4),
        (dw::AT_STMT_LIST, dw::FORM_SEC_OFFSET),
    ];
    for (at, form) in cu_attrs {
        uleb128(at, &mut b);
        uleb128(form, &mut b);
    }
    uleb128(0, &mut b);
    uleb128(0, &mut b);
    // Abbrev 2: DW_TAG_subprogram, no children.
    uleb128(dw::ABBREV_SUBPROGRAM, &mut b);
    uleb128(dw::TAG_SUBPROGRAM, &mut b);
    b.push(dw::CHILDREN_NO);
    let sp_attrs = [
        (dw::AT_NAME, dw::FORM_STRP),
        (dw::AT_DECL_FILE, dw::FORM_DATA1),
        (dw::AT_DECL_LINE, dw::FORM_UDATA),
        (dw::AT_LOW_PC, dw::FORM_ADDR),
        (dw::AT_HIGH_PC, dw::FORM_DATA4),
    ];
    for (at, form) in sp_attrs {
        uleb128(at, &mut b);
        uleb128(form, &mut b);
    }
    uleb128(0, &mut b);
    uleb128(0, &mut b);
    // End of the abbreviation table.
    uleb128(0, &mut b);
    b
}

/// `.debug_info` — the CU header + the compile_unit DIE + one subprogram DIE per function.
/// CU header (DWARF 4, 32-bit): `unit_length(u32) version(u16=4) abbrev_offset(u32=0) addr_size(u8=4)`,
/// then the DIE tree, closed by a 0 abbrev code (the compile_unit's children terminator).
fn build_info(
    fn_name_offs: &[u32],
    funcs: &[DwarfFunc],
    producer_off: u32,
    name_off: u32,
    comp_dir_off: u32,
    cu_low: u32,
    cu_high: u32,
) -> Vec<u8> {
    let mut die = Vec::new();
    // compile_unit DIE (abbrev 1).
    uleb128(dw::ABBREV_COMPILE_UNIT, &mut die);
    die.extend_from_slice(&producer_off.to_le_bytes()); // DW_AT_producer (strp)
    die.extend_from_slice(&name_off.to_le_bytes()); // DW_AT_name (strp)
    die.extend_from_slice(&comp_dir_off.to_le_bytes()); // DW_AT_comp_dir (strp)
    die.extend_from_slice(&cu_low.to_le_bytes()); // DW_AT_low_pc (addr, 4 bytes)
    die.extend_from_slice(&cu_high.saturating_sub(cu_low).to_le_bytes()); // DW_AT_high_pc (data4 = size)
    die.extend_from_slice(&0u32.to_le_bytes()); // DW_AT_stmt_list (sec_offset → .debug_line start)

    // One subprogram DIE (abbrev 2) per function.
    for (f, &name_off) in funcs.iter().zip(fn_name_offs) {
        uleb128(dw::ABBREV_SUBPROGRAM, &mut die);
        die.extend_from_slice(&name_off.to_le_bytes()); // DW_AT_name (strp)
        die.push(1u8); // DW_AT_decl_file (data1) — file 1 (our single file)
        uleb128(f.line.max(1) as u64, &mut die); // DW_AT_decl_line (udata)
        die.extend_from_slice(&f.low_pc.to_le_bytes()); // DW_AT_low_pc (addr)
        die.extend_from_slice(&f.high_pc.saturating_sub(f.low_pc).to_le_bytes()); // DW_AT_high_pc (data4)
    }
    // Terminate the compile_unit's children.
    uleb128(0, &mut die);

    // CU header — unit_length is the byte count AFTER the length field itself.
    let mut out = Vec::new();
    let unit_len = (2 + 4 + 1 + die.len()) as u32; // version + abbrev_off + addr_size + DIEs
    out.extend_from_slice(&unit_len.to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes()); // DWARF version 4
    out.extend_from_slice(&0u32.to_le_bytes()); // .debug_abbrev offset
    out.push(4u8); // address size (wasm32 code offset)
    out.extend_from_slice(&die);
    out
}

/// `.debug_line` — a DWARF 4 line-number program with one row per function (function granularity).
/// Header + program. The program, for each function in ascending code order: set the address to the
/// function's `low_pc`, advance the line to its source line, `copy` (emit a row), and finally
/// `end_sequence` at the last function's `high_pc`.
fn build_line_program(module_path: &str, funcs: &[DwarfFunc]) -> Vec<u8> {
    // ── The line-number program body ──
    let mut prog = Vec::new();
    // The line register starts at 1 (DWARF initial state); track it to emit minimal advances.
    let mut cur_line: i64 = 1;
    let mut ordered: Vec<&DwarfFunc> = funcs.iter().collect();
    ordered.sort_by_key(|f| f.low_pc);
    // Ensure file register is 1 (our single file) — explicit for clarity.
    prog.push(dw::LNS_SET_FILE);
    uleb128(1, &mut prog);
    for f in &ordered {
        // DW_LNE_set_address <addr> — an extended opcode: 0x00 <len-uleb> <sub-opcode> <operand>.
        prog.push(0x00);
        uleb128(1 + 4, &mut prog); // 1 (sub-opcode) + 4 (a 4-byte address)
        prog.push(dw::LNE_SET_ADDRESS);
        prog.extend_from_slice(&f.low_pc.to_le_bytes());
        // Advance the line register to this function's line.
        let target = f.line.max(1) as i64;
        if target != cur_line {
            prog.push(dw::LNS_ADVANCE_LINE);
            sleb128(target - cur_line, &mut prog);
            cur_line = target;
        }
        // Emit a row (DW_LNS_copy).
        prog.push(dw::LNS_COPY);
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
