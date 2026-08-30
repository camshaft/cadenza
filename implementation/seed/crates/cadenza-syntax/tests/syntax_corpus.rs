//! Self-consistency check for the `spec/syntax/` parser/printer golden corpus (Increment 2 of
//! DESIGN-parser-test-corpus.md). Each case is a directory holding `input.<ext>` (the surface source),
//! `tree.sexp` (the STRUCTURAL parse-tree golden — comments expanded to `(comment …)` nodes, produced by
//! `sexpr::render_sexpr`, DESIGN §2), and an OPTIONAL `format.<ext>` (the canonical-format golden, DESIGN
//! §3). This test enforces, against the REFERENCE `cadenza-syntax` reader/printer, that:
//!
//!   render_sexpr(read(input))                     == tree.sexp                         (bytes)
//!   fmt(input)                                     == format.<ext>  (or input if absent) (bytes)
//!
//! It is the Increment-2 gate — a bootstrapping self-consistency check, no new harness. Increment 3 adds
//! the real syntax grader + per-case nix derivations (the authoritative, cached gate). Because this test
//! reads files from the repo's `spec/` tree (which a crate-scoped nix test sandbox may not carry), it
//! SKIPS cleanly (loud, not silent) when the corpus root is absent, so it is a full check in a real
//! checkout / dev-gate and a no-op where the tree is unavailable.
//!
//! Regenerate the goldens after editing an `input.<ext>` (or adding a case) with:
//!   CDZ_BLESS=1 cargo test -p cadenza-syntax --test syntax_corpus
//! Bless writes `tree.sexp` for every case and writes `format.<ext>` only for a case whose input is NOT
//! already canonical (removing a stale one where the input became canonical), so a clean case stays
//! minimal (no redundant format file) per DESIGN §3.

use std::path::{Path, PathBuf};

use cadenza_syntax::convert::{self, Format, Options};
use cadenza_syntax::sexpr;

/// The corpus root: `spec/syntax/`, relative to this crate's manifest dir (crate → crates → seed →
/// implementation → repo root).
fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../spec/syntax")
}

/// `render_sexpr(arena)` with the corpus's one-trailing-newline convention — the `tree.sexp` golden.
fn tree_golden(arena: &cadenza_syntax::ast::Arenas) -> Vec<u8> {
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

        // Read the surface into the arena. A malformed input (a decline) is out of scope for Increment 2
        // (it is Increment 4's `Todo` path) — report it so we don't author one here by accident.
        let arena = match convert::read(&input, surface) {
            Ok(a) => a,
            Err(e) => {
                failures.push(format!(
                    "{label}: input does not parse ({e}) — declines are Increment 4"
                ));
                continue;
            }
        };

        // tree.sexp — the structural parse-tree golden.
        let tree_path = case.join("tree.sexp");
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
        let ext = input_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
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
