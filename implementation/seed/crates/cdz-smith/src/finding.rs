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

/// What kind of finding this is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    /// A panic escaped the compile path.
    Crash,
    /// The compiler did not finish inside the wall-clock budget.
    Timeout,
    /// The compiler reported success but the emitted component failed wasm validation (a backend
    /// miscompile — structurally-invalid wasm).
    InvalidWasm,
    /// The two emit backends produced VALID artifacts that DISAGREE on the program's value (or one
    /// ran to a value where the other trapped) — a wrong-value miscompile the crash/validity oracles
    /// are blind to. `detail` carries the `wasm=… rust=…` disagreement (dedup key + note body).
    Differential,
    /// A generated program that PASSED the compiler's coded well-formedness checks nonetheless
    /// REACHED a CODELESS `Reject::decline` — the class-2 / assumed-unreachable set (operator
    /// directive, 2026-08-31). Either the decline is a defense-in-depth backstop the type checker
    /// was supposed to make unreachable (so reaching it = the invariant is FALSE = a bug), or it is
    /// a reachable-but-uncoded feature gap that should be CODED. Both are findings routed to
    /// v-deferral-declines to triage. `detail` carries the codeless decline message (dedup key).
    ReachabilityInvariant,
    /// The independent Lean TYPE oracle disagrees with rcdzc's `cdz check` accept/reject decision
    /// (design `DESIGN-lean-type-system-oracle.md` §1.2). Unlike the wasm-vs-rust [`Differential`]
    /// (both sides share the front-end, so a decline is never a mismatch there), the type oracle
    /// shares ZERO code with rcdzc, so it catches the front-end's OWN blind spots. `detail` carries
    /// the oracle's mismatch direction — `false-reject:…` (rcdzc coded-rejected a well-typed
    /// program — a compiler bug), `capability-gap:…` (codeless decline of a well-typed program — a
    /// should-work feature gap, backlog), `false-accept:…` (rcdzc accepted an ill-typed program — a
    /// soundness hole), or `code-mismatch:…` (both reject, different CDZ code — diagnostic quality).
    TypeOracle,
}

impl Category {
    fn tag(self) -> &'static str {
        match self {
            Category::Crash => "crash",
            Category::Timeout => "timeout",
            Category::InvalidWasm => "invalid-wasm",
            Category::Differential => "differential",
            Category::ReachabilityInvariant => "reachability-invariant",
            Category::TypeOracle => "type-oracle",
        }
    }
}

/// A single finding, ready to persist.
#[derive(Clone, Debug)]
pub struct Finding {
    pub category: Category,
    /// The (already shrunk) reproducer source, in the runnable export shape.
    pub program: String,
    /// Crash details. `None` for a timeout or invalid-wasm (there is no panic site).
    pub crash: Option<CrashInfo>,
    /// For `InvalidWasm`: the validator's rejection message (dedup key + triage detail). `None`
    /// otherwise.
    pub detail: Option<String>,
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
            // Bucket invalid-wasm findings by the validator's (masked) error, so distinct backend
            // faults get distinct buckets rather than collapsing like timeouts do.
            (None, Category::InvalidWasm) => {
                let d = self.detail.as_deref().unwrap_or("invalid-wasm");
                format!("invalid-wasm::{}", mask_message(d))
            }
            // Bucket differential findings by the (masked) disagreement, so distinct miscompiles get
            // distinct buckets. The detail is `wasm=<a> rust=<b>`; masking digits/hex keeps programs
            // that differ only in literal magnitudes (`wasm=6 rust=7` vs `wasm=42 rust=43`) in ONE
            // bucket per shape rather than one per magnitude.
            (None, Category::Differential) => {
                let d = self.detail.as_deref().unwrap_or("differential");
                format!("differential::{}", mask_message(d))
            }
            (None, Category::Timeout) => "timeout".to_string(),
            // Bucket by the (masked) codeless-decline message, so each distinct assumed-unreachable
            // SITE gets one bucket regardless of how many programs reach it.
            (None, Category::ReachabilityInvariant) => {
                let d = self.detail.as_deref().unwrap_or("reachability-invariant");
                format!("reachability-invariant::{}", mask_message(d))
            }
            // Bucket type-oracle findings by the (masked) mismatch detail — the direction word
            // (`false-reject`/`capability-gap`/`false-accept`/`code-mismatch`) plus the inferred
            // type/code, so distinct typing disagreements get distinct buckets.
            (None, Category::TypeOracle) => {
                let d = self.detail.as_deref().unwrap_or("type-oracle");
                format!("type-oracle::{}", mask_message(d))
            }
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
            (None, Category::InvalidWasm) => format!(
                "backend emitted INVALID wasm: {}",
                first_line(self.detail.as_deref().unwrap_or("validation failed"))
            ),
            (None, Category::Differential) => {
                let d = self.detail.as_deref();
                let body = first_line(d.unwrap_or("wasm ≠ rust"));
                // A `Differential` covers three sub-kinds (see `differential::MismatchKind`); the
                // canned "disagree on VALUE" is wrong for a liveness or artifact-error divergence.
                match differential_tag(d) {
                    "artifact" => {
                        format!(
                            "backends DIVERGE at compile — one builds, the other's emitted source fails to compile: {body}"
                        )
                    }
                    "liveness" => format!("backends DISAGREE — one runs, one traps: {body}"),
                    _ => format!("backends DISAGREE on value: {body}"),
                }
            }
            (None, Category::Timeout) => {
                "compiler timeout (no result inside the budget)".to_string()
            }
            (None, Category::ReachabilityInvariant) => format!(
                "REACHED an assumed-unreachable (codeless) decline: {}",
                first_line(self.detail.as_deref().unwrap_or("codeless decline"))
            ),
            (None, Category::TypeOracle) => format!(
                "type oracle DISAGREES with rcdzc's accept/reject: {}",
                first_line(self.detail.as_deref().unwrap_or("typing disagreement"))
            ),
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
        // The intro states what "never valid behavior" means for THIS category — a differential
        // finding is not a panic/hang/invalid-wasm, it's two backends producing incomparable
        // outcomes below the shared front-end, so the canned crash wording would misdescribe it.
        if finding.category == Category::Differential {
            s.push_str(
                "_Filed by `cdz-smith` (the compiler fuzzer). This is an auto-generated finding: the\n",
            );
            s.push_str(
                "two emit backends (wasm vs rust) produced DIFFERENT outcomes for one program — a\n",
            );
            s.push_str("value disagreement, a liveness split (one runs, one traps), or a compile divergence\n");
            s.push_str("(one emits, one rejects). They share the front-end, so a divergence below the emit\n");
            s.push_str("seam is a lowering bug on one side. Triage, fix, then rename this file `.RESOLVED.md`\n");
            s.push_str("(or `.REJECTED.md` with a rationale) so it is not re-triaged._\n\n");
        } else if finding.category == Category::ReachabilityInvariant {
            s.push_str(
                "_Filed by `cdz-smith` (the compiler fuzzer). This is an auto-generated finding: a\n",
            );
            s.push_str(
                "generated program that PASSED the compiler's coded well-formedness checks then REACHED\n",
            );
            s.push_str(
                "a CODELESS `Reject::decline` (the class-2 / assumed-unreachable set). Operator directive:\n",
            );
            s.push_str(
                "reaching such a decline is a BUG — either the decline is a defense-in-depth backstop the\n",
            );
            s.push_str(
                "type checker was supposed to make unreachable (invariant FALSE — a soundness/reachability\n",
            );
            s.push_str(
                "bug), or it is a reachable-but-uncoded feature gap that should be CODED. Route to\n",
            );
            s.push_str(
                "v-deferral-declines to triage (bug vs code-it). Rename `.RESOLVED.md`/`.REJECTED.md` on triage._\n\n",
            );
        } else if finding.category == Category::TypeOracle {
            s.push_str(
                "_Filed by `cdz-smith` (the compiler fuzzer). This is an auto-generated finding: the\n",
            );
            s.push_str(
                "independent Lean TYPE oracle (which shares ZERO code with rcdzc) disagrees with rcdzc's\n",
            );
            s.push_str(
                "`cdz check` accept/reject decision. A `false-reject` (rcdzc coded-rejected a program the\n",
            );
            s.push_str(
                "oracle types as WELL-TYPED) is a compiler BUG — an over-strict coded diagnostic; file as\n",
            );
            s.push_str(
                "an issue to the type-system owner. A `capability-gap` (codeless decline of a well-typed\n",
            );
            s.push_str(
                "program) is a should-work-but-unimplemented feature — route as a backlog / `(output V)`\n",
            );
            s.push_str(
                "TODO, not a soundness bug. A `false-accept` (rcdzc accepted an ill-typed program) is a\n",
            );
            s.push_str(
                "SOUNDNESS hole. A `code-mismatch` (both reject, different CDZ code) is diagnostic-quality.\n",
            );
            s.push_str("Rename `.RESOLVED.md`/`.REJECTED.md` on triage._\n\n");
        } else {
            s.push_str(
                "_Filed by `cdz-smith` (the compiler fuzzer). This is an auto-generated finding: a\n",
            );
            s.push_str(
                "generated program made the compiler PANIC, HANG, or emit INVALID wasm — never valid\n",
            );
            s.push_str("behavior, since the compiler reports every legitimate \"no\" as a diagnostic. Triage, fix,\n");
            s.push_str("then rename this file `.RESOLVED.md` (or `.REJECTED.md` with a rationale) so it is not\n");
            s.push_str("re-triaged._\n\n");
        }
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
        } else if finding.category == Category::InvalidWasm {
            s.push_str("## Invalid wasm (backend miscompile)\n\n");
            s.push_str(
                "The compiler reported SUCCESS, but the emitted component failed wasm validation\n",
            );
            s.push_str(
                "(`wasmparser` with `WasmFeatures::all()` — the same check rcdzc's own tests assert\n",
            );
            s.push_str(
                "emitted components pass). The backend produced structurally-invalid wasm.\n\n",
            );
            if let Some(d) = &finding.detail {
                s.push_str(&format!("- **Validator error:** {}\n", first_line(d)));
            }
        } else if finding.category == Category::Differential {
            match differential_tag(finding.detail.as_deref()) {
                "artifact" => {
                    s.push_str("## Backend compile divergence (miscompile)\n\n");
                    s.push_str(
                        "The two backends diverged at COMPILE time — one emitted a buildable artifact\n",
                    );
                    s.push_str(
                        "while the other emitted source that FAILED TO BUILD (`cdz run-rust` → a rustc\n",
                    );
                    s.push_str(
                        "`error`, e.g. `E0308`). This is NOT a value disagreement: one side produced no\n",
                    );
                    s.push_str(
                        "value at all. The compiler reported success at the emit seam but produced\n",
                    );
                    s.push_str(
                        "un-compilable source on that side — a build-blocking lowering bug. The\n",
                    );
                    s.push_str("crash/invalid-wasm oracles are blind to it.\n\n");
                }
                "liveness" => {
                    s.push_str("## Backend liveness disagreement (miscompile)\n\n");
                    s.push_str(
                        "Both backends produced a VALID artifact, but they diverged at RUN time — one\n",
                    );
                    s.push_str(
                        "backend ran the program to a value where the other TRAPPED. The backends share\n",
                    );
                    s.push_str(
                        "the front-end and diverge below the emit seam, so this is a lowering bug on one\n",
                    );
                    s.push_str("side. The crash/invalid-wasm oracles are blind to it.\n\n");
                }
                _ => {
                    s.push_str("## Backend value disagreement (miscompile)\n\n");
                    s.push_str(
                        "Both emit backends produced a VALID artifact, but they DISAGREE on the program's\n",
                    );
                    s.push_str(
                        "result — the wasm component (run via `cdz-run`) and the Rust backend (run via\n",
                    );
                    s.push_str(
                        "`cdz run-rust`) computed DIFFERENT values. The backends share the front-end and\n",
                    );
                    s.push_str(
                        "diverge below the emit seam, so this is a lowering bug on one side. The\n",
                    );
                    s.push_str("crash/invalid-wasm oracles are blind to it.\n\n");
                }
            }
            if let Some(d) = &finding.detail {
                s.push_str(&format!("- **Disagreement:** {}\n", first_line(d)));
            }
        } else if finding.category == Category::Timeout {
            s.push_str("## Timeout\n\n");
            s.push_str(
                "The compiler did not produce a result within the wall-clock budget. This is\n",
            );
            s.push_str(
                "detected out of process (an in-process catch cannot interrupt a runaway loop).\n",
            );
        } else if finding.category == Category::ReachabilityInvariant {
            s.push_str("## Reached an assumed-unreachable (codeless) decline\n\n");
            s.push_str(
                "The program compiled far enough to pass every CODED well-formedness check (no\n",
            );
            s.push_str(
                "`CDZ####` rejection fired), then hit a CODELESS `Reject::decline` — the class-2 set\n",
            );
            s.push_str(
                "that is assumed unreachable when the type checker + earlier phases are correct. The\n",
            );
            s.push_str(
                "fuzzer reaching it falsifies that assumption. v-deferral-declines triages: (a) a genuine\n",
            );
            s.push_str(
                "defense-in-depth backstop → soundness/reachability BUG (route to the decline owner); or\n",
            );
            s.push_str(
                "(b) a reachable-but-uncoded feature gap → it should be CODED (`declined(id)`).\n\n",
            );
            if let Some(d) = &finding.detail {
                s.push_str(&format!(
                    "- **Codeless decline message:** {}\n",
                    first_line(d)
                ));
            }
        } else if finding.category == Category::TypeOracle {
            s.push_str(
                "## Type-oracle disagreement (rcdzc vs the independent Lean type checker)\n\n",
            );
            s.push_str(
                "cdz-smith ran the program through rcdzc's front-end (`cdz check`) AND through an\n",
            );
            s.push_str(
                "independent Lean type checker that shares no code with rcdzc. The two disagree on\n",
            );
            s.push_str(
                "whether the program is well-typed. Because the oracle is independent, this catches\n",
            );
            s.push_str(
                "front-end blind spots the same-front-end wasm-vs-rust differential cannot (there a\n",
            );
            s.push_str(
                "decline is never a mismatch). See the intro for how to route each direction.\n\n",
            );
            if let Some(d) = &finding.detail {
                s.push_str(&format!("- **Disagreement:** {}\n", first_line(d)));
            }
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
    shrink_while(source, same_site)
}

/// Minimize an invalid-wasm reproducer, preserving that the shrunk program STILL compiles to a
/// component that fails validation (any validation failure — we don't require the identical
/// validator message, since shrinking may legitimately surface a different first error).
pub fn shrink_invalid_wasm(source: &str) -> String {
    shrink_while(source, |v| matches!(v, Verdict::InvalidWasm { .. }))
}

/// Minimize while the program still reaches THE SAME codeless decline (`Verdict::Declined
/// { code: None, message: target }`) — the class-2 / assumed-unreachable witness. Preserving the
/// exact message (not merely "some uncoded decline") keeps the shrunk repro on the SAME site, so it
/// can't drift into a different codeless decline than the one this finding is filed under.
pub fn shrink_codeless_decline(source: &str, target_message: &str) -> String {
    shrink_while(
        source,
        |v| matches!(v, Verdict::Declined { code: None, message } if message == target_message),
    )
}

/// The shared shrink loop: greedily delete balanced sub-forms, keeping any deletion whose result
/// still satisfies `keep` (and still parses — a `ParseError` fails `keep` for every caller).
fn shrink_while(source: &str, keep: impl Fn(&Verdict) -> bool) -> String {
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
            if keep(&compile_catching(&candidate)) {
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
pub(crate) fn balanced_spans(s: &str) -> Vec<(usize, usize)> {
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

/// Extract the differential `MismatchKind` tag from a finding's `detail`, which the driver formats
/// as `"[<kind>] wasm=… rust=…"` (see `driver.rs`). Returns the bare tag (`"value"`, `"liveness"`,
/// `"artifact"`) or `"value"` as the conservative default when the prefix is absent/unrecognized —
/// so the note wording matches the ACTUAL recorded outcome rather than always claiming a value
/// disagreement (a wasm=value vs rust=E0308 artifact-error is a compile divergence, not a value one).
fn differential_tag(detail: Option<&str>) -> &'static str {
    let d = match detail {
        Some(d) => d.trim_start(),
        None => return "value",
    };
    match d.strip_prefix('[').and_then(|r| r.split_once(']')) {
        Some(("artifact", _)) => "artifact",
        Some(("liveness", _)) => "liveness",
        _ => "value",
    }
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
            detail: None,
            commit: "abc123".into(),
        };
        let s1 = mk("crates/rcdzc/src/lower.rs:766:9", "position 3 not found").signature();
        let s2 = mk("crates/rcdzc/src/lower.rs:766:9", "position 99 not found").signature();
        let s3 = mk("crates/rcdzc/src/select.rs:1266:5", "position 3 not found").signature();
        assert_eq!(s1, s2, "same site + masked message → same bucket");
        assert_ne!(s1, s3, "different site → different bucket");
    }

    #[test]
    fn invalid_wasm_buckets_by_validator_error() {
        let mk = |detail: &str| Finding {
            category: Category::InvalidWasm,
            program: "(do (def (main) 0) (export main))".into(),
            crash: None,
            detail: Some(detail.into()),
            commit: "abc".into(),
        };
        // Same error shape (differing only in offsets) → one bucket; a different error → another.
        let a = mk("type mismatch: expected i32, found i64 (at offset 128)").signature();
        let b = mk("type mismatch: expected i32, found i64 (at offset 992)").signature();
        let c = mk("unknown function 7").signature();
        assert_eq!(a, b, "same masked validator error → same bucket");
        assert_ne!(a, c, "different validator error → different bucket");
        assert!(a.starts_with("invalid-wasm"), "sig namespaced: {a}");
    }

    fn differential(detail: &str) -> Finding {
        Finding {
            category: Category::Differential,
            program: "(do (def (main) 0) (export main))".into(),
            crash: None,
            detail: Some(detail.into()),
            commit: "abc".into(),
        }
    }

    #[test]
    fn differential_tag_reads_the_kind_prefix() {
        assert_eq!(
            differential_tag(Some("[artifact] wasm=value 1 rust=error")),
            "artifact"
        );
        assert_eq!(
            differential_tag(Some("[liveness] wasm=value 1 rust=trap x")),
            "liveness"
        );
        assert_eq!(differential_tag(Some("[value] wasm=3 rust=4")), "value");
        // Missing / unrecognized prefix → conservative "value".
        assert_eq!(differential_tag(Some("wasm=3 rust=4")), "value");
        assert_eq!(differential_tag(None), "value");
    }

    #[test]
    fn artifact_error_note_reads_as_a_compile_divergence_not_a_value_disagreement() {
        // The github-liaison finding: wasm=value, rust=E0308 artifact-error. The note must NOT claim
        // a value disagreement or that both produced a valid artifact, and the intro must not claim a
        // panic/hang/invalid-wasm.
        let f = differential(
            "[artifact] wasm=wasm value (list 1 -41) rust=artifact-error error[E0308]: mismatched types",
        );
        let store = FindingStore::open(std::env::temp_dir()).unwrap();
        let note = store.render_note(&f, std::path::Path::new("x.smith.sexp"));
        assert!(
            note.contains("compile"),
            "title/section should say compile divergence:\n{note}"
        );
        assert!(
            !note.contains("DISAGREE on value"),
            "must not claim a value disagreement:\n{note}"
        );
        assert!(
            !note.contains("PANIC, HANG"),
            "differential intro must not claim panic/hang:\n{note}"
        );
        assert!(
            !note.contains("Both emit backends produced a VALID artifact"),
            "must not assert both produced a valid artifact:\n{note}"
        );
    }

    #[test]
    fn value_and_liveness_notes_keep_their_own_wording() {
        let store = FindingStore::open(std::env::temp_dir()).unwrap();
        let val = store.render_note(
            &differential("[value] wasm=3 rust=4"),
            std::path::Path::new("x.smith.sexp"),
        );
        assert!(
            val.contains("DISAGREE on value"),
            "value note wording:\n{val}"
        );
        let live = store.render_note(
            &differential("[liveness] wasm=value 7 rust=trap overflow"),
            std::path::Path::new("x.smith.sexp"),
        );
        assert!(live.contains("liveness"), "liveness note wording:\n{live}");
        assert!(
            !live.contains("PANIC, HANG"),
            "differential intro must not claim panic/hang:\n{live}"
        );
    }

    fn type_oracle(detail: &str) -> Finding {
        Finding {
            category: Category::TypeOracle,
            program: "(do (def (main) 0) (export main))".into(),
            crash: None,
            detail: Some(detail.into()),
            commit: "abc".into(),
        }
    }

    #[test]
    fn type_oracle_buckets_by_mismatch_direction() {
        let fr1 = type_oracle("false-reject: oracle infers Int64 over CDZ0203").signature();
        // Same direction + shape, differing only in a code number → one bucket (digits masked).
        let fr2 = type_oracle("false-reject: oracle infers Int64 over CDZ0201").signature();
        let cap = type_oracle("capability-gap: oracle infers Bool").signature();
        let fa = type_oracle("false-accept: oracle rejects CDZ0203").signature();
        assert_eq!(
            fr1, fr2,
            "same false-reject shape → one bucket: {fr1} vs {fr2}"
        );
        assert_ne!(
            fr1, cap,
            "false-reject and capability-gap are distinct buckets"
        );
        assert_ne!(
            fr1, fa,
            "false-reject and false-accept are distinct buckets"
        );
        assert!(fr1.starts_with("type-oracle"), "sig namespaced: {fr1}");
    }

    #[test]
    fn type_oracle_note_reads_as_a_typing_disagreement() {
        let store = FindingStore::open(std::env::temp_dir()).unwrap();
        let note = store.render_note(
            &type_oracle("false-reject: oracle infers Int64 over rcdzc CDZ0203"),
            std::path::Path::new("x.smith.sexp"),
        );
        // The note must describe an independent-type-oracle disagreement, route by direction, and NOT
        // reuse the crash/hang/invalid-wasm or wasm-vs-rust value wording.
        assert!(
            note.contains("type-oracle"),
            "category tag in note:\n{note}"
        );
        assert!(
            note.contains("false-reject") && note.contains("independent"),
            "intro must explain the independent oracle + direction routing:\n{note}"
        );
        assert!(
            !note.contains("PANIC, HANG") && !note.contains("wasm vs rust"),
            "must not reuse crash/wasm-vs-rust wording:\n{note}"
        );
        assert!(
            note.contains("- **Disagreement:**"),
            "note surfaces the disagreement detail:\n{note}"
        );
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
            detail: None,
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
