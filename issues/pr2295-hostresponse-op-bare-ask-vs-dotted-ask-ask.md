# PR #2295 review — rcdzc/src/tests.rs (v-effects) — OPEN — test-convention/diagnostic-clarity [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/2295 (close the both-perform gap + narrow to in-program performs;
the #2289 Copilot + breaker as6 follow-up = the `dc8e00aa8` work v-effects acked). Copilot 1 inline (id
3723968725, tests.rs:68292).

## the new as6 host-served pin sets `HostResponse { op: "ask", … }` (bare) but the contract documents `op` as a DOTTED `E.op` name (`ask.ask`) — bare makes diagnostics ambiguous if the response list is exhausted / more host ops are added (Copilot, tests.rs:68292) — convention [VERIFIED, LOW]
> `cdz_run::HostResponse.op` is documented as a dotted `E.op` name (and other tests in this file follow that
> pattern). Using just `"ask"` here makes diagnostics ambiguous if the host-response list is exhausted or if
> additional host ops are added later; `"ask.ask"` matches the intended contract for this effect/op pair.

VERIFIED the contract in source. `cdz-run/src/lib.rs:306` HostResponse doc: "The operation name (`E.op`,
dotted) pairs a response with its call…"; the `op` field doc (:311) is explicit: "The dotted operation name
the response answers (e.g. `ask.ask`) — for the ordered model + a mismatch diagnostic. This increment
consumes responses purely in ORDER (the op name is recorded for the diagnostic, not yet matched)." The #2295
pin (diff:275-276) writes `op: "ask".to_string()` — bare, not `ask.ask`. There's also an explicit in-file
precedent: tests.rs:67660 comments "Dotted `E.op` per the HostResponse.op contract (effect `io`, op `send`)"
and uses `io.send`/`io.get`.

CALIBRATION (honest): the convention is SOFT, not uniform — other pins in the same file use bare op names
(`h`, `emit`, `f` at 67396/88520/91103), and the doc itself says the op is "recorded for the diagnostic, NOT
yet matched" (ordered-consume), so a bare `"ask"` does NOT change the pin's behavior or its asserted value
today. So this is LOW / diagnostic-clarity + contract-alignment, not a correctness bug. Fix per Copilot:
`op: "ask.ask"` to match the documented `E.op` contract + the 67660 precedent, so a future mismatch
diagnostic (when the op name IS matched) reads unambiguously. v-effects owns rcdzc effects. PR OPEN → foldable
pre-merge; entirely v-effects' call whether to tighten the pin or leave the soft-convention bare form.
