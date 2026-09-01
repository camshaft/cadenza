//! `xtask-lint-emoji` — the EMOJI-BAN source lint (operator directive 2026-08-07: "ban emojis in the
//! codebase"). Fails if any emoji/pictographic/dingbat char appears in an `implementation/**/*.rs` source
//! COMMENT (comment-scoped + Unicode-test-doc-excluded, so functional emoji in string/char literals are
//! left alone). Carved out of the xtask monolith into its own crate (v-xtask-decompose); the detector is
//! `xtask_support::emoji_free_lint`, the SAME predicate the omnibus `check` + dev-gate warn step call, so
//! there is one source of truth. The repo root comes from `CDZ_REPO_ROOT` (else cwd) — the `apps.lint-emoji`
//! wrapper sets it to the invoking worktree, so the relocated nix-built bin lints the right tree.

fn main() {
    let repo = xtask_support::repo_root();
    match xtask_support::emoji_free_lint(&repo) {
        Ok(()) => println!("lint-emoji: ok — no emoji in source comments"),
        Err(msg) => {
            eprintln!("lint-emoji: {msg}");
            std::process::exit(1);
        }
    }
}
