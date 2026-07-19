# PR review comment — mirrored from GitHub PR #418 (Copilot inline)

- **PR:** #418 "fleet: forty-second batch (28 MRs …)" (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/compile.rs:2287`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3591765110
- **Link:** https://github.com/camshaft/cadenza/pull/418#discussion_r3591765110

## Comment (verbatim)
> The new peer-closure diagnostic says it is "reported at the bind name", but it's currently anchored at the first nested `(-> …)` position (`.at(pos)`). That makes the error highlight jump to the inner type fragment rather than the `(bind …)` directive, which is the actionable location for the user.

## Liaison triage — CONFIRMED against trunk
Confirmed in compile.rs: the CLOSURE_ACROSS_PEER_MESSAGE reject is emitted `.at(pos)`, where `pos` is
set by the preceding loop to the first nested `(-> …)` arrow form (`if db.ast.as_form(pos, "->")`).
So the diagnostic highlights the inner arrow/type fragment, not the `(bind E "pkg/iface")` directive —
but the message claims it's "reported at the bind name". The actionable locus for the user is the
`(bind …)` line. Diagnostic-precision bug (anchor the reject at the bind directive, or fix the message).
Diagnostics territory (v-diagnostics); it's in the peer-linking/effects diagnostic path. Fix on `trunk`.
Quote + link in queue file.

## ✅ RESOLVED (2026-07-16, v-peer-linking) — anchor moved to the bind name; landed `8cfca82db`
Verified against trunk `6c1e89f65`. The CLOSURE_ACROSS_PEER reject is now `.at(name_occ)` — the
`(bind E …)` directive's effect NAME, the actionable locus — NOT the inner `(-> …)` arrow fragment
`pos` that merely DETECTED the closure (compile.rs:2513-2521, with a comment citing Copilot PR #418).
Pinned: `tests.rs:69064-69075` asserts the anchored node resolves to the bind name `"F"`, not an arrow
list (the compiler is span-free, so the test checks the anchored NODE). Landed in batch 47+48
(#422, `8cfca82db`, "host-closure declines land"). CLOSED.
