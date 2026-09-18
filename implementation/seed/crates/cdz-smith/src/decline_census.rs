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
//! bucket carries its [`DeclineClass`] (codeless / CDZ09xx-unsupported-gap / other-coded) AND whether
//! the site is `declined(id)`-TRACKED — surfaced through the compile ABI as `Diagnostic::decline_id`
//! (the stable catalog key, `None` for a bare untracked decline). So the census reports the TRUE
//! reachable-UNTRACKED count directly (codeless + CDZ09xx-gap with no `declined(id)`), no manual
//! subtraction: a reachable site already migrated to `declined(id)` is counted as tracked, not
//! untracked.
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
    /// A code in the CDZ09xx not-yet-built DECLINE band — `CDZ0900` (the `unsupported` umbrella) OR a
    /// dedicated split-off like `CDZ0901` (closure-across-ABI, family E, rcdzc #9252). ALL are declines
    /// (rcdzc `Reject::is_decline` holds for them), so ALL count toward the reachable-untracked gap — the
    /// band, not just the umbrella. See [`is_decline_band_code`]; keep it in sync with rcdzc `is_decline`.
    Unsupported,
    /// Any OTHER coded rejection (`CDZ####` outside the decline band) — a genuine "the program is WRONG"
    /// rejection (CDZ0101 unbound, CDZ0203 type mismatch, CDZ0999 recursion bound, …), NOT part of the
    /// gap denominator. Carries the raw code.
    Coded(String),
}

/// The `CDZ09xx` codes that are DECLINES (a not-yet-built construct), not genuine rejections — the gap
/// band the census counts toward reachable-untracked. Mirrors rcdzc `diag::Reject::is_decline` (which
/// enumerates `UnsupportedConstruct`=CDZ0900 + `ClosureAcrossAbiUnsupported`=CDZ0901). NOTE: NOT every
/// CDZ09xx is a decline (e.g. CDZ0999 recursion/resource bound is a rejection), so this ENUMERATES the
/// decline codes rather than pattern-matching the `CDZ09` prefix. Add any future split-off decline code
/// here when rcdzc adds it to `is_decline`.
fn is_decline_band_code(code: &str) -> bool {
    matches!(code, "CDZ0900" | "CDZ0901")
}

impl DeclineClass {
    /// Classify a `Verdict::Declined { code, .. }`'s `code` into the census axis.
    pub fn from_code(code: Option<&str>) -> DeclineClass {
        match code {
            None => DeclineClass::Codeless,
            Some(c) if is_decline_band_code(c) => DeclineClass::Unsupported,
            Some(c) => DeclineClass::Coded(c.to_string()),
        }
    }

    /// A short stable tag for the histogram line / summary.
    pub fn tag(&self) -> &str {
        match self {
            DeclineClass::Codeless => "codeless",
            DeclineClass::Unsupported => "CDZ09xx",
            DeclineClass::Coded(c) => c,
        }
    }
}

/// The key of a histogram bucket: the decline class, the emit-site identity, and whether the site is
/// `declined(id)`-TRACKED.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeclineKey {
    pub class: DeclineClass,
    /// The emit-site identity: for a TRACKED decline, the stable catalog KEY (`DeclineId::key()`, the
    /// canonical site id); for an UNTRACKED decline, the masked message template ([`emit_site_key`]).
    pub site: String,
    /// `true` iff this decline named a stable catalog id (`declined(id, …)`) — i.e. it is TRACKED in the
    /// deferral-declines catalog. `false` for a bare codeless decline / `unsupported` with no id. This is
    /// the axis that turns "reachable codeless + CDZ0900" into the TRUE reachable-UNTRACKED count.
    pub tracked: bool,
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

    /// Record one reached decline. `code`/`message` come from `Verdict::Declined`; `decline_id` is the
    /// stable catalog KEY of a `declined(id, …)`-TRACKED decline (`Some(key)`), or `None` for an
    /// untracked bare decline/unsupported. A TRACKED site is keyed by its catalog key (the canonical
    /// site id); an UNTRACKED site by its masked message template.
    pub fn record_decline(&mut self, code: Option<&str>, message: &str, decline_id: Option<&str>) {
        self.declined += 1;
        let tracked = decline_id.is_some();
        let site = match decline_id {
            Some(key) => key.to_string(),
            None => emit_site_key(message),
        };
        let key = DeclineKey {
            class: DeclineClass::from_code(code),
            site,
            tracked,
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

    /// The number of DISTINCT reachable emit sites in a class (tracked + untracked).
    pub fn distinct_sites(&self, class: &DeclineClass) -> usize {
        self.hits.keys().filter(|k| &k.class == class).count()
    }

    /// DISTINCT reachable UNTRACKED sites in a class (no `declined(id)`) — the true untracked surface.
    pub fn distinct_untracked_sites(&self, class: &DeclineClass) -> usize {
        self.hits
            .keys()
            .filter(|k| &k.class == class && !k.tracked)
            .count()
    }

    /// DISTINCT reachable TRACKED sites in a class (already `declined(id)`-tagged).
    pub fn distinct_tracked_sites(&self, class: &DeclineClass) -> usize {
        self.hits
            .keys()
            .filter(|k| &k.class == class && k.tracked)
            .count()
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
        let codeless_u = self.distinct_untracked_sites(&DeclineClass::Codeless);
        let codeless_t = self.distinct_tracked_sites(&DeclineClass::Codeless);
        let unsupported_u = self.distinct_untracked_sites(&DeclineClass::Unsupported);
        let unsupported_t = self.distinct_tracked_sites(&DeclineClass::Unsupported);
        s.push_str("=== reachable-decline census (emit-site histogram) ===\n");
        s.push_str(&format!(
            "outcomes: {} compiled | {} declined | {} other (crash/invalid-wasm/parse)\n",
            self.compiled, self.declined, self.other
        ));
        s.push_str(&format!(
            "reachable CODELESS declines : {} site(s) [{codeless_u} untracked / {codeless_t} tracked], {} hit(s)\n",
            codeless_u + codeless_t,
            self.hits_in(&DeclineClass::Codeless)
        ));
        s.push_str(&format!(
            "reachable UNSUPPORTED-gap (CDZ09xx band: CDZ0900/CDZ0901): {} site(s) [{unsupported_u} untracked / {unsupported_t} tracked], {} hit(s)\n",
            unsupported_u + unsupported_t,
            self.hits_in(&DeclineClass::Unsupported)
        ));
        s.push_str(&format!(
            "TRUE REACHABLE-UNTRACKED (codeless + CDZ09xx, no declined(id)): {} distinct site(s)\n",
            codeless_u + unsupported_u
        ));
        s.push_str(&format!(
            "  (of which {} reachable site(s) are ALREADY declined(id)-tracked — excluded above)\n",
            codeless_t + unsupported_t
        ));
        s.push_str("--- histogram (class · T=tracked/U=untracked · hits · emit site) ---\n");
        for (key, n) in self.sorted() {
            let tag = if key.tracked { "T" } else { "U" };
            s.push_str(&format!(
                "{:>7} {tag}  {:>6}  {}\n",
                key.class.tag(),
                n,
                key.site
            ));
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
        // CDZ0901 (closure-across-ABI, family E) is a DECLINE-band code (rcdzc #9252) — it counts as
        // Unsupported (part of the gap denominator), NOT a generic coded rejection.
        assert_eq!(
            DeclineClass::from_code(Some("CDZ0901")),
            DeclineClass::Unsupported
        );
        // CDZ0999 (recursion/resource bound) is NOT a decline → a genuine coded rejection, excluded.
        assert_eq!(
            DeclineClass::from_code(Some("CDZ0999")),
            DeclineClass::Coded("CDZ0999".to_string())
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
        h.record_decline(None, "cannot lower node 1830 here", None);
        h.record_decline(None, "cannot lower node 42 here", None);
        // A distinct codeless site.
        h.record_decline(None, "unexpected shape in escape", None);
        // Two CDZ0900 hits at one site.
        h.record_decline(Some("CDZ0900"), "unsupported: higher-rank type", None);
        h.record_decline(Some("CDZ0900"), "unsupported: higher-rank type", None);
        // An other-coded reject.
        h.record_decline(Some("CDZ0203"), "type mismatch", None);
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
        h.record_decline(Some("CDZ0900"), "the host operation `o` has a result of type `String`, which has no component boundary form", None);
        h.record_decline(Some("CDZ0900"), "the host operation `o` has a result of type `(Option Unit)`, which has no component boundary form", None);
        h.record_decline(Some("CDZ0900"), "the host operation `o` has a result of type `(List Int64)`, which has no component boundary form", None);
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
                None,
            );
        }
        assert_eq!(h.distinct_sites(&DeclineClass::Unsupported), 1);
        assert_eq!(h.hits_in(&DeclineClass::Unsupported), 4);
    }

    #[test]
    fn tracked_declines_are_excluded_from_the_true_untracked_count() {
        let mut h = DeclineHistogram::new();
        // One untracked codeless + one untracked CDZ0900 = the true reachable-untracked surface.
        h.record_decline(None, "codeless A", None);
        h.record_decline(Some("CDZ0900"), "unsupported B", None);
        // Two TRACKED declines (declined(id)) — reachable but already migrated, must NOT count as untracked.
        h.record_decline(None, "codeless C", Some("some-tracked-codeless"));
        h.record_decline(
            Some("CDZ0900"),
            "unsupported D",
            Some("wasm-host-op-no-boundary-form"),
        );

        assert_eq!(h.distinct_untracked_sites(&DeclineClass::Codeless), 1);
        assert_eq!(h.distinct_tracked_sites(&DeclineClass::Codeless), 1);
        assert_eq!(h.distinct_untracked_sites(&DeclineClass::Unsupported), 1);
        assert_eq!(h.distinct_tracked_sites(&DeclineClass::Unsupported), 1);

        let r = h.report();
        // The true untracked number excludes the 2 tracked sites (2 untracked, not 4).
        assert!(r.contains(
            "TRUE REACHABLE-UNTRACKED (codeless + CDZ09xx, no declined(id)): 2 distinct site(s)"
        ));
        assert!(r.contains("2 reachable site(s) are ALREADY declined(id)-tracked"));
        // A tracked site is keyed by its stable catalog key, not the message.
        assert!(r.contains("wasm-host-op-no-boundary-form"));
    }
}
