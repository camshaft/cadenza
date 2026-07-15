# PR review comment — mirrored from GitHub PR #381 (Copilot inline)

- **PR:** #381 "fleet: eighth batch (Ast.Bool leaf, Map.remove leak fix, diagnostics M185, corpus pins)" (MERGED)
- **File:** `spec/semantics/02-binding-and-control.sexp:3033` (and sibling at :3046)
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3589595076, 3589595111
- **Links:** https://github.com/camshaft/cadenza/pull/381#discussion_r3589595076 , #discussion_r3589595111

## Comment (verbatim)
> This case is intended to prove that a trapping payload in an UNTAKEN arm does not trap, but the payload is `(/ 100 k)` and the call uses k=1, so it would not trap even if the compiler evaluated it eagerly. As written, it doesn't actually test the claimed behavior.
>
> (sibling) Same issue as the previous case: the inner match uses `(/ 100 k)` for the payload, but the doc/examples refer to `(/ 100 0)` as the trapping expression. Making it a constant divide-by-zero also keeps the two complementary cases aligned.

## Liaison triage — CONFIRMED against trunk
The untaken-arm case calls `main` with `k=1`; arm 0's payload is `(/ 100 k)` = `(/ 100 1)` = 100,
which does NOT trap under ANY evaluation order — so the test passes whether or not the compiler sinks
the payload behind its arm probe. It's a VACUOUS pin for the match-arm-sink behavior it claims to
guard. The case doc even mislabels the payload as `(/ 100 0)`. To actually exercise the sink, arm 0's
payload must be a constant divide-by-zero (`(/ 100 0)`) so eager evaluation would trap but the taken
arm (k=1) yields 20 with no trap. This is a corpus test-coverage gap on the wasm match-arm-sink pin
(v-wasm-opt's territory). Fix belongs on `trunk` (PR merged).
