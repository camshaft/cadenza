//! `corpus-chapter-gate` — gate a corpus chapter across EVERY execution target that shares its
//! `spec/semantics/.gate-baseline`, reddening if any one reds.
//!
//! One corpus chapter drives four checks that all grade against the SAME baseline: the wasm corpus
//! (`corpus-<stem>`), the cadenza re-emit (`corpus-cadenza-<stem>`), and the rust + rust-async backends
//! (`corpus-rust-<stem>` / `corpus-rust-async-<stem>`). Only the wasm one is a `gate-local` constituent;
//! the other three are advisory/nightly. So a wasm-only pass can mask a re-emit or rust trap on a
//! baseline-`pass` case, and that trap lands latent on main — exactly what SHAPE 342 did (#346). This
//! helper runs all four in ONE `nix build` (which fails if any attr reds), so "gate the chapter" means
//! every baseline-sharing target, not just wasm.

use crate::Paths;
use std::process::Command;
use xtask_support::default_corpus_files;

/// The four baseline-sharing check attrs for one corpus stem, in gate order: the wasm corpus, the cadenza
/// re-emit, then the rust + rust-async backends. All grade against the same `spec/semantics/.gate-baseline`,
/// so gating all four is what stops a wasm-only pass from masking a re-emit/rust trap (#346). Pure.
pub(crate) fn chapter_target_attrs(stem: &str) -> Vec<String> {
    vec![
        format!("corpus-{stem}"),
        format!("corpus-cadenza-{stem}"),
        format!("corpus-rust-{stem}"),
        format!("corpus-rust-async-{stem}"),
    ]
}

/// Resolve `<chapter>` to the corpus stem(s) it gates, given the `available` corpus stems. An exact stem
/// match resolves to just that stem (the canonical form). A BARE chapter number (all ASCII digits, e.g.
/// `28`) EXPANDS to every stem in that chapter (see [`stem_in_chapter`]) — so `28` gates BOTH
/// `28-compiler-primitives` and `28-wit-abi-boundary`, and `14` gates `14`/`14b`/`14c` — erring toward
/// gating MORE, since an under-gate is the exact bug #346 closes. Any other input is unknown. Returns the
/// matched stems sorted (deterministic output), or an `Err` message listing the available stems. Pure —
/// unit-tested.
pub(crate) fn resolve_stems(chapter: &str, available: &[String]) -> Result<Vec<String>, String> {
    // (1) exact stem match — the canonical form, gates just that one stem.
    if available.iter().any(|s| s == chapter) {
        return Ok(vec![chapter.to_string()]);
    }
    // (2) a bare chapter number expands to every stem in that chapter.
    if !chapter.is_empty() && chapter.bytes().all(|b| b.is_ascii_digit()) {
        let mut matched: Vec<String> = available
            .iter()
            .filter(|s| stem_in_chapter(s, chapter))
            .cloned()
            .collect();
        matched.sort();
        if matched.is_empty() {
            return Err(format!(
                "no corpus chapter matches the number '{chapter}'. Available stems: {}",
                available.join(", ")
            ));
        }
        return Ok(matched);
    }
    // (3) neither a stem nor a bare number.
    Err(format!(
        "'{chapter}' is neither a corpus stem nor a bare chapter number — pass a full stem \
         (e.g. 28-wit-abi-boundary) or a chapter number (e.g. 28). Available stems: {}",
        available.join(", ")
    ))
}

/// Does `stem` belong to the bare chapter `number`? True iff `stem` begins with the digits `number` and
/// the very next character is `-` (e.g. `28-…`) or a lowercase-ASCII variant letter (e.g. `14b-…`). This
/// respects the digit boundary — `2` does NOT match `28-…`, and `14` does NOT match `140-…` — while still
/// catching the `14b`/`14c` letter variants of chapter 14. Pure.
fn stem_in_chapter(stem: &str, number: &str) -> bool {
    let Some(rest) = stem.strip_prefix(number) else {
        return false;
    };
    match rest.bytes().next() {
        Some(b'-') => true,
        Some(b) => b.is_ascii_lowercase(),
        None => false, // a bare number with no `-label` is not a real corpus stem
    }
}

/// The corpus stems available in this repo — the `spec/semantics/*.sexp` file stems, the exact source the
/// nix per-chapter checks are generated from.
fn available_stems(paths: &Paths) -> Vec<String> {
    default_corpus_files(&paths.repo)
        .iter()
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(str::to_string))
        .collect()
}

/// The current nix system double (e.g. `aarch64-linux`), used to address `.#checks.<system>.<attr>`.
/// Falls back to `aarch64-linux` if nix can't be run.
fn current_system() -> String {
    Command::new("nix")
        .args([
            "eval",
            "--raw",
            "--impure",
            "--expr",
            "builtins.currentSystem",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "aarch64-linux".to_string())
}

/// Run the chapter gate: resolve `<chapter>` to its stem(s), PRINT the resolved list up front (so an
/// expansion is never silent), then one `nix build --no-link` over every stem's four baseline-sharing
/// check attrs. A multi-attr `nix build` fails if ANY attr reds — the natural fan-out. When
/// `with_lib_tests` is set and the corpus build is GREEN, it then runs `cargo test -p rcdzc` — the guard
/// a decline-flip land needs, since the four corpus attrs never build the rcdzc lib tests where a stale
/// "must-decline" `#[test]` lives (#982). This process exits with the first RED stage's code. Never returns.
pub(crate) fn run(paths: &Paths, chapter: &str, with_lib_tests: bool) -> ! {
    let available = available_stems(paths);
    let stems = match resolve_stems(chapter, &available) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("xtask corpus-chapter-gate: {e}");
            std::process::exit(2);
        }
    };
    let system = current_system();
    eprintln!(
        "corpus-chapter-gate {chapter} -> gating {} chapter(s): {} \
         [x4 targets each: corpus-/corpus-cadenza-/corpus-rust-/corpus-rust-async-<stem>]{}",
        stems.len(),
        stems.join(", "),
        if with_lib_tests {
            " + cargo test -p rcdzc"
        } else {
            ""
        }
    );
    let attrs: Vec<String> = stems.iter().flat_map(|s| chapter_target_attrs(s)).collect();
    let mut build_args: Vec<String> = vec!["build".to_string(), "--no-link".to_string()];
    build_args.extend(attrs.iter().map(|a| format!(".#checks.{system}.{a}")));
    eprintln!("+ nix {}", build_args.join(" "));
    let status = Command::new("nix")
        .args(&build_args)
        .status()
        .unwrap_or_else(|e| {
            eprintln!("xtask corpus-chapter-gate: could not invoke `nix build`: {e}");
            std::process::exit(1);
        });
    // A red corpus build is the whole result — exit now, before the (slower) lib-test stage.
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    if with_lib_tests {
        eprintln!("+ cargo test -p rcdzc");
        let test_status = Command::new("cargo")
            .args(["test", "-p", "rcdzc"])
            .current_dir(&paths.repo)
            .status()
            .unwrap_or_else(|e| {
                eprintln!("xtask corpus-chapter-gate: could not invoke `cargo test -p rcdzc`: {e}");
                std::process::exit(1);
            });
        std::process::exit(test_status.code().unwrap_or(1));
    }
    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<String> {
        [
            "05-compound-types",
            "14-effects-and-handlers",
            "14b-effects-and-handlers",
            "14c-effects-and-handlers",
            "25-verification",
            "25-verification-neighbors",
            "28-compiler-primitives",
            "28-wit-abi-boundary",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    #[test]
    fn chapter_target_attrs_fans_the_four_baseline_sharing_targets() {
        // The wasm/cadenza/rust/rust-async targets that share one .gate-baseline — the set #346 requires.
        assert_eq!(
            chapter_target_attrs("28-wit-abi-boundary"),
            vec![
                "corpus-28-wit-abi-boundary",
                "corpus-cadenza-28-wit-abi-boundary",
                "corpus-rust-28-wit-abi-boundary",
                "corpus-rust-async-28-wit-abi-boundary",
            ]
        );
    }

    #[test]
    fn resolve_stems_exact_stem_is_canonical() {
        assert_eq!(
            resolve_stems("28-wit-abi-boundary", &sample()).unwrap(),
            vec!["28-wit-abi-boundary".to_string()]
        );
    }

    #[test]
    fn resolve_stems_bare_number_expands_to_every_stem_in_the_chapter() {
        // 28 gates BOTH 28-* stems — the exact ambiguity #346 is about (a wasm-only gate of one masks the other).
        assert_eq!(
            resolve_stems("28", &sample()).unwrap(),
            vec![
                "28-compiler-primitives".to_string(),
                "28-wit-abi-boundary".to_string(),
            ]
        );
        // 14 pulls in the b/c letter-variant chapters.
        assert_eq!(
            resolve_stems("14", &sample()).unwrap(),
            vec![
                "14-effects-and-handlers".to_string(),
                "14b-effects-and-handlers".to_string(),
                "14c-effects-and-handlers".to_string(),
            ]
        );
        // 25 -> both verification stems.
        assert_eq!(
            resolve_stems("25", &sample()).unwrap(),
            vec![
                "25-verification".to_string(),
                "25-verification-neighbors".to_string(),
            ]
        );
    }

    #[test]
    fn resolve_stems_respects_the_digit_boundary() {
        // `2` must NOT match `28-…`, and `1` must NOT match `14…` — a number is a whole chapter, not a prefix.
        assert!(resolve_stems("2", &sample()).is_err());
        assert!(resolve_stems("1", &sample()).is_err());
    }

    #[test]
    fn resolve_stems_unknown_input_errs() {
        assert!(resolve_stems("nope", &sample()).is_err());
        assert!(resolve_stems("14b", &sample()).is_err()); // a label prefix, not a full stem nor a bare number
        assert!(resolve_stems("99", &sample()).is_err()); // no such chapter
        assert!(resolve_stems("", &sample()).is_err());
    }

    #[test]
    fn stem_in_chapter_boundaries() {
        assert!(stem_in_chapter("28-wit-abi-boundary", "28"));
        assert!(stem_in_chapter("14b-effects-and-handlers", "14"));
        assert!(!stem_in_chapter("140-foo", "14")); // trailing digit — different chapter number
        assert!(!stem_in_chapter("28-x", "2")); // `2` is not chapter `28`
    }
}
