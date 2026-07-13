//! The finding store: shrink, deduplicate, and persist findings to `spec/semantics/failures/`.
//!
//! That directory is a queue an existing monitoring loop already watches and triages, so cdz-smith
//! writes into it directly rather than inventing its own sink. Two constraints shape this module:
//!
//! * **Dedup by crash SITE, not by program.** A fuzzer finds the same panic thousands of times with
//!   thousands of different programs. Filing each would bury the triage agent. So findings are
//!   bucketed by a stable [`signature`](Finding::signature) — the panic's `file:line` (path
//!   normalized so it's identical across worktrees/checkouts) plus a digit/hex-masked message
//!   template. One bucket per distinct bug; re-hits bump a counter in the note instead of adding
//!   files.
//! * **Minimal reproducers.** A raw generated program is large and noisy. Before filing, the store
//!   greedily [`shrink`]s it — repeatedly deleting whitespace-balanced sub-forms and retrying —
//!   down to a small program that still crashes at the same site. The triage agent gets a tight
//!   witness, not a 200-node blob.

use std::path::{Path, PathBuf};

use crate::oracle::{CrashInfo, Verdict, compile_catching};

/// What kind of finding this is. (Differential is reserved for the planned miscompile oracle.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    /// A panic escaped the compile path.
    Crash,
    /// The compiler did not finish inside the wall-clock budget.
    Timeout,
}

impl Category {
    fn tag(self) -> &'static str {
        match self {
            Category::Crash => "crash",
            Category::Timeout => "timeout",
        }
    }
}

/// A single finding, ready to persist.
#[derive(Clone, Debug)]
pub struct Finding {
    pub category: Category,
    /// The (already shrunk) reproducer source, in the runnable export shape.
    pub program: String,
    /// Crash details. `None` for a timeout (there is no panic site).
    pub crash: Option<CrashInfo>,
    /// The compiler commit the finding was produced against (short SHA, or "unknown").
    pub commit: String,
}

impl Finding {
    /// The dedup key: a filesystem-safe slug derived from the crash site + a masked message
    /// template (for a crash), or the masked message alone (for a timeout). Stable across runs and
    /// checkouts, so the same bug always lands in the same bucket.
    pub fn signature(&self) -> String {
        let raw = match (&self.crash, self.category) {
            (Some(c), _) => {
                let site = c
                    .site
                    .as_deref()
                    .map(normalize_site)
                    .unwrap_or_else(|| "nosite".into());
                format!("{site}::{}", mask_message(&c.message))
            }
            (None, Category::Timeout) => "timeout".to_string(),
            (None, _) => "unknown".to_string(),
        };
        slugify(&raw)
    }

    /// A one-line human summary for the note title.
    fn title(&self) -> String {
        match (&self.crash, self.category) {
            (Some(c), _) => {
                let site = c
                    .site
                    .as_deref()
                    .map(normalize_site)
                    .unwrap_or_else(|| "unknown site".into());
                format!("compiler panic at {site}: {}", first_line(&c.message))
            }
            (None, Category::Timeout) => {
                "compiler timeout (no result inside the budget)".to_string()
            }
            (None, _) => "compiler finding".to_string(),
        }
    }
}

/// Persists findings under a `failures/` directory, deduping by signature.
pub struct FindingStore {
    dir: PathBuf,
}

/// The outcome of filing a finding.
#[derive(Debug, PartialEq, Eq)]
pub enum Filed {
    /// A brand-new bucket was created (path to the `.md` note).
    New(PathBuf),
    /// An existing bucket for this signature was hit again (its counter was bumped).
    Duplicate(PathBuf),
}

impl FindingStore {
    /// Open (creating if needed) a store rooted at `dir` (the `failures/` directory).
    pub fn open(dir: impl Into<PathBuf>) -> std::io::Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        Ok(FindingStore { dir })
    }

    /// Locate `spec/semantics/failures/` by walking up from `start` looking for a `spec/semantics`
    /// directory (the repo layout). Falls back to `<start>/spec/semantics/failures`.
    pub fn discover(start: &Path) -> std::io::Result<Self> {
        let mut cur = Some(start);
        while let Some(dir) = cur {
            let candidate = dir.join("spec/semantics");
            if candidate.is_dir() {
                return Self::open(candidate.join("failures"));
            }
            cur = dir.parent();
        }
        Self::open(start.join("spec/semantics/failures"))
    }

    /// The directory findings are written to.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// File a finding. If a bucket for its signature exists, bump the hit counter and keep the
    /// SMALLER reproducer; otherwise create a new `<sig>.smith.sexp` + `<sig>.smith.md` pair.
    ///
    /// The `.smith` infix marks these as machine-generated (vs. the hand-written `.sexp`/`.md` in
    /// the queue), so the monitoring agent — and a human — can tell them apart at a glance, and a
    /// resolved finding can be renamed `.RESOLVED`/`.REJECTED` like the existing notes.
    pub fn file(&self, finding: &Finding) -> std::io::Result<Filed> {
        let sig = finding.signature();
        let note = self.dir.join(format!("{sig}.smith.md"));
        let repro = self.dir.join(format!("{sig}.smith.sexp"));

        if note.exists() {
            // Duplicate: bump the counter; adopt the shorter reproducer if this one is smaller.
            bump_hits(&note)?;
            if let Ok(existing) = std::fs::read_to_string(&repro)
                && finding.program.len() < existing.len()
            {
                std::fs::write(&repro, format!("{}\n", finding.program.trim_end()))?;
            }
            return Ok(Filed::Duplicate(note));
        }

        std::fs::write(&repro, format!("{}\n", finding.program.trim_end()))?;
        std::fs::write(&note, self.render_note(finding, &repro))?;
        Ok(Filed::New(note))
    }

    fn render_note(&self, finding: &Finding, repro: &Path) -> String {
        let repro_name = repro
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("<repro>");
        let mut s = String::new();
        s.push_str(&format!("# SMITH FINDING — {}\n\n", finding.title()));
        s.push_str(
            "_Filed by `cdz-smith` (the compiler fuzzer). This is an auto-generated finding: a\n",
        );
        s.push_str(
            "generated program made the compiler PANIC or HANG — never valid behavior, since the\n",
        );
        s.push_str("compiler reports every legitimate \"no\" as a diagnostic. Triage, fix, then rename this\n");
        s.push_str("file `.RESOLVED.md` (or `.REJECTED.md` with a rationale) so it is not re-triaged._\n\n");
        s.push_str(&format!("- **Category:** {}\n", finding.category.tag()));
        s.push_str(&format!("- **Compiler commit:** `{}`\n", finding.commit));
        s.push_str("- **Hits:** 1\n");
        s.push_str(&format!("- **Signature:** `{}`\n\n", finding.signature()));

        s.push_str("## Reproducer\n\n");
        s.push_str(&format!("`{repro_name}` (also inline):\n\n"));
        s.push_str("```scheme\n");
        s.push_str(finding.program.trim_end());
        s.push_str("\n```\n\n");
        s.push_str("Reproduce in-process:\n\n");
        s.push_str("```\n");
        s.push_str(&format!("cargo run -p cdz-smith -- verify {repro_name}\n"));
        s.push_str("```\n\n");

        if let Some(c) = &finding.crash {
            s.push_str("## Panic\n\n");
            if let Some(site) = &c.site {
                s.push_str(&format!("- **Site:** `{}`\n", normalize_site(site)));
            }
            s.push_str(&format!("- **Message:** {}\n\n", first_line(&c.message)));
            s.push_str("<details><summary>Backtrace</summary>\n\n```\n");
            s.push_str(c.backtrace.trim_end());
            s.push_str("\n```\n\n</details>\n");
        } else if finding.category == Category::Timeout {
            s.push_str("## Timeout\n\n");
            s.push_str(
                "The compiler did not produce a result within the wall-clock budget. This is\n",
            );
            s.push_str(
                "detected out of process (an in-process catch cannot interrupt a runaway loop).\n",
            );
        }
        s
    }
}

/// Greedily minimize a crashing program: repeatedly delete a balanced parenthesized sub-form and
/// keep the deletion whenever the result still parses AND still crashes at the SAME site. Returns
/// the smallest program found. Cheap and monotone — each accepted step strictly shrinks the source.
pub fn shrink(source: &str, target_site: Option<&str>) -> String {
    // The site we must preserve; if the original had no site, preserve "crashes at all".
    let same_site = |v: &Verdict| match (v, target_site) {
        (Verdict::Crash(c), Some(t)) => c.site.as_deref().map(normalize_site).as_deref() == Some(t),
        (Verdict::Crash(_), None) => true,
        _ => false,
    };

    let mut best = source.to_string();
    // Bound the passes so a pathological input can't loop; each pass is O(n^2) in sub-forms.
    for _ in 0..12 {
        let mut improved = false;
        // Try deleting each balanced sub-form (by paren span), largest offsets first so indices
        // stay valid as we retry on the shrunk string.
        let spans = balanced_spans(&best);
        for (lo, hi) in spans.into_iter().rev() {
            // Never delete the outermost do-form or the whole thing.
            if lo == 0 && hi == best.len() {
                continue;
            }
            let mut candidate = String::with_capacity(best.len() - (hi - lo));
            candidate.push_str(&best[..lo]);
            candidate.push_str(&best[hi..]);
            let candidate = candidate.trim().to_string();
            if candidate.len() >= best.len() {
                continue;
            }
            if same_site(&compile_catching(&candidate)) {
                best = candidate;
                improved = true;
                break; // re-derive spans on the smaller program
            }
        }
        if !improved {
            break;
        }
    }
    best
}

// ── helpers ─────────────────────────────────────────────────────────────────────────────────

/// The byte spans of every balanced `(...)` sub-form in `s` (outermost and nested).
fn balanced_spans(s: &str) -> Vec<(usize, usize)> {
    let bytes = s.as_bytes();
    let mut stack = Vec::new();
    let mut spans = Vec::new();
    let mut in_str = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => in_str = !in_str,
            b'(' if !in_str => stack.push(i),
            b')' if !in_str => {
                if let Some(lo) = stack.pop() {
                    spans.push((lo, i + 1));
                }
            }
            _ => {}
        }
        i += 1;
    }
    spans
}

/// Normalize a panic site so it is identical across worktrees/checkouts: keep only the portion from
/// the `crates/…` segment onward, dropping the absolute/worktree prefix.
pub fn normalize_site(site: &str) -> String {
    if let Some(idx) = site.find("crates/") {
        return site[idx..].to_string();
    }
    // Fall back to the last path component (file:line:col), if any.
    site.rsplit('/').next().unwrap_or(site).to_string()
}

/// Collapse the variable parts of a panic message (numbers, hex) so two messages that differ only
/// in incidental values share a template — the dedup key.
fn mask_message(msg: &str) -> String {
    let first = first_line(msg);
    let mut out = String::with_capacity(first.len());
    let mut chars = first.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            // collapse a run of digits/hex (and a leading 0x) to a single '#'
            while chars
                .peek()
                .is_some_and(|n| n.is_ascii_hexdigit() || *n == 'x')
            {
                chars.next();
            }
            out.push('#');
        } else {
            out.push(c);
        }
    }
    out
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s).trim()
}

/// Turn an arbitrary string into a filesystem-safe, bounded slug.
fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len().min(80));
    let mut last_dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
        if out.len() >= 80 {
            break;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "finding".to_string()
    } else {
        trimmed
    }
}

/// Bump the `**Hits:** N` line in an existing note.
fn bump_hits(note: &Path) -> std::io::Result<()> {
    let text = std::fs::read_to_string(note)?;
    let bumped = text
        .lines()
        .map(|line| {
            if let Some(rest) = line.strip_prefix("- **Hits:** ") {
                let n: u64 = rest.trim().parse().unwrap_or(1);
                format!("- **Hits:** {}", n + 1)
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(note, format!("{bumped}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_masking_collapses_numbers() {
        assert_eq!(
            mask_message("index 5 out of bounds"),
            "index # out of bounds"
        );
        assert_eq!(
            mask_message("index 12345 out of bounds"),
            "index # out of bounds"
        );
        // Two messages differing only in numbers share a template.
        assert_eq!(
            mask_message("node 0x1f3a bad"),
            mask_message("node 0x9c02 bad")
        );
    }

    #[test]
    fn site_normalization_is_checkout_stable() {
        let a = "/home/x/.claude/worktrees/foo/implementation/seed/crates/rcdzc/src/lower.rs:766:9";
        let b = "/local/home/y/Projects/camshaft/cadenza/implementation/seed/crates/rcdzc/src/lower.rs:766:9";
        assert_eq!(normalize_site(a), normalize_site(b));
        assert_eq!(normalize_site(a), "crates/rcdzc/src/lower.rs:766:9");
    }

    #[test]
    fn signatures_bucket_by_site_and_template() {
        let mk = |site: &str, msg: &str| Finding {
            category: Category::Crash,
            program: "(do (def (main) 0) (export main))".into(),
            crash: Some(CrashInfo {
                site: Some(site.into()),
                message: msg.into(),
                backtrace: String::new(),
            }),
            commit: "abc123".into(),
        };
        let s1 = mk("crates/rcdzc/src/lower.rs:766:9", "position 3 not found").signature();
        let s2 = mk("crates/rcdzc/src/lower.rs:766:9", "position 99 not found").signature();
        let s3 = mk("crates/rcdzc/src/select.rs:1266:5", "position 3 not found").signature();
        assert_eq!(s1, s2, "same site + masked message → same bucket");
        assert_ne!(s1, s3, "different site → different bucket");
    }

    #[test]
    fn file_then_duplicate_bumps_hits() {
        let tmp = std::env::temp_dir().join(format!("cdz-smith-test-{}", std::process::id()));
        let store = FindingStore::open(&tmp).unwrap();
        let f = Finding {
            category: Category::Crash,
            program: "(do (def (main) 0) (export main))".into(),
            crash: Some(CrashInfo {
                site: Some("crates/rcdzc/src/x.rs:1:1".into()),
                message: "boom 7".into(),
                backtrace: String::new(),
            }),
            commit: "deadbeef".into(),
        };
        let first = store.file(&f).unwrap();
        assert!(matches!(first, Filed::New(_)));
        let second = store.file(&f).unwrap();
        assert!(matches!(second, Filed::Duplicate(_)));
        let note = std::fs::read_to_string(store.dir().join(format!("{}.smith.md", f.signature())))
            .unwrap();
        assert!(note.contains("- **Hits:** 2"), "hits should bump:\n{note}");
        std::fs::remove_dir_all(&tmp).ok();
    }
}
