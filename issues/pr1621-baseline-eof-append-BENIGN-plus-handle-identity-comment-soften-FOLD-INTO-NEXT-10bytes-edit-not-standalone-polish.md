# PR #1621 review comments — spec/semantics/.gate-baseline{,-rust,-async} + 10-bytes.sexp (corpus-bugfix) — MERGED

https://github.com/camshaft/cadenza/pull/1621 (pin adv-66 — let-bound Bytes.compact read twice).

## 1. New baseline entry appended at EOF vs case-order (Copilot, .gate-baseline:5594 + -rust + -async) — LOW/verify-only
> This new baseline entry is appended at the end of the file, but this suite's baseline appears to be
> ordered to match the case execution order.

CAVEAT — likely BENIGN: the baseline is a verdict\tdescription set, matched by CONTENT-key (not position),
and `cargo xtask gate --save` REGENERATES the whole file's order. So an EOF-append is cosmetic and
self-corrects on the next `--save` — it does NOT affect gate correctness. (I checked: the file is only
roughly description-ordered, not a clean sort, so "execution order" is itself approximate.) If corpus-bugfix
wants tidy order, a `gate --save` regen fixes all 3 files at once. NOT a functional bug — verify-only.

## 2. Rationale comment asserts Bytes.compact handle-identity/aliasing as fact (Copilot, 10-bytes.sexp:474) — doc
> The new rationale comment asserts as a fact that `Bytes.compact` returns the same handle / aliases its
> operand. Handle identity isn't observable at the language level and may change.

Fair — handle identity is an impl detail, not a language-observable guarantee; the .sexp rationale (a
corpus doc) shouldn't state it as fact. Soften to "shares the operand's resident handle in the current
impl" or drop the aliasing claim. LOW/doc (corpus rationale prose).
