//! `xtask-codegen-unsupported` — generate/check `data/unsupported.sexp`, the repo-root registry of every
//! construct rcdzc declines to compile, from the compiler's `DeclineId` catalog (the oracle in
//! `rcdzc/src/diag.rs`). Increment 2 of the unsupported-error tracker
//! (`implementation/design/DESIGN-unsupported-tracker.md`, operator seq-286-broad).
//!
//! Iterates `rcdzc::diag::DeclineId::ALL` (structurally complete — completeness is not scraped) and writes
//! one `(unsupported <key> …)` form per decline REASON. The COMPILER-DERIVED fields (`code`, `reason`) come
//! from the catalog; the human-authored `(blocked-on …)` routing block is PRESERVED verbatim across
//! regenerations (keyed by the stable `<key>`), so re-running codegen never clobbers a triage. A freshly
//! minted id with no entry yet gets a default `(blocked-on (status unowned))` — the backlog to triage.
//!
//! Modes: `cargo run -p xtask-codegen-unsupported` writes the file; `--check` compares without writing and
//! exits non-zero on drift (a new untracked id, a dead entry, or a changed code/reason) — the staleness
//! gate wired into `xtask check`.

use rcdzc::diag::DeclineId;
use std::path::PathBuf;

fn main() {
    let check = std::env::args().any(|a| a == "--check");
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("resolve repo root from crate manifest dir");
    let out = repo_root.join("data/unsupported.sexp");

    let existing = std::fs::read_to_string(&out).unwrap_or_default();
    let source = render(&existing);

    if check {
        if existing != source {
            eprintln!(
                "xtask-codegen-unsupported --check: {} is OUT OF DATE with the DeclineId catalog.\n  \
                 A decline id was added/removed or its code/reason changed without regenerating the \
                 registry.\n  Fix: run `cargo run -p xtask-codegen-unsupported` and commit {}.",
                out.display(),
                out.display()
            );
            std::process::exit(1);
        }
        println!(
            "xtask-codegen-unsupported --check: {} is up to date.",
            out.display()
        );
        return;
    }
    if let Err(e) = std::fs::write(&out, &source) {
        eprintln!("xtask-codegen-unsupported: writing {}: {e}", out.display());
        std::process::exit(1);
    }
    println!(
        "xtask-codegen-unsupported: wrote {} ({} decline ids)",
        out.display(),
        DeclineId::ALL.len()
    );
}

/// Build the full registry text. `existing` is the current file (empty on first emit) — its per-key
/// `(blocked-on …)` blocks are preserved.
fn render(existing: &str) -> String {
    let mut s = String::new();
    s.push_str(HEADER);
    s.push_str("(do\n");
    for &id in DeclineId::ALL {
        let key = id.key();
        let code = id.code().map(|c| c.code()).unwrap_or("none");
        let reason = escape(id.reason());
        let blocked_on = preserved_blocked_on(existing, key)
            .unwrap_or_else(|| "(blocked-on (status unowned))".to_string());
        s.push_str(&format!(
            "  (unsupported {key}\n    (code {code})\n    (reason \"{reason}\")\n    {blocked_on})\n"
        ));
    }
    s.push_str(")\n");
    s
}

/// Escape a reason string for a double-quoted sexpr literal. Reasons are compiler-authored ASCII prose;
/// escape the two chars that would break the literal.
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Extract the verbatim `(blocked-on …)` block for `key` from the existing registry text, paren-matched.
/// `None` if the key is absent (a freshly minted id) or has no block. This is the merge-preserve: the
/// human-authored routing survives a regen that only refreshes the derived `code`/`reason`.
fn preserved_blocked_on(existing: &str, key: &str) -> Option<String> {
    // Find the `(unsupported <key>` form, then the first `(blocked-on` at/after it, then paren-match.
    let anchor = format!("(unsupported {key}\n");
    let form_start = existing.find(&anchor)?;
    let bo_rel = existing[form_start..].find("(blocked-on")?;
    let bo_start = form_start + bo_rel;
    let bytes = existing.as_bytes();
    let mut depth = 0usize;
    let mut i = bo_start;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(existing[bo_start..=i].to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }
    None // unbalanced (should not happen in a committed, gated file)
}

const HEADER: &str = "\
; data/unsupported.sexp — the auto-generated registry of every construct rcdzc declines to compile.
; GENERATED from the DeclineId catalog (rcdzc/src/diag.rs) by `cargo run -p xtask-codegen-unsupported`.
; The (code …) and (reason …) fields are DERIVED — do NOT hand-edit them (a `codegen --check` gate reds
; on drift). The (blocked-on …) block IS hand-authored (status/owner/needs/ref) and is PRESERVED across
; regenerations — that is where triage + routing-to-owning-lanes lives. Status: blocked | in-flight |
; permanent | design-gated | unowned. (Unsupported-error tracker, operator seq-286-broad.)
";
