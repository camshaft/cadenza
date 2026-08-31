//! Self-consistency check for the `spec/syntax/` parser/printer golden corpus (Increment 2 of
//! DESIGN-parser-test-corpus.md). Each case is a directory holding `input.<ext>` (the surface source),
//! `tree.sexp` (the STRUCTURAL parse-tree golden — comments expanded to `(comment …)` nodes, produced by
//! `sexpr::render_sexpr`, DESIGN §2), and an OPTIONAL `format.<ext>` (the canonical-format golden, DESIGN
//! §3). This test enforces, against the REFERENCE `cadenza-syntax` reader/printer, that:
//!
//!   render_sexpr(read(input))                     == tree.sexp                         (bytes)
//!   fmt(input)                                     == format.<ext>  (or input if absent) (bytes)
//!
//! A DECLINE case (Increment 4, the grader's `Todo`) carries NO `tree.sexp`: the reader is expected to
//! REFUSE its `input` (a malformed or not-yet-realized surface). read-Ok ⟺ well-formed (has a golden);
//! read-Err ⟺ decline (no golden). A decline may carry an optional `error.txt` pinning a substring its
//! diagnostic must contain (parse-error quality, DESIGN §10).
//!
//! It is the Increment-2 gate — a bootstrapping self-consistency check, no new harness. Increment 3 adds
//! the real syntax grader + per-case nix derivations (the authoritative, cached gate). Because this test
//! reads files from the repo's `spec/` tree (which a crate-scoped nix test sandbox may not carry), it
//! SKIPS cleanly (loud, not silent) when the corpus root is absent, so it is a full check in a real
//! checkout / dev-gate and a no-op where the tree is unavailable.
//!
//! Regenerate the goldens after editing an `input.<ext>` (or adding a case) with:
//!   CDZ_BLESS=1 cargo test -p cadenza-syntax --lib syntax_corpus_tests
//! Bless writes `tree.sexp` for every case and writes `format.<ext>` only for a case whose input is NOT
//! already canonical (removing a stale one where the input became canonical), so a clean case stays
//! minimal (no redundant format file) per DESIGN §3.

use std::path::{Path, PathBuf};

use crate::convert::{self, Format, Options};
use crate::sexpr;

/// The corpus root: `spec/syntax/`, relative to this crate's manifest dir (crate → crates → seed →
/// implementation → repo root).
fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../spec/syntax")
}

/// `render_sexpr(arena)` with the corpus's one-trailing-newline convention — the `tree.sexp` golden.
fn tree_golden(arena: &crate::ast::Arenas) -> Vec<u8> {
    let mut s = sexpr::render_sexpr(arena);
    s.push('\n');
    s.into_bytes()
}

/// `fmt(input)` for `surface` — read then re-print in the SAME surface, with the trailing-newline
/// convention `run_fmt` uses (append one `\n` if the printer emitted none). This is the exact
/// canonical-format oracle `cdz fmt` compares against (cli.rs `run_fmt`).
fn fmt_bytes(input: &[u8], surface: Format) -> Result<Vec<u8>, convert::ConvertError> {
    let mut b = convert::convert_with(input, surface, surface, Options::default())?;
    if b.last() != Some(&b'\n') {
        b.push(b'\n');
    }
    Ok(b)
}

/// The single `input.*` file in a case directory (there is exactly one), with its inferred surface.
fn find_input(case: &Path) -> Option<(PathBuf, Format)> {
    for entry in std::fs::read_dir(case).ok()? {
        let path = entry.ok()?.path();
        let name = path.file_name()?.to_str()?;
        if let Some(stem) = name.strip_suffix(&format!(".{}", path.extension()?.to_str()?))
            && stem == "input"
            && let Some(fmt) = Format::from_extension(name)
        {
            return Some((path, fmt));
        }
    }
    None
}

/// Enumerate every case directory `spec/syntax/<surface>/<NN-name>/` (two levels deep), sorted for a
/// stable, legible order.
fn enumerate_cases(root: &Path) -> Vec<PathBuf> {
    let mut cases = Vec::new();
    let mut surfaces: Vec<PathBuf> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    surfaces.sort();
    for surface in surfaces {
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(&surface)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        cases.extend(dirs);
    }
    cases
}

#[test]
fn syntax_corpus_goldens_are_self_consistent() {
    let root = corpus_root();
    if !root.is_dir() {
        eprintln!(
            "syntax_corpus: SKIP — corpus root {} is not present in this sandbox (Increment 3's nix \
             per-case derivations are the authoritative gate)",
            root.display()
        );
        return;
    }
    let bless = std::env::var_os("CDZ_BLESS").is_some();
    let cases = enumerate_cases(&root);
    assert!(
        !cases.is_empty(),
        "syntax_corpus: no case directories under {}",
        root.display()
    );

    let mut failures: Vec<String> = Vec::new();
    for case in &cases {
        let label = case
            .strip_prefix(&root)
            .unwrap_or(case)
            .display()
            .to_string();
        let Some((input_path, surface)) = find_input(case) else {
            failures.push(format!("{label}: no `input.<ext>` file"));
            continue;
        };
        let input = std::fs::read(&input_path).expect("read input file");
        let tree_path = case.join("tree.sexp");
        let ext = input_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();

        // Read the surface into the arena. A case is a DECLINE case (Increment 4's `Todo` path) IFF it
        // carries no `tree.sexp`: the reader is expected to REFUSE the input (a malformed or not-yet-
        // realized surface), which has no parse tree. read Ok ⟺ well-formed (has a `tree.sexp` golden);
        // read Err ⟺ decline (no golden). This deterministic split is what lets the corpus pin
        // parse-error behavior alongside successful parses.
        let arena = match convert::read(&input, surface) {
            Ok(a) => a,
            Err(e) => {
                // A decline. It must NOT carry a tree.sexp (a well-formed case that regressed to a
                // decline would still have its golden — catch that). An optional `error.txt` pins a
                // substring the diagnostic must contain (DESIGN §10 parse-error-quality); an unpinned
                // decline just records that it declines.
                if tree_path.exists() {
                    failures.push(format!(
                        "{label}: input DECLINES ({e}) but a tree.sexp golden exists — a well-formed \
                         case regressed to a decline, or delete tree.sexp to record it as a decline case"
                    ));
                } else if let Ok(want) = std::fs::read_to_string(case.join("error.txt")) {
                    let want = want.trim();
                    if !want.is_empty() && !e.to_string().contains(want) {
                        failures.push(format!(
                            "{label}: decline message {:?} lacks the pinned error.txt substring {want:?}",
                            e.to_string()
                        ));
                    }
                }
                continue;
            }
        };

        // A well-formed case MUST NOT be mislabeled as a decline: if it parses, it needs a golden.
        // tree.sexp — the structural parse-tree golden.
        let want_tree = tree_golden(&arena);
        if bless {
            std::fs::write(&tree_path, &want_tree).expect("write tree.sexp");
        } else {
            match std::fs::read(&tree_path) {
                Ok(have) if have == want_tree => {}
                Ok(_) => failures.push(format!(
                    "{label}: tree.sexp mismatch — render_sexpr(read(input)) differs from the golden \
                     (re-bless with CDZ_BLESS=1)"
                )),
                Err(_) => failures.push(format!("{label}: tree.sexp is missing (bless it)")),
            }
        }

        // format.<ext> (or input-is-canonical). The format golden is in the input's own surface.
        let format_path = case.join(format!("format.{ext}"));
        let formatted = match fmt_bytes(&input, surface) {
            Ok(b) => b,
            Err(e) => {
                failures.push(format!("{label}: fmt failed ({e})"));
                continue;
            }
        };
        if bless {
            if formatted == input {
                // Already canonical — no format file, and shed a stale one.
                let _ = std::fs::remove_file(&format_path);
            } else {
                std::fs::write(&format_path, &formatted).expect("write format golden");
            }
        } else if format_path.exists() {
            match std::fs::read(&format_path) {
                Ok(have) if have == formatted => {}
                _ => failures.push(format!(
                    "{label}: format.{ext} mismatch — fmt(input) differs from the golden (re-bless)"
                )),
            }
        } else if formatted != input {
            failures.push(format!(
                "{label}: input is NOT canonical yet has no format.{ext} — either make input canonical \
                 or bless a format golden (fmt(input) != input)"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "syntax corpus self-consistency failures ({}):\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

/// Surface-independence gate (DESIGN-parser-test-corpus.md §2): a case named `<NN>-<slug>` present in
/// MORE THAN ONE surface (`ml/05-list-literal` + `sexp/09-list-literal`, matched by SLUG — the ordinal
/// may differ) is a parity twin, and all six surfaces build the *same* `cadenza-ast` arena, so their
/// `tree.sexp` goldens MUST be byte-identical. This is the corpus's headline property made a GATE, not
/// just auditable-by-inspection: a reader change that made one surface parse a construct differently
/// from another would red here. (Distinct slugs across surfaces are non-parity by construction and are
/// not compared.)
#[test]
fn matching_slug_cases_are_surface_independent() {
    let root = corpus_root();
    if !root.is_dir() {
        eprintln!(
            "syntax_corpus: SKIP — corpus root {} absent",
            root.display()
        );
        return;
    }
    // slug (dir name minus the leading `NN-`) → list of (surface/name, tree.sexp bytes).
    let mut by_slug: std::collections::BTreeMap<String, Vec<(String, Vec<u8>)>> =
        std::collections::BTreeMap::new();
    for case in enumerate_cases(&root) {
        let name = case.file_name().and_then(|n| n.to_str()).unwrap_or("");
        // Strip a leading `NN-` ordinal to get the slug.
        let slug = name
            .split_once('-')
            .map(|(_, rest)| rest)
            .unwrap_or(name)
            .to_string();
        let label = case
            .strip_prefix(&root)
            .unwrap_or(&case)
            .display()
            .to_string();
        // A decline case carries no tree.sexp — skip (nothing to compare).
        if let Ok(tree) = std::fs::read(case.join("tree.sexp")) {
            by_slug.entry(slug).or_default().push((label, tree));
        }
    }
    let mut failures: Vec<String> = Vec::new();
    for (slug, cases) in &by_slug {
        if cases.len() < 2 {
            continue; // single-surface case — no parity twin to compare
        }
        let (ref_label, ref_tree) = &cases[0];
        for (label, tree) in &cases[1..] {
            if tree != ref_tree {
                failures.push(format!(
                    "slug {slug:?}: {label} and {ref_label} have DIFFERENT tree.sexp — same-slug cases \
                     across surfaces must be surface-independent (identical arenas). Rename one to a \
                     distinct slug if they are intentionally different constructs."
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "surface-independence failures ({}):\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}
