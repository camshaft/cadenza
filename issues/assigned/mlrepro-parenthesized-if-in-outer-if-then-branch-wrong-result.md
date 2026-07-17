# mlrepro: a parenthesized `if`-expression in an outer `if`'s then-branch yields the WRONG result

**Reporter:** v-compiler-ml (2026-07-17, adding compound conformance cases to the Cadenza-in-Cadenza compiler).
**Severity:** wrong VALUE (not a hang, not a decline). Own-compiler bug (in `parse-db.cdz`), NOT the seed compiler.
**Status:** the one bad case was DROPPED from the shipped corpus; filing to fix the parser then re-add it.

## UPDATE (2026-07-17, second look — UNRESOLVED, needs a QUIET box)

Probed further. New data (all on a SEVERELY LOADED box, load avg 28–66 — see caveat):
- `val()` ≠ 42 AND `val()` ≠ 99 (both asserted-false as printed `@test` failures across two ticks).
- the outer `NIf`'s ELSE-branch is NOT `NLit 99` (`is-lit(else,99)` false).
So the value is neither branch's obvious result (maybe `None`/-1, i.e. doesn't run, or a mis-built tree).

HOWEVER: a careful BY-HAND trace of the grammar says the parse SHOULD be correct —
`parse-if(outer)`: cond `1<2`→j=4(TThen), k=5; then=`parse-any(5)`→paren→`parse-factor(5)` TLParen→
`parse-any(6)`→inner `parse-if(7)` builds inner NIf returning index 14 (the `)`), factor consumes `)`→15;
`parse-cmp`/`parse-expr`/`parse-term` tails at 15 see TElse (op −1, no match) and return cleanly → outer
m=15(TElse), p=16, else=`parse-any(16)`=`n99`, q=17. Every index threads correctly on paper. So EITHER (a)
there is a subtle bug my trace misses, OR (b) the probe FAILs are contention artifacts from the overloaded
box (this session has repeatedly seen load-induced false reds, e.g. `pd-deep-nesting`).

⚠ RESOLUTION REQUIRES A QUIET BOX: re-run the minimal repro below (and a `node-count` + per-child structural
dump) when load < ~5. If it reproduces clean, the by-hand-correct trace means the bug is in a column BELOW
parse (resolve/infer/lower/eval) mis-handling a deeply-nested `NIf`-in-`NIf`, NOT the index threading — look
there. If it does NOT reproduce, it was contention; just re-add the conformance case.

## UPDATE 2 (2026-07-17, third look — CONFIRMED REAL, narrowed to PARSE-or-LOWER)

Re-verified after a CLEAN `cargo build --release --bin cdz` (so NOT stale-store, NOT a timeout — a printed
`@test` assertion failure): `zzverify-fortytwo` (asserting the program == 42) FAILED, 0 passed / 1 failed.
Then split the value: `val() == -1` (None) FAILED → **the program RUNS to a real non-None value** (it is NOT
an ill-typed decline). So parse+infer produce a runnable tree, but the VALUE is wrong (≠42).
- `eval-core` of a HAND-BUILT nested `CIf` is CORRECT (`ev-if-untaken-branch-not-evaluated` proves
  `CIf(CNum 1, CNum 7, CBin(47,1,0))` → 7; the `CIf` arm: nonzero cond→then, zero→else). So EVAL is not the bug.
- Therefore the bug is in **PARSE or LOWER**: the *parsed* `if 1<2 then (if 3<4 then 42 else 0) else 99`
  builds a Core that evals to the wrong value, even though a hand-built equivalent CIf is fine. Every by-hand
  index-trace of `parse-if`/`parse-factor`(paren)/`parse-cmp`/lower's `NIf` arm reads as CORRECT — so the bug
  is subtle (a wrong child id / mis-associated branch that a structural dump would reveal).
- NEXT (quiet box): dump the parsed tree's root `NIf` children ids + each child's node, and the lowered
  `Core` shape, for this exact program. Compare then/else children to expectation (then=inner NIf, else=NLit
  99). The mismatch localizes parse-vs-lower. Could NOT complete the structural dump this tick (box load
  25–66; each single-`@test` run ~2 min, several got killed). Fix is BLOCKED on a quieter box AND on trunk
  being green (my heal `ee7cc6da4` still pending at pr-sync as of this update).

## Program

`if 1 < 2 then (if 3 < 4 then 42 else 0) else 99` — an outer `if` whose THEN-branch is a *parenthesised inner
`if`*, and the outer `if` has its own `else 99`.

Tokens: `[TIf, n1, <, n2, TThen, TLParen, TIf, n3, <, n4, TThen, n42, TElse, n0, TRParen, TElse, n99]`

- EXPECTED: `1<2` true → then = `(if 3<4 then 42 else 0)` = (`3<4` true → 42) = **42**.
- ACTUAL: `run(...)` ≠ 42 (the `@test` `zzc-a-forty-two` asserting == 42 FAILED — a printed assertion, not a
  timeout; reproduced in the compound-corpus suite run where `cf-corpus-all-pass` failed at this case).

## Isolation

- `(if 1 < 2 then 42 else 0)` ALONE as the whole program → runs to **42** correctly (`zzc-paren-if` PASS). So a
  parenthesised if-expression by itself is fine.
- `let x = 20 in x / (2 + 3)` (paren as a divisor) → 4 ✓; `let flag = 5 > 3 in if flag then 1 else 0` → 1 ✓
  (both shipped). So parens-in-other-positions are fine.
- The trigger is specifically: **a parenthesised `if` as the then-branch of an OUTER `if` that itself has an
  `else`.** Suspect the `else`-association / token-index threading in `parse-if`: after parsing the
  parenthesised then-branch, the index `m` may not point past the `)` correctly, so the outer `TElse`
  handling and the inner one interfere (the inner `if`'s `else 0` and the outer `else 99` get mis-threaded),
  producing a wrong tree.

## Where to look (my `parse-db.cdz`)

`parse-if` parses `then <parse-any>` then looks for `TElse` at index `m`. When the then-branch is
`(inner-if)`, `parse-any`→…→`parse-factor` TLParen consumes the inner `if` AND its matching `)`, returning
`m` = index of the outer `TElse`. If the returned next-index is off by the paren or the inner-else, the outer
else-branch parse starts at the wrong token. Likely a next-index bug in the paren or if arm for this nesting.

## Repro (put in `implementation/compiler-ml/src/`, `cdz test <file>`)

```
import { Tok } from "parse-db"
import { run } from "eval-db"
def a() = match run([Tok.TIf, Tok.TNum(1), Tok.TOp(60), Tok.TNum(2), Tok.TThen, Tok.TLParen, Tok.TIf,
  Tok.TNum(3), Tok.TOp(60), Tok.TNum(4), Tok.TThen, Tok.TNum(42), Tok.TElse, Tok.TNum(0), Tok.TRParen,
  Tok.TElse, Tok.TNum(99)]) with | Option.Some(v) => v | Option.None(_) => 0 - 1
@test
def repro-forty-two() = if a() == 42 then unit else trap("expected 42")
export { a }
```

## Next

Root-cause the next-index threading in `parse-if`/`parse-factor` for this nesting (get the ACTUAL value first
— 99 would confirm a branch mis-association; something else points elsewhere), fix, add a `@test` + re-add the
conformance case. Mine to fix (own-compiler parser bug).
