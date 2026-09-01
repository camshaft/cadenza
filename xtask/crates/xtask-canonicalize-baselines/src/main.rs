//! `xtask-canonicalize-baselines` — canonicalize every `.gate-baseline*` file in place: sort + de-dup
//! verdict-aware, WITHOUT a gate run. The root-fix for the `merge=union` benign-dup re-accumulation (a
//! concurrent baseline append merges both sides' rows, re-injecting same-verdict duplicate lines that red
//! `check`'s no-dup lint fleet-wide); pr-sync runs this post-land so baselines land already-canonical, and
//! it's runnable by hand. A same-title/DIFFERENT-verdict conflict is SURFACED (exit non-zero, names the
//! titles) — never silently deduped. Writes only files that were non-canonical (an already-clean file is
//! left byte-identical, so it never dirties the worktree).
//!
//! Carved out of the xtask monolith into its own crate (v-xtask-decompose). The pure canonicalizer is
//! `xtask_support::canonicalize_baseline_text`, the SAME predicate the xtask `check` baseline-no-dup lint
//! and the `merge-baseline` git driver use, so there is one source of truth. The repo root comes from
//! `CDZ_REPO_ROOT` (else cwd) — the `apps.canonicalize-baselines` wrapper sets it to the invoking worktree.

use xtask_support::canonicalize_baseline_text;

fn main() {
    let repo = xtask_support::repo_root();

    let mut rewrote: Vec<String> = Vec::new();
    let mut conflicts: Vec<String> = Vec::new();
    for rel in xtask_support::BASELINE_REL {
        let path = repo.join(rel);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            // Absent files are skipped (rust/rust-async baselines are opt-in).
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                eprintln!(
                    "canonicalize-baselines: cannot read {}: {e}",
                    path.display()
                );
                std::process::exit(2);
            }
        };
        match canonicalize_baseline_text(&text) {
            Ok(None) => {} // already canonical — leave the file untouched
            Ok(Some(canonical)) => {
                std::fs::write(&path, canonical).expect("write canonical baseline");
                rewrote.push(path.display().to_string());
            }
            Err(titles) => {
                conflicts.push(format!(
                    "{}: {} conflicting title(s) (same case, DIFFERENT verdicts — cannot auto-dedup):\n    {}",
                    path.display(),
                    titles.len(),
                    titles.join("\n    ")
                ));
            }
        }
    }
    if !conflicts.is_empty() {
        eprintln!(
            "canonicalize-baselines: REFUSING — a same-title/different-verdict conflict is a real \
             integrity error (the map-keyed baseline would mask one via last-wins). Resolve which \
             verdict is correct (keep one line), then re-run. Conflicts:\n  {}",
            conflicts.join("\n  ")
        );
        std::process::exit(1);
    }
    if rewrote.is_empty() {
        println!(
            "canonicalize-baselines: ok — all baselines already canonical (nothing rewritten)."
        );
    } else {
        println!(
            "canonicalize-baselines: rewrote {} baseline(s) to canonical (sorted + verdict-aware deduped):\n  {}",
            rewrote.len(),
            rewrote.join("\n  ")
        );
    }
}
