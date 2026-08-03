# PR #1652 review comments — rcdzc/src/opt.rs (v-rust-backend) — OPEN

https://github.com/camshaft/cadenza/pull/1652 (pin the two O2 CSE soundness guards with direct pass tests).
Both Copilot points VERIFIED against opt.rs — the witnesses don't exercise the path they claim to pin
(the guards may be fine, but these tests wouldn't catch a regression in the intended guard).

## 1. Heap-type witness never reaches the heap-type filter — pure-scalar gate skips it first (Copilot, opt.rs:960) — test-precision [VERIFIED]
> This witness claims to pin the heap-type exclusion in `candidate_groups`, but `(list x x)` resolves to
> `Resolved::List`, which makes `body_is_pure_scalar` return false and causes the pass to skip the entire
> body before it ever reaches the heap-type candidate filter.

VERIFIED: `body_is_pure_scalar` (opt.rs:269) REJECTS `Resolved::List` (in the "Everything else … `List` …
is REJECTED" → `_ => false` set). So a body containing `(list x x)` fails the pure-scalar ELIGIBILITY gate
and the pass skips it BEFORE `candidate_groups`' heap-type filter ever runs — the witness exercises the
wrong guard. To actually pin the heap-type exclusion, build a repeated heap-typed CANDIDATE inside an
ELIGIBLE (pure-scalar) body — e.g. repeat a heap-typed projection `(. r xs)` (type `List Int64`) across two
non-CSE-able prim calls, so the only potential sharing is the heap handle itself. MED test-precision (a
green test that doesn't guard what it names).

## 2. "nested capturing def" witness doesn't actually capture (Copilot, opt.rs:926) — test-precision [VERIFIED]
> This test claims a "nested capturing def", but the local `inner` function body doesn't actually capture
> anything from `outer` (it only references `x`). That makes the witness less precise and the surrounding
> `captured_ref` poisoning comment misleading.

VERIFIED: the def is `(def (inner (: x Int64)) (+ (& x 7) (& x 7)))` — `inner`'s body references only its
OWN param `x`, never `n`/`k` from `outer`. So it isn't a capturing def, and the "captured_ref poisoning"
rationale doesn't apply. Make `inner` capture an outer value (e.g. `k`) so the test really exercises the
capture/lambda-lift hazard the comment describes. LOW-MED test-precision. Fix-forward (both are test-only;
the underlying guards are presumably correct — the witnesses just need to actually hit them).
