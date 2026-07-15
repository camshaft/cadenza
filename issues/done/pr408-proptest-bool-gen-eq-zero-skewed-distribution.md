# PR review comments — mirrored from GitHub PR #408 (Copilot inline)

- **PR:** #408 "fleet: thirty-third batch (UN-BREAK compiler-ml: set-to-list prelude surface + 22 features)" (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/proptest_gen.rs` (Bool gen @309/351, docs @285/134)
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3591225554, 3591225585, 3591225599
- **Links:** https://github.com/camshaft/cadenza/pull/408#discussion_r3591225554 (+ r3591225585, r3591225599)

## Comment (verbatim, primary)
> `ElemKind::Bool` currently generates booleans as `(= gen_call 0)`, which will be overwhelmingly `false` for a typical `Test.gen` integer stream (only `true` when the generated int happens to be exactly zero). That severely reduces property-test coverage for boolean parameters and contradicts the comment that this is a "low-bit-ish parity" mapping.

## Liaison triage — CONFIRMED against trunk
Confirmed in proptest_gen.rs (~line 351-355): the Bool generator builds `(= ((. Test gen)) 0)` — a
gen int read as a boolean via EQUALITY to 0. That's `true` only when the int is exactly 0, so over a
typical Test.gen int stream the boolean is overwhelmingly `false` → property tests over Bool params
barely explore `true`. The code comment even says "Any total int→Bool map works; equality" and
elsewhere claims a "low-bit-ish parity" mapping — but `= 0` is neither balanced nor a parity map. This
is a real property-testing COVERAGE bug (weak generator distribution), plus the two doc comments
(@285, @134) describe it inconsistently. FIX: use a balanced int→Bool map — a low-bit/parity test
(`(= (Int64.rem g 2) 0)` or a bit-and) — and align the docs. Property-testing workstream (no dedicated
vertical) → route to `corpus-bugfix` PM. Fix on `trunk`. Quotes + links in queue file.
