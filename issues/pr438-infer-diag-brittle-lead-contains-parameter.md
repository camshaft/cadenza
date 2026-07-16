# PR review comment — mirrored from GitHub PR #438 (Copilot inline)

- **PR:** #438 "fleet: batch 65+66 (…, tycheck factoring, …)" (OPEN at triage; file on trunk)
- **File:** `implementation/seed/crates/rcdzc/src/infer.rs:1398` (`non_type_annotation_message`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3592525270
- **Link:** https://github.com/camshaft/cadenza/pull/438#discussion_r3592525270

## Comment (verbatim)
> Using `lead.contains("parameter")` to decide whether to include the extra Type-parameter guidance is brittle: it couples behavior to the exact phrasing of a human-readable string and can change unintentionally if the wording is tweaked. Prefer a stricter check at minimum (or ideally a boolean/enum flag passed by the caller).

## Liaison triage — CONFIRMED against trunk
Confirmed: `non_type_annotation_message` (infer.rs:1209) contains `let type_param_route = if
lead.contains("parameter") { … }` (line 1398) — the diagnostic branches its guidance on whether the
human-readable `lead` string happens to contain the substring "parameter". So a reword of `lead`
(harmless prose change) would silently drop/add the type-parameter route guidance. This is the
"brittle string-match to drive behavior" anti-pattern (behavior should key off structure, not
human-facing phrasing). FIX: pass a boolean/enum flag from the caller (the caller knows whether it's the
parameter case) instead of sniffing the message text. Diagnostics territory (v-diagnostics — message
construction). Fix on `trunk`. Quote + link in queue file.
