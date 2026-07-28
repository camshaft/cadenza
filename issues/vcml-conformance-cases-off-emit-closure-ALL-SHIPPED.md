# vcml: ready-to-apply off-emit-closure conformance cases (for when base-pin clears)

Scratch prep by v-compiler-ml while Slice-B is host-blocked (v-inference owns the emit-cache collision) and
the div-left-assoc MR `985e20a13` is base-pinned in pr-sync's queue. These are validated conformance gaps in
`conformance-db.cdz` + `conformance-db-cx.cdz` (OUTSIDE emit-db's build closure → can't re-trip the host bug).
Apply the NEXT one the moment the queue clears (commit + send, one per tick).

## 📤 READY #1 — SENT tick-43 (MR `87d5dfa8d`, floor 65→66) — `if` in LET-VALUE position (a CIf as a CLet value)
STATUS: applied to a scratch working-tree copy on trunk-equivalent base and RAN — `cdz check` clean on both
files, `cdz test conformance-db-cx` = **25 passed 0 failed** incl. `PASS conformance-cx-if-in-let-value`
(value == 15). Reverted to keep HEAD = queued MR ref clean. Next tick: apply verbatim (blocks below) + commit
+ send — no re-validation needed.

Gap: existing cases have `if` as the whole expr / in a branch / a let bound to a comparison or literal — but
NEVER a let bound to a full `if`-expression (the lowering path CLet(value = CIf(...))). Genuine untested shape.

- Program: `let x = (if 1 < 2 then 10 else 20) in x + 5` → x = 10 → **15**.
- Case builder (place near c-cx-var-in-later-binding):
```
/// `let x = (if 1 < 2 then 10 else 20) in x + 5` → 15 — an `if`-expression in LET-VALUE position (the let is
/// bound to a full conditional, not a comparison/literal). Pins that lowering handles a CIf as a CLet value +
/// eval binds the branch result. Distinct from c-cx-bool-let-as-cond (let bound to a Bool comparison) and the
/// if-in-cond/if-in-branch cases.
def c-cx-if-in-let-value() =
  Case.Case(
    [
      Tok.TLet, Tok.TName(1), Tok.TEq,
      Tok.TLParen, Tok.TIf, Tok.TNum(1), Tok.TOp(60), Tok.TNum(2), Tok.TThen, Tok.TNum(10), Tok.TElse, Tok.TNum(20), Tok.TRParen,
      Tok.TIn, Tok.TName(1), Tok.TOp(43), Tok.TNum(5)
    ],
    Expect.Runs(15)
  )
```
- corpus() entry: add `c-cx-if-in-let-value(),`
- size floor: bump 65 → 66 (cf-corpus-size-floor trap message too).
- export from conformance-db + import in conformance-db-cx.
- @test in conformance-db-cx:
```
@test
def conformance-cx-if-in-let-value() =
  if check-case(c-cx-if-in-let-value()) then unit else trap("let x=(if 1<2 then 10 else 20) in x+5 → 15")
```
- Token names confirmed on trunk parse-db: TIf/TThen/TElse/TIn/TLParen/TRParen present; TOp(60)=`<`, TOp(43)=`+`.
- ⚠️ VERIFY the paren-wrapped if parses in let-value on trunk (bug#4 nested-paren was fixed 26e3471ac; if the
  paren form declines, drop the parens: `let x = if 1 < 2 then 10 else 20 in x + 5` — but the paren form is the
  stronger pin). Gate: conformance-db + conformance-db-cx both green before send.

## Backlog ideas (validate before use)
- unary-minus precedence vs `*`: `-2 * 3` → is it `(-2)*3 = -6` or `-(2*3) = -6`? same value — NOT distinguishing, skip.
- `-(2 + 3)` paren-negation: `TNeg TLParen TNum2 TOp43 TNum3 TRParen` → -5. Pins TNeg over a paren-group.
- nested if in ELSE already covered (c-if-nested-else); nested if in THEN via paren covered (c-cx-paren-if-in-then-branch).

## 📤 READY #2 — SENT tick-55 (MR 2be5bd64b, floor 66→67) — unary minus over a PAREN-GROUP
Gap: existing neg cases negate a literal (`--5`) or a bare operand (`-12/4`, `-7%3`); none negate a
parenthesized group. `-(2 + 3)` → -5 pins that TNeg binds the whole paren sub-expression (not `-2+3=1`).
STATUS: scratch-validated `cdz test conformance-db-cx` = 25/0 incl `PASS conformance-cx-neg-paren-group`; reverted.

- Case builder (place after c-cx-div-left-assoc / c-cx-if-in-let-value):
```
/// `-(2 + 3)` -> -5 — unary minus applied to a PARENTHESIZED group (an NNeg over a paren-wrapped NBin).
def c-cx-neg-paren-group() =
  Case.Case(
    [Tok.TNeg, Tok.TLParen, Tok.TNum(2), Tok.TOp(43), Tok.TNum(3), Tok.TRParen],
    Expect.Runs(0 - 5)
  )
```
- corpus() entry: `c-cx-neg-paren-group(),`  · size floor bump (whatever N is current) +1  · export + import.
- @test in conformance-db-cx:
```
@test
def conformance-cx-neg-paren-group() =
  if check-case(c-cx-neg-paren-group()) then unit else trap("-(2 + 3) → -5 (neg binds the paren-group)")
```

## Landing order (both PROVEN, one per tick once pr-sync unwedges + my div-left-assoc MR lands):
1. Apply READY #1 (c-cx-if-in-let-value), size floor 65→66. 2. Then READY #2 (c-cx-neg-paren-group), 66→67.

## ✅ READY #3 — VALIDATED tick-42 (proven 25/0) — `if`-expression as an ARITH OPERAND
Gap: if-in-let-value (READY#1) puts a CIf in a CLet value; this puts a CIf directly under a CBin (arithmetic
operand). `(if 1 < 2 then 3 else 4) * 10` → 30 (cond true → 3, 3*10). Pins lowering of CIf inside CBin.
STATUS: scratch-validated conformance-db-cx 25/0 incl `PASS conformance-cx-if-as-arith-operand`; reverted.

- Case builder:
```
/// `(if 1 < 2 then 3 else 4) * 10` -> 30 — an `if`-expression as the LEFT OPERAND of a multiplication.
def c-cx-if-as-arith-operand() =
  Case.Case(
    [
      Tok.TLParen, Tok.TIf, Tok.TNum(1), Tok.TOp(60), Tok.TNum(2), Tok.TThen, Tok.TNum(3), Tok.TElse, Tok.TNum(4), Tok.TRParen,
      Tok.TOp(42), Tok.TNum(10)
    ],
    Expect.Runs(30)
  )
```
- corpus() + size floor +1 + export + import + @test:
```
@test
def conformance-cx-if-as-arith-operand() =
  if check-case(c-cx-if-as-arith-operand()) then unit else trap("(if 1<2 then 3 else 4) * 10 → 30")
```

## Landing order (all 3 PROVEN, one per tick after div-left-assoc MR lands):
#1 if-in-let-value (floor 65→66), #2 neg-paren-group (66→67), #3 if-as-arith-operand (67→68). 3-tick pipeline.

## ✅ READY #4 — VALIDATED tick-44 (proven 26/0) — unbound name in an IF-BRANCH → Declines
Gap: existing declines cover unbound-in-arith / bool-in-relational / div-by-zero, but NONE put an unbound
name INSIDE an if-branch. `if 1 < 2 then x else 0` (x unbound) → Declines. Pins that the type-gate descends
INTO if-branches (not just the condition) — a decline-case, higher-signal than another precedence pin.
STATUS: scratch-validated conformance-db-cx 26/0 incl `PASS conformance-cx-unbound-in-if-branch-declines`; reverted.

- Case builder:
```
/// `if 1 < 2 then x else 0` (x unbound) -> Declines — the TAKEN branch references an unbound name.
def c-cx-unbound-in-if-branch-declines() =
  Case.Case(
    [Tok.TIf, Tok.TNum(1), Tok.TOp(60), Tok.TNum(2), Tok.TThen, Tok.TName(1), Tok.TElse, Tok.TNum(0)],
    Expect.Declines
  )
```
- corpus() + size floor +1 + export + import + @test:
```
@test
def conformance-cx-unbound-in-if-branch-declines() =
  if check-case(c-cx-unbound-in-if-branch-declines()) then unit else trap("if 1<2 then x else 0 declines (unbound in branch)")
```

## UPDATED landing order — 3 PROVEN cases remain (if-in-let-value #1 is SENT, MR 87d5dfa8d):
Each lands one/tick; bump the size-floor to (current corpus len)+1 at apply-time (don't hardcode — it shifts as each lands):
  A) #2 neg-paren-group → -5
  B) #3 if-as-arith-operand → 30
  C) #4 unbound-in-if-branch-declines → Declines
All independently validated; apply in any order, one per tick, after the prior MR lands.

## 🔧 MUST-FIX (github-liaison PR#772, tick-57): inverted size-floor trap message
conformance-db.cdz size-floor guard: `if List.len(corpus()) == N then unit else trap("corpus has N cases")`
— the trap fires in the ELSE (count != N) but the message asserts the invariant HELD. INVERTED/misleading.
Copilot flagged, liaison confirmed. Diagnostic-only (guard logic correct). Fold the fix into the NEXT
conformance MR (it's base-pinned behind my queued neg-paren-group MR now). Corrected form:
  `if List.len(corpus()) == N then unit else trap("conformance corpus size changed — expected N cases")`
Apply this message fix in the SAME edit when I bump the floor for READY #3 (if-as-arith-operand) — one MR,
both the new case AND the message fix. Do NOT re-propagate the inverted phrasing.
