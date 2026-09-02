//! The corpus-case-title set: which titles the corpus actually defines.
//!
//! A `.gate-baseline` line is `<verdict>\t<title>`; a title present in a baseline but absent from the
//! corpus is a VANISHED ORPHAN (the git union merge-driver re-adds a stale old-title line after a
//! retitle → the gate reds; see the vertical-log note). Both the `fleet-baseline` merge-driver and
//! `cdz-corpus vanished-check --prune` decide "prune this baseline line?" against the set this crate
//! returns. So the set MUST be:
//!
//! * **AST-based, never grep** — the title is the first string child of a top-level `(case "…" …)` /
//!   `(platform-case "…" …)` form, extracted through the sexpr reader; robust to reflow/comments/nesting.
//! * **FAIL-OPEN** — pruning is DELETION. [`corpus_case_titles`] returns `Some(set)` ONLY when it is
//!   confident it read the WHOLE corpus: every path readable, every file parsed, and at least one case
//!   found. On ANY doubt — empty path list, an unreadable file, a parse error, or a zero-case result —
//!   it returns `None` so the caller SKIPS the prune. Pruning against a partial or empty set would strip
//!   live baseline lines; never do that.

use std::collections::BTreeSet;
use std::path::Path;

use cadenza_syntax_core::ast::{Arenas, Leaf, Struct, StructId};
use cadenza_syntax_sexpr::read_all;

/// The top-level heads whose first string child is a corpus case title.
const CASE_HEADS: [&str; 2] = ["case", "platform-case"];

/// The set of case titles defined in one corpus file's `text`. `None` iff the text does not parse as
/// s-expressions (the fail-open signal — the caller must not prune on an unparseable file). A file that
/// parses but defines no case yields `Some(empty)`; [`corpus_case_titles`] is what folds an empty union
/// into the whole-corpus fail-open, because a single case-less fragment file is legitimate.
pub fn titles_in_text(text: &str) -> Option<BTreeSet<String>> {
    let arenas = read_all(text).ok()?;
    let top = match arenas.get(arenas.root) {
        // The reader wraps the document in a synthetic `do` head at index 0 — skip it.
        Struct::List(items) => items.get(1..).unwrap_or(&[]),
        _ => return Some(BTreeSet::new()),
    };
    let mut out = BTreeSet::new();
    for &top_id in top {
        let case_id = arenas.peel_comments(top_id);
        let is_case = arenas
            .head_name(case_id)
            .is_some_and(|h| CASE_HEADS.contains(&h));
        if !is_case {
            continue;
        }
        if let Struct::List(items) = arenas.get(case_id) {
            // The title is the first child after the head: `(case "TITLE" …)`.
            if let Some(title) = items
                .get(1)
                .and_then(|&id| string_leaf(&arenas, arenas.peel_comments(id)))
            {
                out.insert(title);
            }
        }
    }
    Some(out)
}

/// The union of case titles across every corpus file in `paths`, or `None` to signal FAIL-OPEN (skip
/// the prune). `None` when `paths` is empty, any file is unreadable, any file fails to parse, or the
/// union is empty — see the module docs. `Some(set)` is returned only when the whole corpus was read
/// and defines at least one case, so a caller may safely treat a baseline title outside `set` as an
/// orphan to prune.
pub fn corpus_case_titles<P: AsRef<Path>>(paths: &[P]) -> Option<BTreeSet<String>> {
    if paths.is_empty() {
        return None;
    }
    let mut all = BTreeSet::new();
    for p in paths {
        // Any unreadable file OR parse error -> fail-open: we no longer know the full corpus.
        let text = std::fs::read_to_string(p.as_ref()).ok()?;
        all.extend(titles_in_text(&text)?);
    }
    // A non-empty path list that yields zero titles means the corpus glob matched the wrong thing (or a
    // format drift the reader silently tolerated) — pruning against it would strip EVERY baseline line.
    if all.is_empty() { None } else { Some(all) }
}

/// The string value of an atom leaf, or `None` for a non-string / non-atom node.
fn string_leaf(a: &Arenas, id: StructId) -> Option<String> {
    match a.get(id) {
        Struct::Atom(l) => match a.leaf(*l) {
            Leaf::Str(s) => Some(s.to_string()),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_case_and_platform_case_titles_skips_other_heads() {
        let text = r#"
            (case "alpha the first" (program 1))
            (note "not a case")
            (platform-case "beta the platform" (kickoff a))
            (case "gamma the third" (program 3))
        "#;
        let got = titles_in_text(text).unwrap();
        let want: BTreeSet<String> = ["alpha the first", "beta the platform", "gamma the third"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn peels_a_comment_wrapped_case() {
        // A leading line comment wraps the case form; the title must still be found.
        let text = "; a documented case\n(case \"wrapped title\" (program 1))\n";
        let got = titles_in_text(text).unwrap();
        assert!(got.contains("wrapped title"), "got {got:?}");
    }

    #[test]
    fn a_parse_error_is_fail_open_none() {
        // Unbalanced paren -> not parseable -> None (the fail-open signal), not an empty set.
        assert!(titles_in_text("(case \"x\"").is_none());
    }

    #[test]
    fn a_case_less_file_parses_to_an_empty_set() {
        // Legitimate for a single fragment; the whole-corpus fold is what fails open on empty.
        assert_eq!(
            titles_in_text("(note \"nothing\")").unwrap(),
            BTreeSet::new()
        );
    }

    #[test]
    fn corpus_titles_fails_open_on_empty_path_list() {
        let empty: &[&Path] = &[];
        assert!(corpus_case_titles(empty).is_none());
    }

    #[test]
    fn corpus_titles_fails_open_on_unreadable_file() {
        let missing = Path::new("/nonexistent/corpus/does-not-exist.sexp");
        assert!(corpus_case_titles(&[missing]).is_none());
    }

    #[test]
    fn corpus_titles_fails_open_when_union_is_empty() {
        let dir = std::env::temp_dir();
        let p = dir.join(format!("cct-empty-{}.sexp", std::process::id()));
        std::fs::write(&p, "(note \"no cases here\")\n").unwrap();
        let got = corpus_case_titles(std::slice::from_ref(&p));
        let _ = std::fs::remove_file(&p);
        assert!(
            got.is_none(),
            "a case-less corpus must fail open, got {got:?}"
        );
    }

    #[test]
    fn corpus_titles_unions_across_files() {
        let dir = std::env::temp_dir();
        let pid = std::process::id();
        let a = dir.join(format!("cct-a-{pid}.sexp"));
        let b = dir.join(format!("cct-b-{pid}.sexp"));
        std::fs::write(&a, "(case \"from a\" (program 1))\n").unwrap();
        std::fs::write(&b, "(case \"from b\" (program 2))\n").unwrap();
        let got = corpus_case_titles(&[a.clone(), b.clone()]);
        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
        let got = got.unwrap();
        assert!(
            got.contains("from a") && got.contains("from b"),
            "got {got:?}"
        );
    }
}
