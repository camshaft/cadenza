# PR review comment — mirrored from GitHub PR #425 (Copilot inline)

- **PR:** #425 "fleet: forty-ninth batch (…, float-width reject, …)" (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/eval.rs:2811` (`is_ill_formed_float_width`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3592073510
- **Link:** https://github.com/camshaft/cadenza/pull/425#discussion_r3592073510

## Comment (verbatim)
> `is_ill_formed_float_width` returns `false` for `WidthRead::NotConst`, but `read_width` yields `NotConst` for runtime widths (e.g. `(Float n)` where `n` is a parameter/ref). That means runtime float widths won't be flagged by `nested_ill_formed_float_width`, and `reduce_ctor` will clamp them to the sentinel width 0, which can slip past `cdz check` (contradicting the comment that runtime widths are rejected at the annotation). Treat runtime widths as ill-formed here so the checker rejects them instead of accepting `Float0`.

## Liaison triage — CONFIRMED against trunk — CHECKER SOUNDNESS HOLE
Confirmed in eval.rs: `is_ill_formed_float_width` matches `read_width(db, args[0])` → `Fixed(w)` checks
ADMITTED_FLOAT_WIDTHS, `Malformed` → true, but `NotConst` → **false** (not flagged). `read_width`
returns `NotConst` for a runtime width `(Float n)` (n a param/ref). So a runtime float width is NOT
rejected by the float-width check this very batch (#425 "float-width reject") added — and `reduce_ctor`
then clamps it to sentinel width 0 → a bogus `Float0` slips past `cdz check`. This contradicts the
intent that runtime widths are rejected at the annotation. FIX: treat `NotConst` as ill-formed here (a
float width must be a compile-time constant in the admitted set) so the checker rejects it. Checker
soundness — route to `corpus-bugfix` PM (width validation, eval.rs/infer.rs). Fix on `trunk`. Quote +
link in queue file.
