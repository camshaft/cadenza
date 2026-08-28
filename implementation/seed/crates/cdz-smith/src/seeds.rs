//! Seed the fuzz corpus with the semantics-corpus ASTs (S2).
//!
//! Operator mandate: *"seed the corpus with the semantics corpus ASTs — that'll give really high
//! quality seeds."* Each `spec/semantics/NN-*.sexp` case is
//! `(case "desc" (input <program>) <result>…)`; the `<program>` is a real, hand-authored Cadenza
//! fragment covering one language feature. This module extracts every such `<program>`, re-roots it
//! as a standalone [`Arenas`], and encodes it to the canonical binary-AST bytes the entropy oracle
//! ([`crate::oracle::compile_catching_ast`]) consumes.
//!
//! Why it matters: the next-gen engine's entropy IS the binary AST. Mutating from a corpus of dense,
//! feature-covering real programs reaches the compiler with well-formed programs far more often than
//! mutating from empty/random bytes — libFuzzer splices and bit-flips these seeds, and the strict
//! decode-gate keeps only the mutations that stay well-formed. High-quality seeds are the difference
//! between a fuzzer that spends its budget in the parser and one that spends it in the backend.

use std::io;
use std::path::{Path, PathBuf};

use cadenza_syntax::ast::{Arenas, Builder, Struct, StructId};

/// Extract every corpus `(input <program>)` program from one PARSED corpus document, re-rooting each
/// as a standalone `Arenas` and encoding it to canonical binary-AST bytes.
///
/// Precise by construction: we only extract an `(input X)` form that is a DIRECT CHILD of a
/// `(case …)` form, so a program fragment that happens to apply a function named `input` is never
/// mistaken for a case clause. `(input <program>)` has exactly one payload child (`X`), which is the
/// program we re-root.
pub fn extract_from_arenas(arenas: &Arenas) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for i in 0..arenas.structure.len() {
        let case_id = StructId(i as u32);
        let Struct::List(case_kids) = arenas.get(case_id) else {
            continue;
        };
        if case_kids.first().map(|&h| arenas.as_name(h)) != Some(Some("case")) {
            continue;
        }
        for &clause in case_kids {
            let Struct::List(input_kids) = arenas.get(clause) else {
                continue;
            };
            if input_kids.first().map(|&h| arenas.as_name(h)) == Some(Some("input"))
                && input_kids.len() >= 2
            {
                let program = input_kids[1];
                let rerooted = reroot(arenas, program);
                out.push(cadenza_syntax::codec::encode(&rerooted));
            }
        }
    }
    out
}

/// Re-root the subtree at `id` of `src` as its own `Arenas` (a standalone program). `codec::encode`
/// canonicalizes, so equal programs across cases encode to identical bytes (which the writer dedups).
fn reroot(src: &Arenas, id: StructId) -> Arenas {
    let mut b = Builder::new();
    let root = copy_subtree(src, id, &mut b);
    b.finish(root)
}

fn copy_subtree(src: &Arenas, id: StructId, b: &mut Builder) -> StructId {
    match src.get(id) {
        Struct::Atom(leaf_id) => {
            let leaf = src.leaf(*leaf_id).clone();
            b.atom_leaf(leaf)
        }
        Struct::List(children) => {
            let kids: Vec<StructId> = children
                .clone()
                .into_iter()
                .map(|c| copy_subtree(src, c, b))
                .collect();
            b.list(kids)
        }
    }
}

/// Read one corpus `.sexp` file and return its extracted seed blobs. A parse failure is an error — a
/// promoted corpus file always parses; a failure here means the corpus (or the reader) is broken.
pub fn extract_from_file(path: &Path) -> io::Result<Vec<Vec<u8>>> {
    let text = std::fs::read_to_string(path)?;
    // A corpus file is a SEQUENCE of top-level `(case …)` forms — `read_all` wraps them in a
    // synthetic `(do …)` root (plain `read` accepts only a single form and errors on the rest).
    let arenas = cadenza_syntax::sexpr::read_all(&text).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("parse {}: {}", path.display(), e.0),
        )
    })?;
    Ok(extract_from_arenas(&arenas))
}

/// The result of a corpus-seeding run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SeedStats {
    /// `.sexp` files scanned.
    pub files: usize,
    /// Distinct seed blobs written (after content dedup).
    pub written: usize,
    /// Seed blobs that were byte-identical to one already written (skipped).
    pub duplicates: usize,
}

/// Extract all seeds from every `*.sexp` in `semantics_dir` and write them to `out_dir`, one canonical
/// binary-AST blob per file, named by a stable content hash (`<hex>.ast`) so identical programs across
/// cases collapse to ONE seed and re-running is idempotent (never accumulates). `out_dir` is created
/// if missing and any pre-existing `*.ast` seeds are cleared first, so the output reflects exactly the
/// current corpus.
pub fn write_seed_corpus(semantics_dir: &Path, out_dir: &Path) -> io::Result<SeedStats> {
    std::fs::create_dir_all(out_dir)?;
    clear_existing_seeds(out_dir)?;

    let mut paths: Vec<PathBuf> = std::fs::read_dir(semantics_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("sexp"))
        .collect();
    paths.sort();

    let mut stats = SeedStats::default();
    let mut seen = std::collections::HashSet::new();
    for path in &paths {
        stats.files += 1;
        for bytes in extract_from_file(path)? {
            let hash = content_hash(&bytes);
            if !seen.insert(hash) {
                stats.duplicates += 1;
                continue;
            }
            std::fs::write(out_dir.join(format!("{hash:016x}.ast")), &bytes)?;
            stats.written += 1;
        }
    }
    Ok(stats)
}

/// Remove any `*.ast` seed files already in `out_dir` (idempotent re-runs).
fn clear_existing_seeds(out_dir: &Path) -> io::Result<()> {
    for entry in std::fs::read_dir(out_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|s| s.to_str()) == Some("ast") {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// A stable content hash for seed filenames (FNV-1a — deterministic across runs, unlike the stdlib
/// `DefaultHasher`, and dependency-free so the fuzz binary stays lean).
fn content_hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Walk up from `start` to find the `spec/semantics` corpus directory. Mirrors how the finding store
/// discovers the repo root, so `seed-corpus` works from any CWD inside the tree.
pub fn discover_semantics_dir(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        let cand = d.join("spec").join("semantics");
        if cand.is_dir() {
            return Some(cand);
        }
        dir = d.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oracle::{Verdict, compile_catching_ast};

    /// A minimal corpus document with two cases → two extracted seeds, each a decodable, compilable
    /// binary-AST blob. Proves the case→input→reroot→encode pipeline end to end.
    #[test]
    fn extracts_input_programs_as_compilable_ast_blobs() {
        let doc = r#"
            (case "addition"
              (input  (do (def (main) (+ 2 3)) (export main)))
              (output (: 5 Int64)))
            (case "a literal"
              (input  (do (def (main) 42) (export main)))
              (output (: 42 Int64)))
        "#;
        let arenas = cadenza_syntax::sexpr::read_all(doc).expect("doc parses");
        let seeds = extract_from_arenas(&arenas);
        assert_eq!(seeds.len(), 2, "one seed per (input …) case clause");
        for bytes in &seeds {
            // Every extracted seed is a well-formed binary AST that the entropy oracle compiles.
            let v = compile_catching_ast(bytes);
            assert!(
                matches!(v, Verdict::Compiled { .. }),
                "extracted seed should compile, got {v:?}"
            );
        }
    }

    /// A bare `(input X)` NOT inside a `(case …)`, and a program that applies a function named
    /// `input`, must both be ignored — extraction keys on the case-clause structure, not the name.
    #[test]
    fn ignores_non_case_input_forms() {
        let doc = r#"
            (input (+ 1 1))
            (case "real" (input 7) (output (: 7 Int64)))
            (some-form (input 9))
        "#;
        let arenas = cadenza_syntax::sexpr::read_all(doc).expect("doc parses");
        let seeds = extract_from_arenas(&arenas);
        assert_eq!(
            seeds.len(),
            1,
            "only the (input …) under a (case …) is a seed"
        );
    }

    /// Identical programs across cases dedup to a single content-hashed seed; the writer is
    /// idempotent (re-running yields the same file set, never accumulating).
    #[test]
    fn write_seed_corpus_dedups_and_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("cdz-smith-seeds-{}", std::process::id()));
        let semantics = dir.join("spec").join("semantics");
        std::fs::create_dir_all(&semantics).unwrap();
        std::fs::write(
            semantics.join("00-x.sexp"),
            r#"(case "a" (input 42) (output (: 42 Int64)))
               (case "b" (input 42) (output (: 42 Int64)))
               (case "c" (input (+ 1 2)) (output (: 3 Int64)))"#,
        )
        .unwrap();
        let out = dir.join("out");

        let s1 = write_seed_corpus(&semantics, &out).unwrap();
        assert_eq!(s1.files, 1);
        assert_eq!(s1.written, 2, "42 and (+ 1 2) — the two 42s collapse");
        assert_eq!(s1.duplicates, 1);
        let count1 = std::fs::read_dir(&out).unwrap().count();
        assert_eq!(count1, 2);

        // Re-run: same output, no accumulation.
        let s2 = write_seed_corpus(&semantics, &out).unwrap();
        assert_eq!(s2, s1);
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn discovers_semantics_dir_walking_up() {
        let dir = std::env::temp_dir().join(format!("cdz-smith-disc-{}", std::process::id()));
        let semantics = dir.join("spec").join("semantics");
        let deep = dir.join("a").join("b");
        std::fs::create_dir_all(&semantics).unwrap();
        std::fs::create_dir_all(&deep).unwrap();
        assert_eq!(
            discover_semantics_dir(&deep).as_deref(),
            Some(semantics.as_path())
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
