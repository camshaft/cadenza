//! `xtask-prune-baselines` — prune UNREFERENCED entries from every `.gate-baseline*` file: read the
//! corpus's current title set, then drop any `verdict\ttitle` line whose title is no longer in the corpus
//! (a renamed/removed case). `--check` only LISTS what would be pruned and exits non-zero if anything is
//! stale (a cheap CI/preview pass); otherwise it rewrites each file that had stale entries. All three
//! backend baselines are pruned against the same corpus set (keeping them title-set-agreeing).
//!
//! Carved out of the xtask monolith into its own crate (v-xtask-decompose). The corpus titles come from
//! the nix-built `cdz-corpus` (from `CDZ_SEED_BIN_DIR`, via `xtask_support::read_corpus`) — no cargo build
//! of the toolchain; the repo root from `CDZ_REPO_ROOT` (else cwd). The pure prune core
//! (`prune_baseline_text`) lives here (prune-only, not shared).
//!
//! Usage: `xtask-prune-baselines [--check]`.

use std::collections::BTreeSet;
use xtask_support::{default_corpus_files, read_corpus};

fn main() {
    let check = std::env::args_os().any(|a| a == "--check");

    let repo = xtask_support::repo_root();
    // The nix-built cdz-corpus (records extractor), from the dir the app wrapper points CDZ_SEED_BIN_DIR
    // at. Falls back to `<repo>/target/debug` for a bare `cargo run -p xtask-prune-baselines` (dev).
    let bin_dir = xtask_support::seed_bin_dir(&repo);
    let cdz_corpus = bin_dir.join("cdz-corpus");

    // The corpus's current title set: each record's `description` is the `case\t<title>` line the baseline
    // writes as its data line's title half, so a baseline title matches iff the raw strings are equal.
    let mut corpus: BTreeSet<String> = BTreeSet::new();
    for file in default_corpus_files(&repo) {
        for rec in read_corpus(&cdz_corpus, &file) {
            corpus.insert(rec.description);
        }
    }

    // SAFETY GUARD: an EMPTY corpus title set is a broken read (no `.sexp` files found, a wrong CWD, or
    // cdz-corpus emitting nothing) — NOT a legitimate "every case vanished". Pruning against it would treat
    // EVERY baseline entry as unreferenced and DELETE the entire regression baseline, silently destroying
    // fleet-wide coverage. There is never a real reason to prune against an empty corpus, so REFUSE rather
    // than mass-prune. `--check` refuses too — a preview that reports "prune everything" is the same broken signal.
    if corpus.is_empty() {
        eprintln!(
            "prune-baselines: REFUSING — the corpus title set is EMPTY (cdz-corpus produced no cases from \
             spec/semantics). That is a broken read (wrong dir / no .sexp / build issue), not a reason to \
             prune every baseline entry — doing so would wipe the whole regression baseline. Fix the \
             corpus read and re-run."
        );
        std::process::exit(2);
    }

    let mut report: Vec<String> = Vec::new();
    let mut total_pruned = 0usize;
    for rel in xtask_support::BASELINE_REL {
        let path = repo.join(rel);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                eprintln!("prune-baselines: cannot read {}: {e}", path.display());
                std::process::exit(2);
            }
        };
        let (pruned_text, pruned) = prune_baseline_text(&text, &corpus);
        if pruned.is_empty() {
            continue;
        }
        total_pruned += pruned.len();
        report.push(format!(
            "{}: {} unreferenced entr{}:\n    {}",
            path.display(),
            pruned.len(),
            if pruned.len() == 1 { "y" } else { "ies" },
            pruned.join("\n    ")
        ));
        if !check {
            std::fs::write(&path, pruned_text).expect("write pruned baseline");
        }
    }

    if report.is_empty() {
        println!(
            "prune-baselines: ok — no unreferenced baseline entries (every title is in the corpus)."
        );
        return;
    }
    if check {
        eprintln!(
            "prune-baselines --check: {total_pruned} unreferenced baseline entr{} (title absent from the \
             corpus — a renamed/removed case). Run `cargo xtask prune-baselines` to delete:\n  {}",
            if total_pruned == 1 { "y" } else { "ies" },
            report.join("\n  ")
        );
        std::process::exit(1);
    }
    println!(
        "prune-baselines: pruned {total_pruned} unreferenced baseline entr{}:\n  {}",
        if total_pruned == 1 { "y" } else { "ies" },
        report.join("\n  ")
    );
}

/// Pure core of the baseline prune: given a baseline file's text and the set of titles the corpus
/// currently defines, return the text with every UNREFERENCED `verdict\ttitle` data line removed (its
/// title absent from `corpus`), plus the pruned titles (sorted). Header/comment/blank lines and every
/// still-referenced data line are preserved VERBATIM in order, so an already-canonical (sorted) baseline
/// stays canonical — the prune only DELETES, it never reorders or rewrites a kept line. A line that isn't
/// `something\ttitle` is treated as structure and kept (we only drop lines we positively identify as an
/// unreferenced data line — never eat a line we don't understand).
fn prune_baseline_text(
    text: &str,
    corpus: &std::collections::BTreeSet<String>,
) -> (String, Vec<String>) {
    let mut kept: Vec<&str> = Vec::new();
    let mut pruned: Vec<String> = Vec::new();
    for line in text.lines() {
        // Drop ONLY a line we positively identify as an unreferenced `verdict\ttitle` data line; a
        // header/comment/blank line, or any line whose title is still in the corpus, is kept verbatim.
        if !line.starts_with('#')
            && !line.is_empty()
            && let Some((_verdict, title)) = line.split_once('\t')
            && !corpus.contains(title)
        {
            pruned.push(title.to_string());
            continue;
        }
        kept.push(line);
    }
    let mut out = kept.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    pruned.sort();
    (out, pruned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_baseline_text_drops_unreferenced_titles_and_keeps_structure() {
        // The lm5-rename shape: an OLD title (no #3756) lingers next to its RENAMED twin. Only the title
        // absent from the corpus is pruned; the header, blank lines, and every referenced line survive
        // VERBATIM in order (a canonical/sorted file stays canonical).
        let text = "# gate baseline\npass\tlm3 old title\npass\tlm5 new title (#3756)\npass\tlm5 old title\nfail\tmsr6 old title\n";
        let corpus: std::collections::BTreeSet<String> =
            ["lm5 new title (#3756)".to_string()].into_iter().collect();
        let (out, pruned) = prune_baseline_text(text, &corpus);
        assert_eq!(
            pruned,
            vec![
                "lm3 old title".to_string(),
                "lm5 old title".to_string(),
                "msr6 old title".to_string(),
            ],
            "every title absent from the corpus is pruned (sorted)"
        );
        assert_eq!(
            out, "# gate baseline\npass\tlm5 new title (#3756)\n",
            "header + the one referenced line survive verbatim; trailing newline preserved"
        );
    }

    #[test]
    fn prune_baseline_text_is_a_noop_when_every_title_is_referenced() {
        let text = "# h\npass\ta\ntodo\tb\n";
        let corpus: std::collections::BTreeSet<String> =
            ["a".to_string(), "b".to_string()].into_iter().collect();
        let (out, pruned) = prune_baseline_text(text, &corpus);
        assert!(pruned.is_empty(), "nothing pruned when all referenced");
        assert_eq!(
            out, text,
            "a fully-referenced file is returned byte-identical"
        );
    }

    #[test]
    fn prune_baseline_text_never_drops_a_line_it_cannot_parse() {
        // A line with no tab is structure we don't model — keep it (we only delete lines we positively
        // identify as an unreferenced `verdict\ttitle` data line), mirroring the canonicalizer's hands-off.
        let text = "# h\nno-tab structure line\npass\tdead\n";
        let corpus = std::collections::BTreeSet::new(); // corpus empty → `dead` is unreferenced
        let (out, pruned) = prune_baseline_text(text, &corpus);
        assert_eq!(pruned, vec!["dead".to_string()]);
        assert_eq!(
            out, "# h\nno-tab structure line\n",
            "the tab-less line is kept; only the identified data line is dropped"
        );
    }

    #[test]
    fn prune_baseline_text_with_empty_corpus_prunes_everything_which_is_why_the_driver_guards() {
        // WHY the driver REFUSES an empty corpus title set: the pure core has no way to know an empty corpus
        // is a BROKEN READ rather than "every case vanished", so with no referenced titles EVERY data line
        // is unreferenced and gets dropped — wiping the baseline down to its header. The empty-corpus guard
        // lives in the driver precisely to prevent this; this test pins the behavior that motivates it.
        let text = "# gate baseline\npass\ta\ntodo\tb\nfail\tc\n";
        let empty = std::collections::BTreeSet::new();
        let (out, pruned) = prune_baseline_text(text, &empty);
        assert_eq!(
            pruned,
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
            "an empty corpus makes every entry unreferenced"
        );
        assert_eq!(
            out, "# gate baseline\n",
            "everything but the header is pruned"
        );
    }
}
