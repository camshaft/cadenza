use crate::abi::Artifact;
use crate::backend::Target;
use crate::compile::compile;
use crate::sidecar::{self, Request};
use crate::spans;
use crate::testkit::{parse, parse_spanned};

/// Read an unsigned LEB128 varint from `b` at `*i`, advancing `*i` past it — the section/name
/// length encoding these raw wasm-framing scanners walk.
fn uleb(b: &[u8], i: &mut usize) -> u64 {
    let (mut r, mut s) = (0u64, 0u32);
    loop {
        let x = b[*i];
        *i += 1;
        r |= u64::from(x & 0x7f) << s;
        s += 7;
        if x & 0x80 == 0 {
            return r;
        }
    }
}

/// The `ast` + `spans` input artifacts for `src` — the pair a debug-enabled driver supplies.
fn debug_inputs(src: &str) -> Vec<Artifact> {
    let (arenas, span_data) = parse_spanned(src);
    vec![
        Artifact::new(Artifact::KIND_AST, "m", crate::codec::encode(&arenas)),
        Artifact::new(spans::KIND_SPANS, "m", spans::encode(&span_data)),
    ]
}

/// Compile `src` to a `component` under `target`, returning the component bytes. A debug target
/// (which requires the `spans` input, §9.4) is given both artifacts; a plain target just the `ast`.
fn component_of(src: &str, target: Target) -> Vec<u8> {
    let inputs = if target.needs_spans() {
        debug_inputs(src)
    } else {
        vec![Artifact::new(
            Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse(src)),
        )]
    };
    let out = compile(&inputs, &[target]);
    assert!(!out.has_error(), "compile failed: {:?}", out.diagnostics);
    out.artifact("component")
        .expect("a component artifact")
        .to_vec()
}

/// Every embedded core module's bytes, in order — the resource envelope embeds several (a dtor plus
/// the main walker core), and the `.debug_*` sections ride in a LATER one, not the first `dtor`. A
/// raw section-framing scan (a component's core-module is section id 1) rather than `wasmparser`,
/// whose stateful reader chokes when a nested module's own `\0asm` magic follows the section header.
fn core_modules_of(component: &[u8]) -> Vec<Vec<u8>> {
    let mut mods = Vec::new();
    if component.get(0..4) != Some(b"\0asm") {
        return mods;
    }
    let mut i = 8; // past `\0asm` + the 4-byte version/layer word
    while i < component.len() {
        let sid = component[i];
        i += 1;
        let len = uleb(component, &mut i) as usize;
        // In a COMPONENT, section id 1 is an embedded core module (its body IS a `\0asm…` module).
        if sid == 1 {
            mods.push(component[i..i + len].to_vec());
        }
        i += len;
    }
    mods
}

/// The bytes of a named custom section inside a CORE module (`\0asm`…), or `None`. A dev-only
/// scanner used to byte-compare a sidecar's `.debug_*` against the embedded component's.
fn custom_section_of(core: &[u8], want: &str) -> Option<Vec<u8>> {
    if core.get(0..4) != Some(b"\0asm") {
        return None;
    }
    let mut i = 8;
    while i < core.len() {
        let sid = core[i];
        i += 1;
        let len = uleb(core, &mut i) as usize;
        let body = &core[i..i + len];
        if sid == 0 {
            let mut j = 0;
            let nl = uleb(body, &mut j) as usize;
            if std::str::from_utf8(&body[j..j + nl]) == Ok(want) {
                return Some(body[j + nl..].to_vec());
            }
        }
        i += len;
    }
    None
}

/// The parsed `name` section: the optional module name and the `(func_index, name)` pairs.
type NameSection = (Option<String>, Vec<(u32, String)>);

/// The embedded core module's bytes, extracted from a component wrapper via `wasmparser` (a dev-only
/// validator, never in the compile path) — the blob that carries the `name` + `.debug_*` sections.
fn core_module_of(component: &[u8]) -> Option<Vec<u8>> {
    use wasmparser::{Chunk, Parser, Payload};
    let mut parser = Parser::new(0);
    let mut offset = 0usize;
    let mut buf = component;
    loop {
        match parser.parse(buf, true).expect("parse component") {
            Chunk::NeedMoreData(_) => return None,
            Chunk::Parsed { consumed, payload } => {
                if let Payload::ModuleSection {
                    unchecked_range, ..
                } = &payload
                {
                    return Some(component[unchecked_range.start..unchecked_range.end].to_vec());
                }
                offset += consumed;
                buf = &component[offset..];
                if let Payload::End(_) = payload {
                    return None;
                }
            }
        }
    }
}

/// Extract the embedded core module's `name`-section function names from a component's bytes.
/// Returns `(module_name, [(func_index, name)])`; `None` if there is no `name` section.
fn name_section_of(component: &[u8]) -> Option<NameSection> {
    use wasmparser::{Parser, Payload};
    let core = core_module_of(component)?;
    let core = core.as_slice();
    // Now parse the core module for its `name` custom section.
    let mut module_name = None;
    let mut func_names = Vec::new();
    let mut found = false;
    for payload in Parser::new(0).parse_all(core) {
        if let Payload::CustomSection(reader) = payload.expect("parse core")
            && reader.name() == "name"
        {
            found = true;
            let name_reader = wasmparser::NameSectionReader::new(wasmparser::BinaryReader::new(
                reader.data(),
                reader.data_offset(),
            ));
            for subsection in name_reader {
                match subsection.expect("name subsection") {
                    wasmparser::Name::Module { name, .. } => module_name = Some(name.to_string()),
                    wasmparser::Name::Function(map) => {
                        for naming in map {
                            let naming = naming.expect("naming");
                            func_names.push((naming.index, naming.name.to_string()));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    found.then_some((module_name, func_names))
}

#[test]
fn a_plain_component_carries_no_name_section() {
    // Baseline: without a debug request, there is no `name` section — today's exact bytes.
    let src = "(module m (def (main) 42) (export main))";
    let plain = component_of(src, Target::Wasm);
    assert!(
        name_section_of(&plain).is_none(),
        "a plain component must carry no `name` section (byte-identical to today)"
    );
}

#[test]
fn a_debug_component_names_its_functions() {
    // A program with an exported `main` calling a RECURSIVE internal callee `countdown`. The callee
    // is recursive so it cannot be inlined away — it survives as a real emitted function, so the
    // `name` section names both `main` and `countdown` (a non-recursive helper would β-reduce into
    // its caller and no longer be a function to name). The module name is the first export, `main`.
    let src = "(module m \
                     (def (countdown (: n Int64)) (if (< n 1) 0 (countdown (- n 1)))) \
                     (def (main) (countdown 5)) \
                     (export main))";
    let debug = component_of(src, Target::WasmDebug);
    let (module_name, func_names) =
        name_section_of(&debug).expect("a debug component carries a `name` section");
    assert_eq!(module_name.as_deref(), Some("main"));
    // The source names appear in the function-name map (order/index is emission-determined; assert
    // membership so the test is robust to which index each lands at).
    let names: Vec<&str> = func_names.iter().map(|(_, n)| n.as_str()).collect();
    assert!(names.contains(&"main"), "names = {names:?}");
    assert!(names.contains(&"countdown"), "names = {names:?}");
}

#[test]
fn a_debug_component_only_adds_custom_sections() {
    // The debug component must differ from the plain one ONLY by added custom sections — i.e. the
    // debug bytes are strictly longer (the `name` section is appended, moving no executed byte).
    let src = "(module m (def (main) 42) (export main))";
    let plain = component_of(src, Target::Wasm);
    let debug = component_of(src, Target::WasmDebug);
    assert!(
        debug.len() > plain.len(),
        "the debug component must be larger (it carries an extra `name` section): plain {} vs debug {}",
        plain.len(),
        debug.len()
    );
    assert!(
        name_section_of(&debug).is_some() && name_section_of(&plain).is_none(),
        "only the debug component carries the `name` section"
    );
}

#[test]
fn strip_recovers_the_undecorated_component_byte_for_byte() {
    // The reproducibility anchor (§5): the debug component is the plain component PLUS inert,
    // strippable custom sections — so `wasm-tools strip --all` (a section REMOVER, never a
    // re-serializer) on BOTH yields byte-for-byte identical bare modules. Proves inertness,
    // strippability, and reproducibility in one cheap check. (Was `strip(debug) == plain`, but PLAIN
    // now legitimately carries the inert `cdz-result-type` run-wiring custom section (#5951) that
    // `strip --all` also removes — so compare stripped-vs-stripped, not stripped-vs-plain.) Skips if
    // `wasm-tools` is not installed.
    use std::io::Write;
    use std::process::Command;
    let src = "(module m (def (main) 42) (export main))";
    let plain = component_of(src, Target::Wasm);
    let debug = component_of(src, Target::WasmDebug);

    // Strip EVERY custom section (`--all`: `name`, `cdz-result-type`, and the debug sections) from a
    // component. `None` (⇒ the test skips) when `wasm-tools` is not on PATH.
    let strip_all = |bytes: &[u8], tag: &str| -> Option<Vec<u8>> {
        let dir = std::env::temp_dir();
        let in_path = dir.join(format!("cdz-{tag}-{}.wasm", std::process::id()));
        let out_path = dir.join(format!("cdz-{tag}-{}-stripped.wasm", std::process::id()));
        std::fs::File::create(&in_path)
            .and_then(|mut f| f.write_all(bytes))
            .expect("write component");
        let ran = match Command::new("wasm-tools")
            .args(["strip", "--all"])
            .arg(&in_path)
            .arg("-o")
            .arg(&out_path)
            .status()
        {
            Ok(s) => {
                assert!(s.success(), "wasm-tools strip failed");
                Some(std::fs::read(&out_path).expect("read stripped"))
            }
            Err(_) => None,
        };
        let _ = std::fs::remove_file(&in_path);
        let _ = std::fs::remove_file(&out_path);
        ran
    };

    let (Some(stripped_debug), Some(stripped_plain)) =
        (strip_all(&debug, "dbg"), strip_all(&plain, "plain"))
    else {
        eprintln!("wasm-tools not found on PATH; skipping strip round-trip");
        return;
    };
    assert_eq!(
        stripped_debug, stripped_plain,
        "stripping every custom section from the debug + plain components must recover identical bare bytes"
    );
}

#[test]
fn a_sidecar_emit_wasm_debug_request_drives_the_debug_component() {
    // Mode-E enablement end-to-end: an `EMIT_WASM_DEBUG` request in the sidecar list, with the
    // `spans` input supplied, drives a debug-carrying `component` — the debug directive is a
    // request, not a build flag.
    let src = "(module m (def (main) 42) (export main))";
    let (arenas, span_data) = parse_spanned(src);
    let out = compile(
        &[
            Artifact::new(Artifact::KIND_AST, "m", crate::codec::encode(&arenas)),
            Artifact::new(spans::KIND_SPANS, "m", spans::encode(&span_data)),
            Artifact::new(
                sidecar::KIND_SIDECAR,
                "drive",
                sidecar::encode(&[Request::Emit(Target::WasmDebug)]),
            ),
        ],
        &[],
    );
    assert!(!out.has_error(), "compile failed: {:?}", out.diagnostics);
    let component = out.artifact("component").expect("a component artifact");
    assert!(
        name_section_of(component).is_some(),
        "an EMIT_WASM_DEBUG request must produce a component carrying the `name` section"
    );
    // The round-trip through the sidecar codec preserves the debug request.
    let reqs = sidecar::decode(&sidecar::encode(&[Request::Emit(Target::WasmDebug)]));
    assert_eq!(reqs, Some(vec![Request::Emit(Target::WasmDebug)]));
}

#[test]
fn a_debug_emit_without_spans_declines() {
    // §9.4 — a debug artifact requested WITHOUT the `spans` input is a decline, not a silent
    // undecorated component. The debug `Emit` is the signal; `spans` is the data — required together.
    let src = "(module m (def (main) 42) (export main))";
    let out = compile(
        &[Artifact::new(
            Artifact::KIND_AST,
            "m",
            crate::codec::encode(&parse(src)),
        )],
        &[Target::WasmDebug],
    );
    assert!(out.has_error(), "must decline without a spans input");
    let d = out
        .diagnostics
        .iter()
        .find(|d| d.severity == crate::abi::Severity::Error)
        .unwrap();
    assert!(
        d.message.contains("`spans`"),
        "the decline must name the missing spans input: {}",
        d.message
    );
    // Crucially, NO component was produced — not an undecorated one.
    assert!(out.artifact("component").is_none());
}

#[test]
fn a_malformed_spans_artifact_declines() {
    // A present-but-malformed `spans` input is a decline (reject-don't-miscompile at the tool edge),
    // exactly like a malformed sidecar list.
    let src = "(module m (def (main) 42) (export main))";
    let out = compile(
        &[
            Artifact::new(Artifact::KIND_AST, "m", crate::codec::encode(&parse(src))),
            // A path length claiming 9 bytes but with none present — a truncated table.
            Artifact::new(spans::KIND_SPANS, "m", vec![0x09]),
        ],
        &[Target::WasmDebug],
    );
    assert!(out.has_error());
    let d = out
        .diagnostics
        .iter()
        .find(|d| d.severity == crate::abi::Severity::Error)
        .unwrap();
    assert!(
        d.message.contains("malformed `spans`"),
        "message: {}",
        d.message
    );
}

#[test]
fn a_plain_wasm_target_ignores_a_spans_input() {
    // A `spans` input present without a debug request changes nothing — the plain component is
    // byte-identical to one compiled with no spans input at all (spans are inert until a debug
    // target reads them).
    let src = "(module m (def (main) 42) (export main))";
    let (arenas, span_data) = parse_spanned(src);
    let with_spans = compile(
        &[
            Artifact::new(Artifact::KIND_AST, "m", crate::codec::encode(&arenas)),
            Artifact::new(spans::KIND_SPANS, "m", spans::encode(&span_data)),
        ],
        &[Target::Wasm],
    );
    let without = component_of(src, Target::Wasm);
    assert_eq!(
        with_spans.artifact("component").expect("component"),
        without.as_slice(),
        "a spans input must not change the plain (non-debug) component's bytes"
    );
}

// ── D1b: the offset→StructId line-table primitive (§2.1b) ──────────────────────────────────────

#[test]
fn code_ranges_partition_the_code_section_payload_and_carry_src() {
    // The D1b primitive: `code_ranges` gives each function's byte range within the code-section
    // PAYLOAD, paired with its source occurrence. Verify (a) the ranges are contiguous, start past
    // the function-count prefix, and cover exactly the payload; (b) each range's bytes ARE that
    // function's `code_entry` (byte-identical); (c) `src` round-trips. Two functions with distinct
    // source ids exercise the pairing.
    use crate::ast::StructId;
    use crate::backend::wasm::lir::Lir;
    use crate::backend::wasm::runtime_abi::OPS;
    use crate::backend::wasm::select::SelectedFunc;
    use crate::backend::wasm::serialize::{code_entry_bytes, code_ranges, core_module};
    use crate::layout::{ExportPlan, Layout};
    use crate::ty::Ty;

    let funcs = vec![
        SelectedFunc {
            params: vec![],
            ret: Ty::int64(),
            code: vec![Lir::ConstI64(7)],
            declared: vec![],
            src_body: Some(StructId(11)),
            locals: vec![],
            scopes: vec![],
            stmt_lines: vec![],
        },
        SelectedFunc {
            params: vec![],
            ret: Ty::int64(),
            code: vec![Lir::ConstI64(1000), Lir::ConstI64(2), Lir::I64Add],
            declared: vec![],
            src_body: Some(StructId(22)),
            locals: vec![],
            scopes: vec![],
            stmt_lines: vec![],
        },
    ];
    // One import so the layout mirrors the oracle shape; ranges are import-independent.
    let imports = [OPS.arr_alloc];
    let layout = Layout::new(
        vec![ExportPlan {
            name: "main".to_string(),
            def: 0,
            body: StructId(11),
            params: vec![],
            result: Ty::int64(),
        }],
        vec![0, 1],
        1,
    );

    let ranges = code_ranges(&funcs, &imports);
    assert_eq!(ranges.len(), 2);
    // (c) src round-trips, in emission order.
    assert_eq!(ranges[0].src, Some(StructId(11)));
    assert_eq!(ranges[1].src, Some(StructId(22)));

    // (a) contiguous + starts past the count prefix (a 2-function module: count = 1 byte).
    assert_eq!(
        ranges[0].code_start, 1,
        "starts past the 1-byte function count"
    );
    assert_eq!(
        ranges[0].code_end, ranges[1].code_start,
        "ranges are contiguous"
    );

    // (b) each range's bytes ARE that function's code entry.
    let entry0 = code_entry_bytes(&funcs[0], &imports);
    let entry1 = code_entry_bytes(&funcs[1], &imports);
    assert_eq!(
        (ranges[0].code_end - ranges[0].code_start) as usize,
        entry0.len()
    );
    assert_eq!(
        (ranges[1].code_end - ranges[1].code_start) as usize,
        entry1.len()
    );

    // Cross-check the payload total: count prefix + both entries == last range end.
    let count_prefix = 1u32; // uleb(2) is one byte
    assert_eq!(
        ranges[1].code_end,
        count_prefix + entry0.len() as u32 + entry1.len() as u32
    );

    // And the whole thing is consistent with a real `core_module`: the module must contain the two
    // entries' bytes consecutively at the payload region the ranges describe.
    let core = core_module(&funcs, &imports, &layout).expect("core module");
    let mut concat = entry0.clone();
    concat.extend_from_slice(&entry1);
    assert!(
        core.windows(concat.len()).any(|w| w == concat.as_slice()),
        "the emitted core module must contain both code entries consecutively"
    );
}

// ── D2: the .debug_* DWARF sections (§2.3) ─────────────────────────────────────────────────────

/// The names of the custom sections in a component's embedded core module (in order).
fn custom_section_names(component: &[u8]) -> Vec<String> {
    use wasmparser::{Parser, Payload};
    let core = core_module_of(component).expect("embedded core module");
    let mut names = Vec::new();
    for payload in Parser::new(0).parse_all(&core) {
        if let Payload::CustomSection(reader) = payload.expect("parse core") {
            names.push(reader.name().to_string());
        }
    }
    names
}

#[test]
fn a_debug_component_carries_the_dwarf_sections() {
    // A debug component's embedded core module carries the four `.debug_*` custom sections (plus the
    // D0 `name` section) — the sections a debugger reads to step through source.
    let src = "(module m \
                     (def (countdown (: n Int64)) (if (< n 1) 0 (countdown (- n 1)))) \
                     (def (main) (countdown 5)) \
                     (export main))";
    let debug = component_of(src, Target::WasmDebug);
    let names = custom_section_names(&debug);
    for want in [
        "name",
        ".debug_abbrev",
        ".debug_info",
        ".debug_str",
        ".debug_line",
    ] {
        assert!(
            names.iter().any(|n| n == want),
            "missing custom section `{want}`; found {names:?}"
        );
    }
    // A plain component has none of them.
    let plain = component_of(src, Target::Wasm);
    assert!(
        custom_section_names(&plain)
            .iter()
            .all(|n| !n.starts_with(".debug")),
        "a plain component must carry no `.debug_*` sections"
    );
}

#[test]
fn the_dwarf_parses_under_llvm_dwarfdump() {
    // The correctness ORACLE (§6): the hand-rolled DWARF must parse cleanly under `llvm-dwarfdump`.
    // We extract the embedded core module (dwarfdump rejects a component's version number) and dump
    // it; the output must name our compile unit + a subprogram, and report NO parse error. Skips if
    // `llvm-dwarfdump` is not installed.
    use std::io::Write;
    use std::process::Command;
    let src = "(module m \
                     (def (countdown (: n Int64)) (if (< n 1) 0 (countdown (- n 1)))) \
                     (def (main) (countdown 5)) \
                     (export main))";
    let debug = component_of(src, Target::WasmDebug);
    let core = core_module_of(&debug).expect("embedded core module");

    let dir = std::env::temp_dir();
    let path = dir.join(format!("cdz-dwarf-{}.wasm", std::process::id()));
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(&core))
        .expect("write core module");

    let output = match Command::new("llvm-dwarfdump")
        .arg("--all")
        .arg(&path)
        .output()
    {
        Ok(o) => o,
        Err(_) => {
            eprintln!("llvm-dwarfdump not found on PATH; skipping DWARF-validity check");
            let _ = std::fs::remove_file(&path);
            return;
        }
    };
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "llvm-dwarfdump failed: {stderr}\n{stdout}"
    );
    // The dump must NOT report a parse error, and must name our compile unit + a subprogram.
    assert!(
        !stdout.to_lowercase().contains("error:") && !stderr.to_lowercase().contains("error:"),
        "llvm-dwarfdump reported an error:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("DW_TAG_compile_unit"),
        "no compile unit in the dump:\n{stdout}"
    );
    assert!(
        stdout.contains("DW_TAG_subprogram"),
        "no subprogram in the dump:\n{stdout}"
    );
    // The producer + a function name must round-trip through .debug_str.
    assert!(
        stdout.contains("cadenza-rcdzc"),
        "producer string missing:\n{stdout}"
    );
    assert!(
        stdout.contains("countdown") || stdout.contains("main"),
        "no source function name in the dump:\n{stdout}"
    );
    // The compile unit declares a source language (DW_LANG_C) — a CU with no language reads as
    // "<not loaded>" in a real debugger, which then declines to format scalar values.
    assert!(
        stdout.contains("DW_AT_language") && stdout.contains("DW_LANG_C"),
        "the CU must declare DW_LANG_C:\n{stdout}"
    );
}

// ── Mode S: the detached `dwarf` sidecar artifact (§9.2) ───────────────────────────────────────

/// Compile `src` with the given request list + the `spans` input, returning the full output.
fn compile_debug(src: &str, requests: &[Request]) -> crate::abi::CompileOutput {
    let (arenas, span_data) = parse_spanned(src);
    compile(
        &[
            Artifact::new(Artifact::KIND_AST, "m", crate::codec::encode(&arenas)),
            Artifact::new(spans::KIND_SPANS, "m", spans::encode(&span_data)),
            Artifact::new(sidecar::KIND_SIDECAR, "d", sidecar::encode(requests)),
        ],
        &[],
    )
}

#[test]
fn an_emit_dwarf_request_produces_a_dwarf_artifact_not_a_component() {
    // A lone `Emit(Dwarf)` yields a `dwarf` artifact and NO component — the detached sidecar mode.
    let src = "(module m (def (main) 42) (export main))";
    let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
    assert!(!out.has_error(), "compile failed: {:?}", out.diagnostics);
    assert!(
        out.artifact("dwarf").is_some(),
        "an Emit(Dwarf) request must produce a `dwarf` artifact"
    );
    assert!(
        out.artifact("component").is_none(),
        "a lone Emit(Dwarf) must NOT produce a component"
    );
}

#[test]
fn wasm_and_dwarf_compose_into_two_artifacts() {
    // Requesting both a plain component AND a detached dwarf yields both — the "lean component + its
    // detached DWARF" shape.
    let src = "(module m (def (main) 42) (export main))";
    let out = compile_debug(
        src,
        &[Request::Emit(Target::Wasm), Request::Emit(Target::Dwarf)],
    );
    assert!(!out.has_error(), "compile failed: {:?}", out.diagnostics);
    assert!(
        out.artifact("component").is_some(),
        "the runnable component"
    );
    assert!(out.artifact("dwarf").is_some(), "the detached DWARF");
}

#[test]
fn the_dwarf_sidecar_parses_under_llvm_dwarfdump() {
    // The sidecar module is ALREADY a bare core module (no component wrapper), so llvm-dwarfdump
    // reads it directly. It must show the same compile unit + subprograms as the embedded form.
    use std::io::Write;
    use std::process::Command;
    let src = "(module m \
                     (def (countdown (: n Int64)) (if (< n 1) 0 (countdown (- n 1)))) \
                     (def (main) (countdown 5)) \
                     (export main))";
    let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
    let dwarf = out.artifact("dwarf").expect("a dwarf artifact").to_vec();

    let dir = std::env::temp_dir();
    let path = dir.join(format!("cdz-sidecar-{}.wasm", std::process::id()));
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(&dwarf))
        .expect("write dwarf sidecar");
    let output = match Command::new("llvm-dwarfdump")
        .arg("--all")
        .arg(&path)
        .output()
    {
        Ok(o) => o,
        Err(_) => {
            eprintln!("llvm-dwarfdump not found; skipping");
            let _ = std::fs::remove_file(&path);
            return;
        }
    };
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "dwarfdump failed: {stderr}\n{stdout}"
    );
    assert!(
        !stdout.to_lowercase().contains("error:") && !stderr.to_lowercase().contains("error:"),
        "dwarfdump reported an error:\n{stdout}\n{stderr}"
    );
    assert!(stdout.contains("DW_TAG_compile_unit"), "no CU:\n{stdout}");
    assert!(
        stdout.contains("DW_TAG_subprogram"),
        "no subprogram:\n{stdout}"
    );
    assert!(
        stdout.contains("countdown") || stdout.contains("main"),
        "no fn name:\n{stdout}"
    );
}

#[test]
fn the_dwarf_sidecar_loads_in_lldb() {
    // A SECOND, INDEPENDENT oracle beyond llvm-dwarfdump (design §6 "wall #2" — does a REAL
    // debugger, not just the lenient dump parser, consume our hand-rolled DWARF?). lldb loads the
    // bare sidecar core module, recognizes it as a `wasm32` target, and parses the compile unit —
    // reporting its source language (which requires the `DW_AT_language` we emit; a CU without it
    // reads as "<not loaded>"). Skips if lldb is absent.
    use std::io::Write;
    use std::process::Command;
    let src = "(module m \
                     (def (countdown (: n Int64)) (if (< n 1) 0 (countdown (- n 1)))) \
                     (def (main) (countdown 5)) \
                     (export main))";
    let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
    let dwarf = out.artifact("dwarf").expect("a dwarf artifact").to_vec();

    let dir = std::env::temp_dir();
    let path = dir.join(format!("cdz-lldb-{}.wasm", std::process::id()));
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(&dwarf))
        .expect("write dwarf sidecar");
    // Dump the compile unit via lldb's Python API — `str(CompileUnit)` includes `language = "…"` and
    // the source file, which is exactly what a debugger reads from our CU DIE + line-program header.
    let output = match Command::new("lldb")
        .arg("--batch")
        .arg("-o")
        .arg(
            "script m=lldb.target.module_iter().__next__(); \
                 print('CU=' + str(m.compile_unit_iter().__next__()))",
        )
        .arg(path.to_str().unwrap())
        .output()
    {
        Ok(o) => o,
        Err(_) => {
            eprintln!("lldb not found; skipping the second-oracle check");
            let _ = std::fs::remove_file(&path);
            return;
        }
    };
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // lldb recognizes the module as a wasm32 target (it accepts our bare core module).
    assert!(
        stdout.contains("wasm32"),
        "lldb did not recognize a wasm32 target:\n{stdout}\n{stderr}"
    );
    // It parsed a compile unit and read our declared source language (proving DW_AT_language landed —
    // an absent language would print `language = "<not loaded>"`).
    assert!(
        stdout.contains("CU=") && stdout.contains("language = \"c\""),
        "lldb did not read the CU's source language:\n{stdout}\n{stderr}"
    );
}

#[test]
fn the_sidecar_code_offsets_match_the_embedded_ones() {
    // The load-bearing invariant: a detached DWARF's code addresses must reference the RUNNABLE
    // component's code section identically to the embedded form — otherwise a debugger would map to
    // the wrong instruction. Compare the subprogram low_pc/high_pc reported by dwarfdump for the
    // embedded component vs the sidecar; they must be identical. Skips if llvm-dwarfdump is absent.
    use std::io::Write;
    use std::process::Command;
    let src = "(module m \
                     (def (countdown (: n Int64)) (if (< n 1) 0 (countdown (- n 1)))) \
                     (def (main) (countdown 5)) \
                     (export main))";
    // Embedded: extract the core module from the WasmDebug component.
    let embedded_component = component_of(src, Target::WasmDebug);
    let embedded_core = core_module_of(&embedded_component).expect("core module");
    // Sidecar: the standalone dwarf module.
    let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
    let sidecar_mod = out.artifact("dwarf").expect("dwarf").to_vec();

    // Dump each and pull the ordered list of (low_pc, high_pc) from the subprogram DIEs.
    let pcs = |bytes: &[u8], tag: &str| -> Option<Vec<(String, String)>> {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("cdz-cmp-{}-{tag}.wasm", std::process::id()));
        std::fs::File::create(&path)
            .and_then(|mut f| f.write_all(bytes))
            .ok()?;
        let o = Command::new("llvm-dwarfdump")
            .arg("--debug-info")
            .arg(&path)
            .output()
            .ok()?;
        let _ = std::fs::remove_file(&path);
        let s = String::from_utf8_lossy(&o.stdout).to_string();
        let mut lows: Vec<String> = Vec::new();
        let mut highs: Vec<String> = Vec::new();
        for line in s.lines() {
            let t = line.trim();
            if let Some(v) = t.strip_prefix("DW_AT_low_pc") {
                lows.push(v.trim().to_string());
            } else if let Some(v) = t.strip_prefix("DW_AT_high_pc") {
                highs.push(v.trim().to_string());
            }
        }
        Some(lows.into_iter().zip(highs).collect())
    };

    let (Some(emb), Some(side)) = (pcs(&embedded_core, "emb"), pcs(&sidecar_mod, "side")) else {
        eprintln!("llvm-dwarfdump not found; skipping offset-match check");
        return;
    };
    assert!(!emb.is_empty(), "the embedded DWARF had no pc ranges");
    assert_eq!(
        emb, side,
        "the sidecar's code offsets must match the embedded component's exactly"
    );
}

// ── D3: scalar variable inspection (§2.4) ──────────────────────────────────────────────────────

#[test]
fn scalar_params_get_formal_parameter_dies_with_locations() {
    // D3: an exported function's scalar parameters emit `DW_TAG_formal_parameter` DIEs, each with a
    // `DW_AT_type` (→ a base type) and a `DW_AT_location` naming the wasm local slot — so a debugger
    // can `print` the argument. Verified via llvm-dwarfdump on the detached sidecar (a bare module).
    use std::io::Write;
    use std::process::Command;
    // Two Int64 params so the fn has real scalar locals at slots 0 and 1.
    let src = "(module m (def (add (: a Int64) (: b Int64)) (+ a b)) (export add))";
    let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
    assert!(!out.has_error(), "compile failed: {:?}", out.diagnostics);
    let dwarf = out.artifact("dwarf").expect("dwarf").to_vec();

    let dir = std::env::temp_dir();
    let path = dir.join(format!("cdz-d3-{}.wasm", std::process::id()));
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(&dwarf))
        .expect("write");
    let output = match Command::new("llvm-dwarfdump")
        .arg("--debug-info")
        .arg(&path)
        .output()
    {
        Ok(o) => o,
        Err(_) => {
            eprintln!("llvm-dwarfdump not found; skipping");
            let _ = std::fs::remove_file(&path);
            return;
        }
    };
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "dwarfdump failed:\n{stdout}");
    assert!(
        !stdout.to_lowercase().contains("error:"),
        "dwarfdump error:\n{stdout}"
    );
    // A base type for Int64.
    assert!(
        stdout.contains("DW_TAG_base_type") && stdout.contains("\"i64\""),
        "missing the i64 base type:\n{stdout}"
    );
    // Both params, as formal parameters, with a type ref and a wasm-local location.
    assert!(
        stdout.contains("DW_TAG_formal_parameter"),
        "no formal_parameter DIE:\n{stdout}"
    );
    assert!(
        stdout.contains("\"a\"") && stdout.contains("\"b\""),
        "param names:\n{stdout}"
    );
    assert!(
        stdout.contains("DW_OP_WASM_location"),
        "no wasm-local location:\n{stdout}"
    );
    // The two params sit at consecutive local slots 0 and 1.
    assert!(
        stdout.contains("DW_OP_WASM_location 0x0 0x0")
            && stdout.contains("DW_OP_WASM_location 0x0 0x1"),
        "params must be at local slots 0 and 1:\n{stdout}"
    );
}

#[test]
fn a_nullary_function_has_no_formal_parameters() {
    // A function with no params emits a childless subprogram — no formal_parameter DIEs.
    use std::io::Write;
    use std::process::Command;
    let src = "(module m (def (main) 42) (export main))";
    let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
    let dwarf = out.artifact("dwarf").expect("dwarf").to_vec();
    let dir = std::env::temp_dir();
    let path = dir.join(format!("cdz-d3n-{}.wasm", std::process::id()));
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(&dwarf))
        .expect("write");
    let output = match Command::new("llvm-dwarfdump")
        .arg("--debug-info")
        .arg(&path)
        .output()
    {
        Ok(o) => o,
        Err(_) => {
            let _ = std::fs::remove_file(&path);
            return;
        }
    };
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "dwarfdump failed:\n{stdout}");
    assert!(
        !stdout.contains("DW_TAG_formal_parameter"),
        "a nullary function must have no formal parameters:\n{stdout}"
    );
}

#[test]
fn float_params_get_formal_parameter_dies_with_a_float_base_type() {
    // D3 for FLOATS: a `Float32`/`Float64` parameter is a scalar the runtime holds in a wasm
    // `f32`/`f64` local, so it earns a `DW_TAG_formal_parameter` with a `DW_ATE_float` base type +
    // a `DW_OP_WASM_location` local slot — a debugger can `print` a float argument, exactly as for
    // an integer. (Before this, `base_type_of` returned `None` for `Ty::Float`, so a float param got
    // NO DIE.) Both widths are covered: `Float64` → `f64` (8 bytes), `Float32` → `f32` (4 bytes).
    use std::io::Write;
    use std::process::Command;
    let src = "(module m \
                     (def (scale (: x Float64) (: k Float32)) (* x (Float64.of-int 1))) \
                     (export scale))";
    let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
    assert!(!out.has_error(), "compile failed: {:?}", out.diagnostics);
    let dwarf = out.artifact("dwarf").expect("dwarf").to_vec();
    let dir = std::env::temp_dir();
    let path = dir.join(format!("cdz-d3f-{}.wasm", std::process::id()));
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(&dwarf))
        .expect("write");
    let output = match Command::new("llvm-dwarfdump")
        .arg("--debug-info")
        .arg(&path)
        .output()
    {
        Ok(o) => o,
        Err(_) => {
            eprintln!("llvm-dwarfdump not found; skipping");
            let _ = std::fs::remove_file(&path);
            return;
        }
    };
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "dwarfdump failed:\n{stdout}");
    assert!(
        !stdout.to_lowercase().contains("error:"),
        "dwarfdump reported an error:\n{stdout}"
    );
    // Both scalar float widths get a base type with the IEEE-float encoding.
    assert!(
        stdout.contains("DW_ATE_float"),
        "a float param must reference a DW_ATE_float base type:\n{stdout}"
    );
    assert!(
        stdout.contains("\"f64\"") && stdout.contains("\"f32\""),
        "both f64 and f32 base types must be present:\n{stdout}"
    );
    // The params are formal parameters with names + wasm-local locations.
    assert!(
        stdout.contains("DW_TAG_formal_parameter")
            && stdout.contains("\"x\"")
            && stdout.contains("\"k\""),
        "the float params x and k must have formal_parameter DIEs:\n{stdout}"
    );
    assert!(
        stdout.contains("DW_OP_WASM_location 0x0 0x0")
            && stdout.contains("DW_OP_WASM_location 0x0 0x1"),
        "the float params must sit at local slots 0 and 1:\n{stdout}"
    );
}

#[test]
fn a_nominal_newtype_scalar_param_gets_a_formal_parameter_die() {
    // D3 for a NOMINAL NEWTYPE over a scalar: `(type UserId (Mk Int64))` is erased to a runtime i64,
    // so a `UserId` parameter is a scalar the runtime holds in a wasm local — it must earn a
    // `DW_TAG_formal_parameter` with the UNDERLYING scalar's base type (`i64`, the value the runtime
    // actually holds), exactly like a bare `Int64`. (Before this, `base_type_of`/`select`'s scalar
    // guards didn't peel the nominal tag, so a nominal-scalar param got NO DIE.)
    use std::io::Write;
    use std::process::Command;
    let src = "(module m \
                     (type UserId (Mk Int64)) \
                     (def (idof (: u UserId)) (match u ((Mk n) n))) \
                     (export idof))";
    let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
    assert!(!out.has_error(), "compile failed: {:?}", out.diagnostics);
    let dwarf = out.artifact("dwarf").expect("dwarf").to_vec();
    let dir = std::env::temp_dir();
    let path = dir.join(format!("cdz-d3nom-{}.wasm", std::process::id()));
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(&dwarf))
        .expect("write");
    let output = match Command::new("llvm-dwarfdump")
        .arg("--debug-info")
        .arg(&path)
        .output()
    {
        Ok(o) => o,
        Err(_) => {
            eprintln!("llvm-dwarfdump not found; skipping");
            let _ = std::fs::remove_file(&path);
            return;
        }
    };
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "dwarfdump failed:\n{stdout}");
    assert!(
        !stdout.to_lowercase().contains("error:"),
        "dwarfdump reported an error:\n{stdout}"
    );
    // The nominal param `u` is described via its erased i64 base type + a wasm-local location.
    assert!(
        stdout.contains("DW_TAG_formal_parameter") && stdout.contains("\"u\""),
        "the nominal-scalar param u must have a formal_parameter DIE:\n{stdout}"
    );
    assert!(
        stdout.contains("\"i64\"") && stdout.contains("DW_ATE_signed"),
        "the nominal param must reference its underlying i64 base type:\n{stdout}"
    );
    assert!(
        stdout.contains("DW_OP_WASM_location 0x0 0x0"),
        "the nominal param must sit at local slot 0:\n{stdout}"
    );
}

#[test]
fn wasm_plus_dwarf_links_the_component_to_the_sidecar() {
    // When a run emits BOTH a lean component and a detached DWARF sidecar, the component carries an
    // `external_debug_info` custom section naming the sidecar file — so a debugger auto-loads it.
    let src = "(module m (def (main) 42) (export main))";
    let out = compile_debug(
        src,
        &[Request::Emit(Target::Wasm), Request::Emit(Target::Dwarf)],
    );
    assert!(!out.has_error(), "compile failed: {:?}", out.diagnostics);
    let comp = out.artifact("component").expect("component").to_vec();
    let names = custom_section_names(&comp);
    assert!(
        names.iter().any(|n| n == "external_debug_info"),
        "the lean component must point at the sidecar; found {names:?}"
    );
    // The lean component embeds NO DWARF itself (that lives in the sidecar) — only the pointer.
    assert!(
        names.iter().all(|n| !n.starts_with(".debug")),
        "a lean component must not embed DWARF (it points at the sidecar): {names:?}"
    );
    // The pointer's payload names the on-disk sidecar file (`main.dwarf` — the program name).
    assert!(
        comp.windows(b"main.dwarf".len())
            .any(|w| w == b"main.dwarf"),
        "the external_debug_info payload must name the sidecar file"
    );
}

#[test]
fn a_lone_wasm_carries_no_external_debug_info() {
    // Without a paired `Dwarf` target, a plain component has no `external_debug_info` pointer — it
    // stays byte-identical to today's undecorated output.
    let src = "(module m (def (main) 42) (export main))";
    let plain = component_of(src, Target::Wasm);
    assert!(
        !custom_section_names(&plain)
            .iter()
            .any(|n| n == "external_debug_info"),
        "a lone Wasm target must carry no external_debug_info"
    );
}

#[test]
fn a_compound_returning_program_carries_dwarf() {
    // A program returning a RUNTIME compound crosses via the resource-escape path (a different core
    // than the multi-export path). Its user function bodies still lead the escape core's code
    // section, so the `.debug_*` sections attribute correctly — a compound-returning program is
    // debuggable too. `f` recurses (not constant-foldable), so `main` builds the tuple on the heap.
    let src = "(module m \
                     (def (f n) (if (= n 0) (tuple n 7) (f (- n 1)))) \
                     (def (main) (f 3)) \
                     (export main))";
    let debug = component_of(src, Target::WasmDebug);
    // The resource envelope embeds MULTIPLE core modules (a dtor + the main walker core); the DWARF
    // rides in the main one, which is not necessarily first — so scan the raw component bytes for
    // the section names (those bytes are exactly what ships to the debugger) rather than only the
    // first embedded core. A plain (non-debug) build of the same program carries none of them.
    let has = |needle: &[u8]| debug.windows(needle.len()).any(|w| w == needle);
    for want in [b".debug_info".as_slice(), b".debug_line".as_slice()] {
        assert!(
            has(want),
            "a compound-returning program must carry {:?}",
            std::str::from_utf8(want).unwrap()
        );
    }
    // The source function names ride in — the resource core's user bodies get subprogram DIEs.
    assert!(
        has(b"main"),
        "the escape component's DWARF must name the source functions"
    );
    // The wasm `name` section rides in too (uniformly with the ordinary path now) — so a plain
    // profiler/trace shows `f`/`main`, not `func[N]`. Its section-name string is the length-prefixed
    // `\x04name` (the custom-section name), distinct from the incidental substring "name".
    assert!(
        has(b"\x04name"),
        "the escape component must carry the wasm `name` section"
    );
    // The plain build embeds no DWARF (the sections are debug-only).
    let plain = component_of(src, Target::Wasm);
    assert!(
        !plain
            .windows(b".debug_info".len())
            .any(|w| w == b".debug_info"),
        "a plain compound-returning component must carry no DWARF"
    );
}

#[test]
fn a_compound_returning_export_gets_a_dwarf_sidecar_matching_the_embedded_dwarf() {
    // Mode S for the RESOURCE-ESCAPE path: a compound-returning export used to DECLINE a detached
    // `dwarf` sidecar ("not yet supported"). It now emits one, and — the load-bearing invariant —
    // its `.debug_*` sections are BYTE-IDENTICAL to the ones the embedded (Mode E) component carries,
    // so a debugger maps to the same instructions with either artifact. Covers the flat-tuple, sum,
    // and runtime-Bytes escape cores (each a distinct resource core layout).
    for src in [
        // runtime tuple (recursive build → real user bodies, resource-escape core)
        "(module m \
               (def (f (: n Int64)) (if (= n 0) (tuple n 7) (f (- n 1)))) \
               (def (main) (f 3)) \
               (export main))",
        // runtime sum (Some payload, disc-switch walker)
        "(module m \
               (def (pick (: n Int64)) (if (= n 0) (Some 5) (pick (- n 1)))) \
               (def (main) (pick 3)) \
               (export main))",
        // runtime Bytes (looping walker)
        "(module m \
               (def (grow (: n Int64) (: acc Bytes)) \
                 (if (= n 0) acc (grow (- n 1) (Bytes.concat acc (Bytes.of (list 65)))))) \
               (def (main) (grow 3 (Bytes.of (list)))) \
               (export main))",
    ] {
        // The sidecar (Mode S).
        let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
        assert!(
            !out.has_error(),
            "a compound-returning export must now emit a dwarf sidecar, not decline: {:?}",
            out.diagnostics
        );
        let sidecar = out.artifact("dwarf").expect("a dwarf artifact").to_vec();

        // The embedded (Mode E) component — find the core module that carries the DWARF (a resource
        // envelope embeds several; the dtor core carries none).
        let embedded = component_of(src, Target::WasmDebug);
        let debug_core = core_modules_of(&embedded)
            .into_iter()
            .find(|m| custom_section_of(m, ".debug_info").is_some())
            .expect("an embedded core module carrying DWARF");

        for sect in [".debug_info", ".debug_line", ".debug_abbrev", ".debug_str"] {
            let s = custom_section_of(&sidecar, sect);
            let e = custom_section_of(&debug_core, sect);
            assert_eq!(
                s, e,
                "the sidecar's {sect} must be byte-identical to the embedded component's \
                     (same code offsets); src = {src}"
            );
            assert!(s.is_some(), "the sidecar must carry {sect}");
        }
    }
}

#[test]
fn a_constant_compound_export_gets_a_valid_empty_dwarf_sidecar() {
    // A FULLY-CONSTANT compound bakes its bytes into a resource core with NO user function to
    // attribute — the embedded path emits no `.debug_*` for it. The sidecar must therefore be a
    // VALID module with an empty compile unit (no subprograms), NOT a decline and not malformed.
    let src = "(module m (def (pair) (tuple 42 7)) (export pair))";
    let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
    assert!(
        !out.has_error(),
        "a constant compound must emit an (empty) sidecar, not decline: {:?}",
        out.diagnostics
    );
    let sidecar = out.artifact("dwarf").expect("a dwarf artifact").to_vec();
    // A bare core module carrying a `.debug_info` (the CU) but the CU has no subprogram DIEs — there
    // were no user bodies. We assert the section EXISTS (the standalone module is well-formed) and
    // that the embedded component carried no `.debug_info` (nothing to attribute), the dual property.
    assert!(
        custom_section_of(&sidecar, ".debug_info").is_some(),
        "even an empty CU sidecar carries a .debug_info section"
    );
    let embedded = component_of(src, Target::WasmDebug);
    let has_embedded_dwarf = core_modules_of(&embedded)
        .iter()
        .any(|m| custom_section_of(m, ".debug_info").is_some());
    assert!(
        !has_embedded_dwarf,
        "a constant compound's embedded component has no user body → no embedded DWARF"
    );
}

#[test]
fn a_multi_line_body_gets_a_line_row_per_line() {
    // Per-statement/expression granularity: an `if` whose condition, then-branch, and else-branch
    // sit on DISTINCT source lines produces a `.debug_line` row per line (not one function-entry
    // row) — so a debugger steps line-by-line. Rows are at ascending code offsets. Skips if
    // llvm-dwarfdump is absent.
    use std::io::Write;
    use std::process::Command;
    let src = "(module m\n  (def (f (: a Int64))\n    (if (< a 0)\n      (- a 1)\n      (+ a 1)))\n  (export f))";
    let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
    let dwarf = out.artifact("dwarf").expect("a dwarf artifact").to_vec();
    let dir = std::env::temp_dir();
    let path = dir.join(format!("cdz-lines-{}.wasm", std::process::id()));
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(&dwarf))
        .expect("write");
    let output = match Command::new("llvm-dwarfdump")
        .arg("--debug-line")
        .arg(&path)
        .output()
    {
        Ok(o) => o,
        Err(_) => {
            eprintln!("llvm-dwarfdump not found; skipping");
            let _ = std::fs::remove_file(&path);
            return;
        }
    };
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "dwarfdump failed:\n{stdout}");
    // Parse the line-table rows: lines beginning with `0x…` have columns [Address, Line, …]. Collect
    // the distinct source lines (col 2) across the rows.
    let mut lines: Vec<u32> = stdout
        .lines()
        .filter_map(|l| {
            let l = l.trim_start();
            if !l.starts_with("0x") {
                return None;
            }
            l.split_whitespace().nth(1)?.parse().ok()
        })
        .collect();
    lines.sort_unstable();
    lines.dedup();
    // The `if` condition (line 3), then-branch (line 4), and else-branch (line 5) each get a row —
    // at least 3 distinct source lines (function granularity would give only 1).
    assert!(
        lines.len() >= 3,
        "expected a row per source line (≥3 distinct), got {lines:?}\n{stdout}"
    );
    assert!(
        lines.contains(&3) && lines.contains(&4) && lines.contains(&5),
        "expected rows for lines 3/4/5, got {lines:?}\n{stdout}"
    );
}

#[test]
fn a_kept_scalar_let_binding_gets_a_variable_die() {
    // D3 locals: a `let` binding whose runtime value is used more than once is KEPT as a named slot
    // (`Core::Let`); a scalar one earns a `DW_TAG_variable` DIE with a type and a wasm-local location,
    // so a debugger can `print` the local. The binder key is the initializer occurrence, so the name
    // is recovered from its `(name init)` pair (`db.let_binding_name`) — this test guards that path.
    use std::io::Write;
    use std::process::Command;
    // `x` is used twice, so it survives A-normalization as a kept `Core::Let` binding.
    let src = "(module m (def (f (: a Int64)) (let ((x (+ a 1))) (+ x x))) (export f))";
    let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
    assert!(!out.has_error(), "compile failed: {:?}", out.diagnostics);
    let dwarf = out.artifact("dwarf").expect("dwarf").to_vec();
    let dir = std::env::temp_dir();
    let path = dir.join(format!("cdz-letloc-{}.wasm", std::process::id()));
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(&dwarf))
        .expect("write");
    let output = match Command::new("llvm-dwarfdump")
        .arg("--debug-info")
        .arg(&path)
        .output()
    {
        Ok(o) => o,
        Err(_) => {
            eprintln!("llvm-dwarfdump not found; skipping");
            let _ = std::fs::remove_file(&path);
            return;
        }
    };
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "dwarfdump failed:\n{stdout}");
    assert!(
        !stdout.to_lowercase().contains("error:"),
        "dwarfdump error:\n{stdout}"
    );
    // The kept binding `x` is a variable (not a formal parameter), with a wasm-local location.
    assert!(
        stdout.contains("DW_TAG_variable"),
        "no variable DIE for the kept let binding:\n{stdout}"
    );
    assert!(
        stdout.contains("\"x\""),
        "the local `x` is unnamed:\n{stdout}"
    );
    // `x` sits ABOVE the single param slot 0 — a variable at a non-zero local slot.
    assert!(
        stdout.contains("DW_OP_WASM_location 0x0 0x1"),
        "the kept local must live at local slot 1:\n{stdout}"
    );
}

#[test]
fn a_scalar_match_binder_gets_a_lexical_block_variable() {
    // D3 match binders: a bare-binder arm over a COMPUTED scrutinee (`(match (+ a b) (x (* x x)))`)
    // binds the scrutinee's spill slot — described by a `DW_TAG_variable` inside a
    // `DW_TAG_lexical_block` whose PC range fences it to the match (its slot is a reused scratch slot,
    // so a function-scoped variable would misreport `x` for the rest of the function). Verified via
    // llvm-dwarfdump on the sidecar. Skips if the tool is absent.
    use std::io::Write;
    use std::process::Command;
    let src = "(module m (def (f (: a Int64) (: b Int64)) (match (+ a b) (x (* x x)))) (export f))";
    let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
    assert!(!out.has_error(), "compile failed: {:?}", out.diagnostics);
    let dwarf = out.artifact("dwarf").expect("dwarf").to_vec();
    let dir = std::env::temp_dir();
    let path = dir.join(format!("cdz-mb-{}.wasm", std::process::id()));
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(&dwarf))
        .expect("write");
    let output = match Command::new("llvm-dwarfdump")
        .arg("--debug-info")
        .arg(&path)
        .output()
    {
        Ok(o) => o,
        Err(_) => {
            eprintln!("llvm-dwarfdump not found; skipping");
            let _ = std::fs::remove_file(&path);
            return;
        }
    };
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "dwarfdump failed:\n{stdout}");
    assert!(
        !stdout.to_lowercase().contains("error:"),
        "dwarfdump error:\n{stdout}"
    );
    // A lexical block scopes the binder — NOT a function-scoped variable.
    assert!(
        stdout.contains("DW_TAG_lexical_block"),
        "no lexical block for the match binder:\n{stdout}"
    );
    // The binder `x` is a variable inside it, with a wasm-local location (the scrutinee spill slot).
    assert!(
        stdout.contains("DW_TAG_variable") && stdout.contains("\"x\""),
        "the match binder `x` is missing:\n{stdout}"
    );
    // The lexical block's low_pc must be ABOVE the subprogram's (the block covers only the match's
    // arm code, not the whole function) — i.e. two distinct low_pc values appear.
    let low_pcs: Vec<&str> = stdout
        .lines()
        .filter_map(|l| l.trim().strip_prefix("DW_AT_low_pc"))
        .collect();
    assert!(
        low_pcs.len() >= 3,
        "expected a low_pc for the CU, subprogram, AND lexical block:\n{stdout}"
    );
}

#[test]
fn a_scalar_match_binder_dwarf_verifies_under_llvm_dwarfdump() {
    // The lexical-block DIE tree (subprogram → lexical_block → variable, with the nested NULL
    // terminators) must be WELL-FORMED — `llvm-dwarfdump --verify` reports no errors. This guards the
    // delicate DIE-tree/offset math of the match-binder scope. Skips if the tool is absent.
    use std::io::Write;
    use std::process::Command;
    let src = "(module m (def (f (: a Int64) (: b Int64)) (match (- a b) (y (+ y 1)))) (export f))";
    let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
    let dwarf = out.artifact("dwarf").expect("dwarf").to_vec();
    let dir = std::env::temp_dir();
    let path = dir.join(format!("cdz-mbv-{}.wasm", std::process::id()));
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(&dwarf))
        .expect("write");
    let output = match Command::new("llvm-dwarfdump")
        .arg("--verify")
        .arg(&path)
        .output()
    {
        Ok(o) => o,
        Err(_) => {
            eprintln!("llvm-dwarfdump not found; skipping");
            let _ = std::fs::remove_file(&path);
            return;
        }
    };
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("No errors"),
        "the match-binder DWARF failed --verify:\n{stdout}"
    );
}

#[test]
fn single_line_constructs_get_distinct_columns() {
    // COLUMN granularity: an `if` whose condition, then-branch, and else-branch all sit on ONE
    // source line still produces DISTINCT line-table rows — each carrying the sub-expression's
    // COLUMN — so a debugger highlights the exact construct within the line (the payoff for
    // s-expression Cadenza, where a whole `(if c a b)` is one line). Line-only granularity would
    // collapse these to a single row. Skips if llvm-dwarfdump is absent.
    use std::io::Write;
    use std::process::Command;
    // Everything on line 2 (line 1 is the module header) — distinct columns, same line.
    let src = "(module m\n(def (f (: a Int64)) (if (< a 0) (- a 1) (+ a 1)))\n(export f))";
    let out = compile_debug(src, &[Request::Emit(Target::Dwarf)]);
    let dwarf = out.artifact("dwarf").expect("a dwarf artifact").to_vec();
    let dir = std::env::temp_dir();
    let path = dir.join(format!("cdz-cols-{}.wasm", std::process::id()));
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(&dwarf))
        .expect("write");
    let output = match Command::new("llvm-dwarfdump")
        .arg("--debug-line")
        .arg(&path)
        .output()
    {
        Ok(o) => o,
        Err(_) => {
            eprintln!("llvm-dwarfdump not found; skipping");
            let _ = std::fs::remove_file(&path);
            return;
        }
    };
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "dwarfdump failed:\n{stdout}");
    // Line-table rows begin with `0x…`; columns are [Address, Line, Column, …]. Collect the
    // (line, column) of the rows on line 2 (the single source line the code lives on).
    let cols: Vec<u32> = stdout
        .lines()
        .filter_map(|l| {
            let l = l.trim_start();
            if !l.starts_with("0x") {
                return None;
            }
            let mut it = l.split_whitespace();
            let _addr = it.next()?;
            let line: u32 = it.next()?.parse().ok()?;
            let col: u32 = it.next()?.parse().ok()?;
            (line == 2).then_some(col)
        })
        .collect();
    // Several DISTINCT non-zero columns on the one source line — the condition, then, and else each
    // get their own row (line-only granularity would give a single row / a single column).
    let mut distinct: Vec<u32> = cols.iter().copied().filter(|&c| c != 0).collect();
    distinct.sort_unstable();
    distinct.dedup();
    assert!(
        distinct.len() >= 3,
        "expected ≥3 distinct source columns on line 2 (condition/then/else), got {distinct:?}\n{stdout}"
    );
}

#[test]
fn a_runtime_bytes_returning_program_carries_dwarf() {
    // A program returning a RUNTIME `Bytes` (a recursion + `Bytes.concat` result) crosses via the
    // looping-walker escape path (`emit_runtime_bytes_resource`) — a THIRD resource core beyond the
    // flat/sum ones. Its user bodies lead the code section, so the `.debug_*` + `name` sections
    // attribute correctly; a compound/bytes-returning program is debuggable too.
    let src = "(module m \
                     (def (uleb n) (if (< n 128) \
                        ((. Bytes of) (list ((. UInt8 wrap) n))) \
                        ((. Bytes concat) ((. Bytes of) (list ((. UInt8 wrap) (| (& n 127) 128)))) (uleb (>> n 7))))) \
                     (def (main) (uleb 624485)) \
                     (export main))";
    let debug = component_of(src, Target::WasmDebug);
    let has = |needle: &[u8]| debug.windows(needle.len()).any(|w| w == needle);
    for want in [
        b".debug_info".as_slice(),
        b".debug_line".as_slice(),
        b"\x04name".as_slice(),
    ] {
        assert!(
            has(want),
            "a runtime-Bytes program must carry {:?}",
            std::str::from_utf8(want).unwrap_or("<name>")
        );
    }
    assert!(
        has(b"uleb") && has(b"main"),
        "DWARF must name the source functions"
    );
    // A plain build embeds no DWARF (debug-only sections).
    let plain = component_of(src, Target::Wasm);
    assert!(
        !plain
            .windows(b".debug_info".len())
            .any(|w| w == b".debug_info"),
        "a plain runtime-Bytes component must carry no DWARF"
    );
}
