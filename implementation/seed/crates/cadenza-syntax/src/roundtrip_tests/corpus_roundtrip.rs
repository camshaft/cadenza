//! The milestone gate: the ML surface round-trips every corpus program.
//!
//! For each `(input <program>)` in `spec/semantics/*.sexp`, assert
//! `read_ml(print_ml(sexpr_input)).structurally_eq(sexpr_input)` — i.e. printing the program as ML
//! and re-parsing it yields the same tree. The s-expr reader is the independent ORACLE (a different
//! code path from the ML reader/printer), so this catches a bug in either direction.
//!
//! Failures are bucketed by the head symbol of the input form, so a regression points at the
//! construct that broke. Also checks idempotence: `print_ml(x) == print_ml(read_ml(print_ml(x)))`.
//!
//! A few heads are CANONICALIZED away by the ML surface (a qualified infix head like `Unit.^` prints
//! as the bare glyph `^`); for those the contract is idempotence only, not structural equality — see
//! [`has_canonicalizing_head`].

use crate::ast::{Arenas, CompoundCtor, Struct, StructId};
use crate::{codec, parser, printer, sexpr, token};
use std::collections::BTreeMap;

const WIDTH: usize = 100;

/// Does this tree contain a head that the ML surface CANONICALIZES away — a QUALIFIED infix head
/// (`Unit.^`, `Unit.*`, `Unit./`) whose surface glyph drops the qualifier (`^`, `*`, `/`)? For such
/// an input the guaranteed contract is only an IDEMPOTENT round-trip (`ml(ml(x)) == ml(x)`), not
/// structural equality: `(Unit.^ u 2)` prints as `u ^ 2`, which re-reads to the BARE `^` arena head
/// (the units layer treats `^`/`*`/`/` as unit composition), so the surface deliberately collapses
/// `Unit.^` → `^` exactly as it collapses name-alias constructors. `structurally_eq` cannot hold
/// across that collapse, so these inputs are checked for parse-ok + idempotence only. See
/// `token::infix_glyph`.
fn has_canonicalizing_head(a: &Arenas) -> bool {
    (0..a.structure.len() as u32).map(StructId).any(|id| {
        a.head_name(id)
            .is_some_and(|h| h.contains('.') && token::infix_glyph(h) != h)
    })
}

/// Does this tree contain a MEMBER access `(. obj key)` whose KEY is a COMPOUND (call-shaped) form —
/// e.g. `(. m (meta capabilities))`, a module METADATA projection (the `(meta …)` channel, distinct from
/// an export field)? The ML `.member` surface only spells a PLAIN-NAME key (`obj.key`); a compound key
/// has no faithful member surface, so the printer falls back to the quoted-operator `` `.`(obj, key) ``,
/// which re-reads to a `.`-APPLICATION, not the same `Member` node. Exactly like `Unit.^`, these are held
/// to the weaker parse-ok contract only — the codec (binary) and s-expr paths preserve the node EXACTLY
/// (the program's identity), only the ML text round-trip is exempt. See [`has_canonicalizing_head`].
fn has_compound_key_member(a: &Arenas) -> bool {
    (0..a.structure.len() as u32).map(StructId).any(|id| {
        a.member_parts(id)
            .is_some_and(|(_, key)| matches!(a.get(key), Struct::List(_)))
    })
}

/// Does this tree contain a native RATIONAL literal `(RationalTag num den)`? Per seq-204 the ML surface
/// has NO rational literal — a rational is a DISPLAY value form (`num/den`), and on the ML surface an
/// unspaced `num/den` LEXES as integer DIVISION `(/ num den)`, not a rational. So a rational prints to
/// `num/den` but re-reads to a `/`-division node — structurally DIFFERENT *and* non-idempotent (`(/ n d)`
/// re-prints spaced as `n / d`, not `n/d`). A rational thus has no faithful ML SOURCE round-trip; like a
/// rejected (error) program, its only meaningful ML contract is PARSE-OK — the codec (binary) and s-expr
/// paths preserve the rational EXACTLY (its program identity). A rational reaches ML *source* via a
/// `(/ n d)` / `Rational.of` construction, never a literal. See `cadenza-syntax` lexer (seq-204).
fn has_rational(a: &Arenas) -> bool {
    (0..a.structure.len() as u32)
        .map(StructId)
        .any(|id| a.rational_parts(id).is_some())
}

/// Does this tree contain a MALFORMED `record`/`map` compound-ctor literal — one whose direct element
/// list carries a BARE ATOM (a non-`(= key value)`, non-`(.. rest)` element)? A record/map field MUST be
/// a `(= k v)` pair (or a `(.. rest)` spread); a bare atom (e.g. `#record((= a 1) 2)`) is malformed and
/// has NO faithful ML surface — the `{ … }` / `#{ … }` literal can only spell `k = v` fields, so the
/// printer falls back to the name-head `record(…)` / `map(…)` CALL form, which re-reads as a name-head
/// list (NOT the `#record`/`#map` ctor). Exactly like a rejected (error) program or a rational literal,
/// such a malformed literal's only meaningful ML contract is PARSE-OK; the codec/s-expr paths keep it
/// EXACT. These arise as reify/quote TEST DATA (v-metaprog's malformed-collection cases, #6921) —
/// deliberately-malformed AST carried as data, whose identity the binary AST preserves. See
/// [`has_canonicalizing_head`] / `no_ml_source_form`.
fn has_malformed_compound_ctor(a: &Arenas) -> bool {
    (0..a.structure.len() as u32).map(StructId).any(|id| {
        // A record/map — the native ctor-leaf `#record`/`#map`, or a str/name `record`/`map` head.
        let is_rec_or_map = matches!(
            a.compound_ctor_leaf(id),
            Some(CompoundCtor::Record) | Some(CompoundCtor::Map)
        ) || matches!(a.head_ctor(id), Some("record") | Some("map"))
            || matches!(a.head_name(id), Some("record") | Some("map"));
        if !is_rec_or_map {
            return false;
        }
        let Struct::List(items) = a.get(id) else {
            return false;
        };
        // Skip the ctor head (items[0]); a bare ATOM element is definitively malformed (a valid field is
        // the `(= k v)` field-pair LIST or a `(.. rest)` spread LIST — never a bare scalar).
        items
            .iter()
            .skip(1)
            .any(|&e| matches!(a.get(e), Struct::Atom(_)))
    })
}

/// A hint for the commonest round-trip authoring mistake: a `record` literal whose fields are written
/// POSITIONAL `(name value)` instead of `(= name value)`. `structurally_eq` collapses a ctor HEAD
/// (`record` ↔ `"record"`, `list` ↔ `"list"`, …), so a name-head compound literal is fine — but the ML
/// surface prints a record field as `name = value`, which re-reads to the 3-element `(= name value)`,
/// NOT the 2-element positional `(name value)` the input used, so they differ and the round-trip fails.
/// If the tree carries a `record` head, point the author at the `=` field form. Heuristic — only
/// consulted on an already-failing case, so an over-broad match is harmless.
fn name_head_ctor_hint(a: &Arenas) -> Option<String> {
    let heads: Vec<Option<&str>> = (0..a.structure.len() as u32)
        .map(StructId)
        .map(|id| a.head_name(id))
        .collect();
    let mut hints: Vec<&str> = Vec::new();
    // A record VALUE/PATTERN literal (`record` head): a field is `(= name value)`. A positional
    // `(name value)` reprints as `name = value` → re-reads to `(= name value)` (structural mismatch).
    if heads.contains(&Some("record")) {
        hints.push(
            "a record VALUE/PATTERN field must be (= name value), NOT positional (name value) — \
             author (record (= f v) …)",
        );
    }
    // A record TYPE (`Record` head): a field is `(: name Type)`. A positional `(name Type)` reprints
    // as the un-reparseable `Record(name(Type))` (a record-type field is `name: Type`, not `name(Type)`)
    // → a hard parse ERROR on re-read, not merely a mismatch.
    if heads.contains(&Some("Record")) {
        hints.push(
            "a record-TYPE field must be (: name Type), NOT positional (name Type) — author \
             (Record (: f T) …) (positional prints the un-reparseable field(T))",
        );
    }
    (!hints.is_empty()).then(|| hints.join("; "))
}

/// Truncate to at most `n` CHARS (not bytes — never splits a scalar), appending `…` when cut.
fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

/// Directory of corpus files, relative to this crate's manifest.
fn corpus_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../spec/semantics")
        .canonicalize()
        .expect("locate spec/semantics")
}

/// Pull every case's `(input <program>)` out of a parsed corpus file, paired with whether that case is
/// an ERROR case (has an `(error …)` sibling). The file is a `(do (case …) …)`; each case contains an
/// `(input …)` and either an `(output …)` or an `(error …)`.
///
/// The error flag drives a weaker round-trip contract for a REJECTED program: its INPUT is a malformed
/// program (a coded reject, e.g. CDZ0201), so its exact AST structure need not survive the ML surface —
/// the surface legitimately normalizes/re-homes a malformed construct (e.g. a valueless `type`
/// declaration in tail position re-homes to a sibling). Such an input is held to parse-ok + idempotence
/// only, not structural equality (the same weakening `has_canonicalizing_head` applies to a
/// surface-canonicalized head). The codec (binary) and s-expr paths stay exact — they are bijections
/// that preserve any well-formed arena regardless of its semantic validity.
fn inputs_of(file: &Arenas) -> Vec<(Arenas, bool)> {
    let mut out = Vec::new();
    for id in (0..file.structure.len() as u32).map(StructId) {
        let Some(case_args) = file.as_form(id, "case") else {
            continue;
        };
        let mut input: Option<StructId> = None;
        let mut is_error = false;
        for &child in case_args {
            if let Some(ia) = file.as_form(child, "input")
                && ia.len() == 1
            {
                input = Some(ia[0]);
            } else if file.as_form(child, "error").is_some() {
                is_error = true;
            }
        }
        if let Some(x) = input {
            out.push((lift(file, x), is_error));
        }
    }
    out
}

/// Copy the sub-tree rooted at `id` (within `src`) into a fresh standalone arena.
fn lift(src: &Arenas, id: StructId) -> Arenas {
    let mut b = crate::Builder::new();
    let root = copy(src, id, &mut b);
    b.finish(root)
}

fn copy(src: &Arenas, id: StructId, b: &mut crate::Builder) -> StructId {
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
    // EVERY failure, with its FILE, so a corpus author sees exactly which case to fix (not just one
    // sample per head bucket) — this is the actionable-lint half of the round-trip gate.
    let mut failures: Vec<String> = Vec::new();
    let mut files = 0usize;

    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("read corpus dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("sexp"))
        .collect();
    entries.sort();

    for path in entries {
        files += 1;
        let fname = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let text = std::fs::read_to_string(&path).unwrap();
        let file = sexpr::read_all(&text)
            .unwrap_or_else(|e| panic!("oracle parse {}: {}", path.display(), e.0));
        for (input, is_error) in inputs_of(&file) {
            total += 1;
            let bucket = input.head_name(input.root).unwrap_or("<leaf>").to_string();

            let ml = printer::print(&input, WIDTH);
            let reparsed = parser::read_ml(&ml);

            // A head the surface canonicalizes away (`Unit.^` → `^`) cannot satisfy structural equality,
            // so it is held to the weaker parse-ok + idempotence contract only.
            let structural_required =
                !has_canonicalizing_head(&input) && !has_compound_key_member(&input);

            // A REJECTED program (an error case) is MALFORMED, so the ML surface has no faithful
            // rendering of it — a valueless construct in a value position (e.g. a trailing `type`
            // declaration in a function body) legitimately re-homes/collapses, breaking BOTH structural
            // equality AND idempotence. Its only meaningful ML-surface contract is PARSE-OK: the printed
            // ML re-parses. (Surface FIDELITY is the contract for VALID programs; the codec/s-expr paths
            // stay exact regardless.)
            // A native RATIONAL literal has no faithful ML SOURCE form (seq-204): it prints to the display
            // `num/den`, which the ML surface RE-READS as integer division `(/ num den)` — structurally
            // different AND non-idempotent. So, exactly like a rejected (error) program, a rational-bearing
            // input's only meaningful ML contract is PARSE-OK; the codec/s-expr paths keep it exact.
            let no_ml_source_form =
                is_error || has_rational(&input) || has_malformed_compound_ctor(&input);
            let ok = reparsed.ok()
                && (no_ml_source_form
                    || ((!structural_required || reparsed.arenas.structurally_eq(&input))
                        // idempotence: printing the reparsed tree is byte-identical
                        && printer::print(&reparsed.arenas, WIDTH) == ml));

            if ok {
                passed += 1;
            } else {
                let reason = if !reparsed.ok() {
                    format!("parse error: {:?}", reparsed.errors.first())
                } else if structural_required && !reparsed.arenas.structurally_eq(&input) {
                    "AST mismatch".to_string()
                } else {
                    "not idempotent".to_string()
                };
                let sexp = sexpr::print(&input);
                // Actionable hint for the most common authoring mistake: a value-position name-head
                // compound-ctor literal (`(record (f v) …)` / `(list …)` / `(tuple …)` / `(map …)`),
                // which the ML surface canonicalizes to the unshadowable str-head form. Author it that
                // way from the start so it round-trips structurally.
                let hint = name_head_ctor_hint(&input);
                let entry = fail_buckets.entry(bucket).or_insert((0, String::new()));
                entry.0 += 1;
                if entry.1.is_empty() {
                    entry.1 = format!("{reason}\n    s-expr: {sexp}\n    ml:     {ml}");
                }
                failures.push(format!(
                    "  [{fname}] {reason}{}\n    s-expr: {}",
                    hint.map(|h| format!(" — {h}")).unwrap_or_default(),
                    truncate(&sexp, 220),
                ));
            }
        }
    }

    eprintln!("corpus round-trip: {passed}/{total} inputs across {files} .sexp files");
    if passed != total {
        eprintln!("\nfailure buckets (head symbol -> count):");
        for (head, (count, sample)) in &fail_buckets {
            eprintln!("  {head}: {count}\n    {sample}");
        }
        eprintln!("\nALL {} failing cases (file + reason):", failures.len());
        for f in &failures {
            eprintln!("{f}");
        }
    }
    // This test's oracle is the s-expression reader, so it covers the `.sexp` corpus only. As files
    // migrate to markdown (their inputs are already ML), the `.sexp` count shrinks; the migrated
    // `.md` files' ML round-trip is covered by `cdz-corpus`'s own `markdown::check`. So we only
    // require that SOME `.sexp` corpus remains to exercise this path — not a fixed count.
    assert!(
        files > 0,
        "expected at least one .sexp corpus file, found none"
    );
    assert_eq!(
        passed, total,
        "not all corpus inputs round-trip through the ML surface"
    );
}

#[test]
fn binary_surface_round_trips_the_corpus() {
    // The BINARY codec's round-trip over the whole real program corpus — the counterpart to the ML
    // test above, whose oracle is the same independent s-expr reader. `codec::decode(encode(x))` must
    // be structurally equal to `x` (the bijection guarantee, ast-encoding.md), and `encode` must be a
    // deterministic canonical fixed point (`encode(decode(encode(x))) == encode(x)`). Before this,
    // the codec's only round-trip coverage was a handful of hand-built arenas in `codec.rs` — the
    // corpus (every construct the language actually emits) never exercised the binary surface.
    let dir = corpus_dir();
    let mut total = 0usize;
    let mut passed = 0usize;
    let mut fail_buckets: BTreeMap<String, (usize, String)> = BTreeMap::new();

    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("read corpus dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("sexp"))
        .collect();
    entries.sort();

    for path in entries {
        let text = std::fs::read_to_string(&path).unwrap();
        let file = sexpr::read_all(&text)
            .unwrap_or_else(|e| panic!("oracle parse {}: {}", path.display(), e.0));
        // The codec is a bijection over any well-formed arena, so binary round-trip is exact even for a
        // rejected program's input — no error-case exemption here (unlike the ML surface).
        for (input, _is_error) in inputs_of(&file) {
            total += 1;
            let bucket = input.head_name(input.root).unwrap_or("<leaf>").to_string();

            let bytes = codec::encode(&input);
            let ok = match codec::decode(&bytes) {
                None => false,
                Some(back) => {
                    // Structural equality (the raw arena fields differ after a canonicalizing encode)
                    // AND encode is a deterministic fixed point on the decoded canonical form.
                    back.structurally_eq(&input) && codec::encode(&back) == bytes
                }
            };

            if ok {
                passed += 1;
            } else {
                let entry = fail_buckets.entry(bucket).or_insert((0, String::new()));
                entry.0 += 1;
                if entry.1.is_empty() {
                    let reason = match codec::decode(&bytes) {
                        None => "decode returned None on a valid encoding".to_string(),
                        Some(back) if !back.structurally_eq(&input) => "AST mismatch".to_string(),
                        _ => "encode not a fixed point".to_string(),
                    };
                    entry.1 = format!("{reason}\n    s-expr: {}", sexpr::print(&input));
                }
            }
        }
    }

    eprintln!("binary round-trip: {passed}/{total} corpus inputs");
    if passed != total {
        eprintln!("\nfailure buckets (head symbol -> count):");
        for (head, (count, sample)) in &fail_buckets {
            eprintln!("  {head}: {count}\n    {sample}");
        }
    }
    assert_eq!(
        passed, total,
        "not all corpus inputs round-trip through the binary codec"
    );
}

#[test]
fn all_surface_paths_round_trip_the_corpus() {
    // The full surface matrix: every corpus program must survive ml→sexpr, sexpr→binary→sexpr, and
    // ml→binary→ml — the three text/binary projections of one arena are lossless in every direction.
    // The ml and binary single-surface tests above pin the two hard paths; this pins the CROSS-surface
    // compositions (the ones a real tool chains: read ML → serialize binary → later print ML) so a
    // regression in any conversion seam is caught against the whole corpus, not a hand-picked sample.
    let dir = corpus_dir();
    let mut total = 0usize;
    let mut passed = 0usize;
    let mut fail_buckets: BTreeMap<String, (usize, String)> = BTreeMap::new();

    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("read corpus dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("sexp"))
        .collect();
    entries.sort();

    for path in entries {
        let text = std::fs::read_to_string(&path).unwrap();
        let file = sexpr::read_all(&text)
            .unwrap_or_else(|e| panic!("oracle parse {}: {}", path.display(), e.0));
        for (input, is_error) in inputs_of(&file) {
            total += 1;
            let bucket = input.head_name(input.root).unwrap_or("<leaf>").to_string();

            // A canonicalizing head (`Unit.^` → `^`) collapses under the ML surface, so Path A
            // (ml→binary→ml) is held to the idempotence contract, not structural equality.
            let structural = !has_canonicalizing_head(&input) && !has_compound_key_member(&input);

            // Path A: ml → binary → ml. Print ML, read it back to an arena, encode, decode, print ML
            // again — the two ML renderings must be byte-identical (and structurally equal to the input
            // when the head is not canonicalized away). A REJECTED program (error case) is malformed and
            // has no faithful ML rendering (it re-homes/collapses), so Path A requires only that the
            // ml→binary→decode composition SUCCEEDS, not fidelity — matching the ML-only test's parse-ok
            // contract for error cases. (Path B below stays exact: the s-expr oracle and codec are
            // bijections independent of semantic validity.)
            // A rational literal has no faithful ML SOURCE form (seq-204: `num/den` prints for display but
            // re-reads as integer division), so Path A — which routes THROUGH ML — is held to the same
            // composition-succeeds contract as an error case. Path B below (sexpr→binary→sexpr) stays EXACT
            // for a rational: the s-expr surface + codec are lossless bijections for `(RationalTag n d)`.
            let ml = printer::print(&input, WIDTH);
            let via_bin = codec::decode(&codec::encode(&parser::read_ml(&ml).arenas));
            let path_a = match &via_bin {
                Some(a) => {
                    is_error
                        || has_rational(&input)
                        // A malformed record/map ctor (a bare non-field element) has no faithful ML
                        // surface (prints name-head), so Path A is held to composition-succeeds, like an
                        // error/rational case; Path B below keeps its identity exact via the s-expr codec.
                        || has_malformed_compound_ctor(&input)
                        || (printer::print(a, WIDTH) == ml
                            && (!structural || a.structurally_eq(&input)))
                }
                None => false,
            };

            // Path B: sexpr → binary → sexpr. The s-expr text is the input's own surface here; encode
            // then decode then re-print s-expr must reproduce the canonical s-expr text.
            let sx = sexpr::print(&input);
            let sx_arena = sexpr::read(&sx).expect("oracle re-reads its own print");
            let path_b = match codec::decode(&codec::encode(&sx_arena)) {
                Some(a) => sexpr::print(&a) == sx && a.structurally_eq(&input),
                None => false,
            };

            if path_a && path_b {
                passed += 1;
            } else {
                let entry = fail_buckets.entry(bucket).or_insert((0, String::new()));
                entry.0 += 1;
                if entry.1.is_empty() {
                    let which = if !path_a {
                        "ml→binary→ml"
                    } else {
                        "sexpr→binary→sexpr"
                    };
                    entry.1 = format!("{which} failed\n    s-expr: {sx}\n    ml:     {ml}");
                }
            }
        }
    }

    eprintln!("all-surface round-trip: {passed}/{total} corpus inputs");
    if passed != total {
        eprintln!("\nfailure buckets (head symbol -> count):");
        for (head, (count, sample)) in &fail_buckets {
            eprintln!("  {head}: {count}\n    {sample}");
        }
    }
    assert_eq!(
        passed, total,
        "not all corpus inputs round-trip through every surface path"
    );
}
