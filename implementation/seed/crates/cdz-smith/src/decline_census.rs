//! Reachable-decline census — the emit-site histogram of every `Reject::decline` /
//! `Reject::unsupported` (CDZ0900) a GENERATED program actually reaches.
//!
//! WHY this exists (v-deferral-declines request, 2026-09-18): the *static* decline-emit surface in
//! `rcdzc` is ~741 sites (16 `declined(id)`-tracked + 197 CDZ0900 `unsupported` + 528 codeless
//! `Reject::decline`). A static count over-states the real production-readiness gap because most
//! untracked sites are UNREACHABLE defensive guards. The honest prod-readiness denominator is the
//! REACHABLE-untracked count — and reachability is exactly what cdz-smith knows: a decline reached by
//! a program the generator emitted is, by construction, reachable. This module buckets those reached
//! declines by emit site so the reachable surface can be reconciled down from the static 741.
//!
//! "Emit site" is keyed by [`emit_site_key`]: the finding-dedup mask
//! ([`crate::finding::mask_message`]: first line, digit/hex runs → `#`) with backtick-quoted spans
//! AND bare parenthesized type-spans ALSO collapsed to placeholders — so a single compiler emit site
//! that names a varying type (e.g. the host-boundary-form check, or the parameterized-heap-return
//! export) stays ONE bucket instead of splitting per type. Each
//! bucket carries its [`DeclineClass`] (codeless / CDZ0900-unsupported / other-coded), which is the
//! axis v-deferral-declines splits on (codeless + CDZ0900, minus the already-`declined(id)`-tracked
//! set, is the reachable-untracked number).
//!
//! Declines are EXPECTED compiler output, never a bug — this is a GAP INVENTORY, not a finding hunt.

use std::collections::HashMap;

use crate::finding::{first_line, mask_message};

/// The class of a reached decline — the axis v-deferral-declines splits the reachable surface on.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DeclineClass {
    /// A codeless `Reject::decline` (`code == None`) — the class-2 / assumed-unreachable set (the 528
    /// static codeless sites). A reached one is the reachable-codeless denominator.
    Codeless,
    /// A `Reject::unsupported` — the `CDZ0900` "not lowered yet" gap (the 197 static CDZ0900 sites).
    Unsupported,
    /// Any OTHER coded rejection (`CDZ####`, not `CDZ0900`) — a genuine semantic reject, tracked here
    /// for completeness so a caller can subtract it. Carries the raw code.
    Coded(String),
}

impl DeclineClass {
    /// Classify a `Verdict::Declined { code, .. }`'s `code` into the census axis.
    pub fn from_code(code: Option<&str>) -> DeclineClass {
        match code {
            None => DeclineClass::Codeless,
            Some("CDZ0900") => DeclineClass::Unsupported,
            Some(c) => DeclineClass::Coded(c.to_string()),
        }
    }

    /// A short stable tag for the histogram line / summary.
    pub fn tag(&self) -> &str {
        match self {
            DeclineClass::Codeless => "codeless",
            DeclineClass::Unsupported => "CDZ0900",
            DeclineClass::Coded(c) => c,
        }
    }
}

/// The key of a histogram bucket: the decline class + the masked emit-site template.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeclineKey {
    pub class: DeclineClass,
    /// The masked first line of the decline message — the emit-site template (dedup key).
    pub site: String,
}

/// A running census of reached declines, keyed by emit site, plus outcome tallies.
#[derive(Clone, Debug, Default)]
pub struct DeclineHistogram {
    hits: HashMap<DeclineKey, u64>,
    /// One example message (unmasked first line) per bucket — for the human report.
    example: HashMap<DeclineKey, String>,
    pub compiled: u64,
    pub declined: u64,
    /// Non-compile, non-decline outcomes (crash / invalid-wasm / parse-error) — NOT declines, so they
    /// are not bucketed here, only counted so the totals reconcile.
    pub other: u64,
}

impl DeclineHistogram {
    pub fn new() -> DeclineHistogram {
        DeclineHistogram::default()
    }

    /// Record one reached decline (`code` from `Verdict::Declined`, `message` its detail).
    pub fn record_decline(&mut self, code: Option<&str>, message: &str) {
        self.declined += 1;
        let key = DeclineKey {
            class: DeclineClass::from_code(code),
            site: emit_site_key(message),
        };
        *self.hits.entry(key.clone()).or_insert(0) += 1;
        self.example
            .entry(key)
            .or_insert_with(|| first_line(message).to_string());
    }

    pub fn record_compiled(&mut self) {
        self.compiled += 1;
    }

    pub fn record_other(&mut self) {
        self.other += 1;
    }

    /// The number of DISTINCT reachable emit sites in a class.
    pub fn distinct_sites(&self, class: &DeclineClass) -> usize {
        self.hits.keys().filter(|k| &k.class == class).count()
    }

    /// Total decline hits in a class (across all its sites).
    pub fn hits_in(&self, class: &DeclineClass) -> u64 {
        self.hits
            .iter()
            .filter(|(k, _)| &k.class == class)
            .map(|(_, n)| *n)
            .sum()
    }

    /// Buckets sorted for a stable report: by class, then by hit-count descending, then by site.
    fn sorted(&self) -> Vec<(&DeclineKey, u64)> {
        let mut v: Vec<(&DeclineKey, u64)> = self.hits.iter().map(|(k, n)| (k, *n)).collect();
        v.sort_by(|(ka, na), (kb, nb)| {
            ka.class
                .cmp(&kb.class)
                .then(nb.cmp(na))
                .then(ka.site.cmp(&kb.site))
        });
        v
    }

    /// Render the human-readable emit-site histogram + per-class summary. This is the artifact
    /// v-deferral-declines reconciles the static 741 surface against.
    pub fn report(&self) -> String {
        let mut s = String::new();
        let codeless_sites = self.distinct_sites(&DeclineClass::Codeless);
        let unsupported_sites = self.distinct_sites(&DeclineClass::Unsupported);
        s.push_str("=== reachable-decline census (emit-site histogram) ===\n");
        s.push_str(&format!(
            "outcomes: {} compiled | {} declined | {} other (crash/invalid-wasm/parse)\n",
            self.compiled, self.declined, self.other
        ));
        s.push_str(&format!(
            "reachable CODELESS declines : {codeless_sites} distinct site(s), {} hit(s)\n",
            self.hits_in(&DeclineClass::Codeless)
        ));
        s.push_str(&format!(
            "reachable CDZ0900 (unsupported): {unsupported_sites} distinct site(s), {} hit(s)\n",
            self.hits_in(&DeclineClass::Unsupported)
        ));
        s.push_str(&format!(
            "REACHABLE decline surface (codeless + CDZ0900): {} distinct site(s) — UPPER BOUND on reachable-untracked\n",
            codeless_sites + unsupported_sites
        ));
        s.push_str(
            "  NOTE: a declined(id)-TAGGED site STILL appears above — `declined(id)` keeps code None/CDZ0900,\n",
        );
        s.push_str(
            "  and the DeclineId that marks it tracked is dropped in the Reject->Diagnostic ABI projection,\n",
        );
        s.push_str(
            "  so this oracle cannot yet subtract tracked sites. Subtract the declined(id)-tracked set for the\n",
        );
        s.push_str("  true reachable-untracked gap (see cdz-smith note to v-deferral-declines, 2026-09-18).\n");
        s.push_str("--- histogram (class · hits · masked emit site) ---\n");
        for (key, n) in self.sorted() {
            s.push_str(&format!("{:>7}  {:>6}  {}\n", key.class.tag(), n, key.site));
        }
        s
    }
}

/// The histogram bucket key for a decline message — an approximation of the compiler EMIT SITE.
///
/// Starts from the finding-dedup mask ([`mask_message`]: first line, digit/hex runs → `#`), then
/// ALSO collapses every backtick-quoted span (`` `String` ``, `` `(Option Unit)` ``, `` `o` ``) to a
/// single `` `_` `` placeholder. A decline's message almost always quotes the offending TYPE or NAME,
/// which varies program-to-program even though the EMIT SITE is one place in the source; collapsing
/// the quoted span keys the histogram on the site's STABLE template rather than the incidental type —
/// so e.g. the one host-boundary-form check does not split into ~50 buckets by result type. (This is
/// deliberately MORE aggressive than the finding dedup key, whose goal is per-shape distinctness, not
/// per-site collapse — the census wants the tighter reachable-SITE count.)
///
/// Then ALSO collapses each balanced parenthesized span (a Cadenza type expression like
/// `(List Any)`, `(Result Int# _)`) to `(_)`. Some declines quote the offending type in BARE PARENS,
/// not backticks (e.g. "returning a (List Any) from `_`: a parameterized export cannot return this
/// heap type …"); without this, that ONE emit site splits into a bucket per return type. Nested
/// parens collapse under the OUTERMOST span.
fn emit_site_key(message: &str) -> String {
    collapse_parens(&collapse_backticks(&mask_message(message)))
}

/// Replace each balanced parenthesized span with `(_)`. Scans for a top-level `(` and skips to its
/// matching `)` (depth-counted, so nested parens collapse under the outermost). An unbalanced `(`
/// with no matching close leaves the rest of the string under one placeholder.
fn collapse_parens(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '(' {
            out.push_str("(_");
            let mut depth = 1usize;
            for d in chars.by_ref() {
                match d {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if depth == 0 {
                out.push(')');
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Replace the contents of each backtick-quoted span with a single `_`. An unterminated backtick
/// (no closing `` ` ``) leaves the rest of the string as-is under one placeholder.
fn collapse_backticks(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '`' {
            out.push('`');
            let mut closed = false;
            for d in chars.by_ref() {
                if d == '`' {
                    closed = true;
                    break;
                }
            }
            out.push('_');
            if closed {
                out.push('`');
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_code_into_census_axis() {
        assert_eq!(DeclineClass::from_code(None), DeclineClass::Codeless);
        assert_eq!(
            DeclineClass::from_code(Some("CDZ0900")),
            DeclineClass::Unsupported
        );
        assert_eq!(
            DeclineClass::from_code(Some("CDZ0203")),
            DeclineClass::Coded("CDZ0203".to_string())
        );
    }

    #[test]
    fn buckets_by_masked_site_and_tallies_per_class() {
        let mut h = DeclineHistogram::new();
        // Two codeless declines whose messages differ only in numbers → ONE masked site, 2 hits.
        h.record_decline(None, "cannot lower node 1830 here");
        h.record_decline(None, "cannot lower node 42 here");
        // A distinct codeless site.
        h.record_decline(None, "unexpected shape in escape");
        // Two CDZ0900 hits at one site.
        h.record_decline(Some("CDZ0900"), "unsupported: higher-rank type");
        h.record_decline(Some("CDZ0900"), "unsupported: higher-rank type");
        // An other-coded reject.
        h.record_decline(Some("CDZ0203"), "type mismatch");
        h.record_compiled();
        h.record_other();

        assert_eq!(h.declined, 6);
        assert_eq!(h.compiled, 1);
        assert_eq!(h.other, 1);
        // Codeless: the two number-only-differing messages collapse to one site; the escape one is a
        // second site → 2 distinct sites, 3 hits.
        assert_eq!(h.distinct_sites(&DeclineClass::Codeless), 2);
        assert_eq!(h.hits_in(&DeclineClass::Codeless), 3);
        // CDZ0900: one site, two hits.
        assert_eq!(h.distinct_sites(&DeclineClass::Unsupported), 1);
        assert_eq!(h.hits_in(&DeclineClass::Unsupported), 2);
        // Other-coded is tracked but NOT part of the reachable-untracked denominator.
        assert_eq!(
            h.distinct_sites(&DeclineClass::Coded("CDZ0203".to_string())),
            1
        );
    }

    #[test]
    fn quoted_type_variants_collapse_to_one_emit_site() {
        // The real host-boundary-form check emits the SAME message with only the quoted result type
        // varying — one emit site, many types. The census must key them to ONE bucket.
        let mut h = DeclineHistogram::new();
        h.record_decline(Some("CDZ0900"), "the host operation `o` has a result of type `String`, which has no component boundary form");
        h.record_decline(Some("CDZ0900"), "the host operation `o` has a result of type `(Option Unit)`, which has no component boundary form");
        h.record_decline(Some("CDZ0900"), "the host operation `o` has a result of type `(List Int64)`, which has no component boundary form");
        assert_eq!(h.distinct_sites(&DeclineClass::Unsupported), 1);
        assert_eq!(h.hits_in(&DeclineClass::Unsupported), 3);
    }

    #[test]
    fn collapse_backticks_handles_multiple_and_unterminated_spans() {
        assert_eq!(collapse_backticks("a `X` b `Y` c"), "a `_` b `_` c");
        assert_eq!(collapse_backticks("no ticks here"), "no ticks here");
        // Unterminated: consume the rest under one placeholder, no closing tick emitted.
        assert_eq!(collapse_backticks("open `tail with no close"), "open `_");
    }

    #[test]
    fn collapse_parens_handles_nesting_and_unbalanced() {
        assert_eq!(collapse_parens("a (List Any) b"), "a (_) b");
        assert_eq!(collapse_parens("(Result (Map Int (List Int)) _)"), "(_)");
        assert_eq!(collapse_parens("two (A) then (B)"), "two (_) then (_)");
        assert_eq!(collapse_parens("no parens"), "no parens");
        // Unbalanced open paren: rest of string under one placeholder, no closing paren emitted.
        assert_eq!(collapse_parens("open (tail no close"), "open (_");
    }

    #[test]
    fn parameterized_heap_return_type_variants_collapse_to_one_site() {
        // The real "parameterized export cannot return this heap type" message quotes the return type
        // in BARE PARENS (not backticks), so only the paren-collapse merges these into ONE site.
        let mut h = DeclineHistogram::new();
        for ty in [
            "(List Any)",
            "(Option Int)",
            "(Result Int _)",
            "(Record (: a Int))",
        ] {
            h.record_decline(
                Some("CDZ0900"),
                &format!(
                    "returning a {ty} from `_`: a parameterized export cannot return this heap type"
                ),
            );
        }
        assert_eq!(h.distinct_sites(&DeclineClass::Unsupported), 1);
        assert_eq!(h.hits_in(&DeclineClass::Unsupported), 4);
    }

    #[test]
    fn report_names_the_reachable_decline_surface_with_tracked_caveat() {
        let mut h = DeclineHistogram::new();
        h.record_decline(None, "codeless A");
        h.record_decline(Some("CDZ0900"), "unsupported B");
        let r = h.report();
        assert!(r.contains("REACHABLE decline surface (codeless + CDZ0900): 2 distinct site(s)"));
        // The report must flag that declined(id)-tracked sites are NOT yet subtractable by this oracle.
        assert!(r.contains("declined(id)-TAGGED site STILL appears"));
        assert!(r.contains("codeless"));
        assert!(r.contains("CDZ0900"));
    }
}
