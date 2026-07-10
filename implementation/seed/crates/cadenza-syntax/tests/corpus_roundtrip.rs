//! The milestone gate: the ML surface round-trips every corpus program.
//!
//! For each `(input <program>)` in `spec/semantics/*.sexp`, assert
//! `read_ml(print_ml(sexpr_input)).structurally_eq(sexpr_input)` — i.e. printing the program as ML
//! and re-parsing it yields the same tree. The s-expr reader is the independent ORACLE (a different
//! code path from the ML reader/printer), so this catches a bug in either direction.
//!
//! Failures are bucketed by the head symbol of the input form, so a regression points at the
//! construct that broke. Also checks idempotence: `print_ml(x) == print_ml(read_ml(print_ml(x)))`.

use cadenza_syntax::ast::{Arenas, Struct, StructId};
use cadenza_syntax::{parser, printer, sexpr};
use std::collections::BTreeMap;

const WIDTH: usize = 100;

/// Directory of corpus files, relative to this crate's manifest.
fn corpus_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../spec/semantics")
        .canonicalize()
        .expect("locate spec/semantics")
}

/// Pull every `(input <program>)` argument out of a parsed corpus file. The file is a `(do (case …)
/// …)`; each case contains an `(input …)`.
fn inputs_of(file: &Arenas) -> Vec<Arenas> {
    let mut out = Vec::new();
    // walk the whole structure arena for any `(input X)` form and lift X into its own arena
    for id in (0..file.structure.len() as u32).map(StructId) {
        if let Some(args) = file.as_form(id, "input") {
            if args.len() == 1 {
                out.push(lift(file, args[0]));
            }
        }
    }
    out
}

/// Copy the sub-tree rooted at `id` (within `src`) into a fresh standalone arena.
fn lift(src: &Arenas, id: StructId) -> Arenas {
    let mut b = cadenza_syntax::Builder::new();
    let root = copy(src, id, &mut b);
    b.finish(root)
}

fn copy(src: &Arenas, id: StructId, b: &mut cadenza_syntax::Builder) -> StructId {
    match src.get(id) {
        Struct::Atom(l) => b.atom_leaf(src.leaf(*l).clone()),
        Struct::List(items) => {
            let children: Vec<StructId> = items.iter().map(|&c| copy(src, c, b)).collect();
            b.list(children)
        }
    }
}

#[test]
fn ml_surface_round_trips_the_corpus() {
    let dir = corpus_dir();
    let mut total = 0usize;
    let mut passed = 0usize;
    // head-symbol -> (count, first failing example rendered)
    let mut fail_buckets: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut files = 0usize;

    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("read corpus dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("sexp"))
        .collect();
    entries.sort();

    for path in entries {
        files += 1;
        let text = std::fs::read_to_string(&path).unwrap();
        let file = sexpr::read_all(&text)
            .unwrap_or_else(|e| panic!("oracle parse {}: {}", path.display(), e.0));
        for input in inputs_of(&file) {
            total += 1;
            let bucket = input.head_name(input.root).unwrap_or("<leaf>").to_string();

            let ml = printer::print(&input, WIDTH);
            let reparsed = parser::read_ml(&ml);

            let ok = reparsed.ok()
                && reparsed.arenas.structurally_eq(&input)
                // idempotence: printing the reparsed tree is byte-identical
                && printer::print(&reparsed.arenas, WIDTH) == ml;

            if ok {
                passed += 1;
            } else {
                let entry = fail_buckets.entry(bucket).or_insert((0, String::new()));
                entry.0 += 1;
                if entry.1.is_empty() {
                    let reason = if !reparsed.ok() {
                        format!("parse error: {:?}", reparsed.errors.first())
                    } else if !reparsed.arenas.structurally_eq(&input) {
                        "AST mismatch".to_string()
                    } else {
                        "not idempotent".to_string()
                    };
                    entry.1 = format!("{reason}\n    s-expr: {}\n    ml:     {ml}", sexpr::print(&input));
                }
            }
        }
    }

    eprintln!("corpus round-trip: {passed}/{total} inputs across {files} files");
    if passed != total {
        eprintln!("\nfailure buckets (head symbol -> count):");
        for (head, (count, sample)) in &fail_buckets {
            eprintln!("  {head}: {count}\n    {sample}");
        }
    }
    assert!(files >= 20, "expected >=20 corpus files, found {files}");
    assert_eq!(passed, total, "not all corpus inputs round-trip through the ML surface");
}
