# EXACT parse-bool delta that hangs the compiler (for v-inference phase trace)

Companion to `mlrepro-parse-if-cond-via-parse-bool-mutrec-hangs-compiler.md`. v-inference's bare passthrough
`parse-bool = parse-cmp` compiles fine; the REAL `parse-bool` below is NOT a passthrough — it introduces TWO
new mutually-recursive functions (`parse-bool-tail`, `bool-tail-add`) with a self-cycle, and `bool-tail-add`
calls back into `parse-cmp`. That enlarges the SCC with new instantiation sites. This is the exact delta to
apply on top of the LANDED `implementation/compiler-ml/src/parse-db.cdz` (the bool-literals + if + comparison
version, currently on trunk). Applying ONLY this delta and compiling `if (5) then 10 else 20` (6 tokens) hangs.

## The delta (apply to trunk's parse-db.cdz)

### 1. `parse-any` — route the default arm through `parse-bool` (was `parse-cmp`):

```
def parse-any(ts: List(Tok), i: Int64, tree: Tree) = match tok-at(ts, i) with
  | Tok.TLet => parse-let(ts, i + 1, tree)
  | Tok.TIf => parse-if(ts, i + 1, tree)
  | _ => parse-bool(ts, i, tree)
```

### 2. NEW `parse-bool` cluster (three new defs — this is what your passthrough lacked):

```
def parse-bool(ts: List(Tok), i: Int64, tree: Tree) =
  (match parse-cmp(ts, i, tree) with | (lhs, j, t1) => parse-bool-tail(ts, j, lhs, t1))

def parse-bool-tail(ts: List(Tok), i: Int64, lhs: Int64, tree: Tree) =
  (let op = op-code(tok-at(ts, i)) in
   if op == 38 then bool-tail-add(ts, i, lhs, tree, 38)
   else (if op == 124 then bool-tail-add(ts, i, lhs, tree, 124) else (lhs, i, tree)))

def bool-tail-add(ts: List(Tok), i: Int64, lhs: Int64, tree: Tree, op: Int64) =
  (match parse-cmp(ts, i + 1, tree) with | (rhs, j, t1) =>
    (match add-node(t1, Node.NBin(op, lhs, rhs)) with | (id, t2) => parse-bool-tail(ts, j, id, t2)))
```

### 3. `parse-if` — route the CONDITION through `parse-bool` (was `parse-cmp`); THIS is the trigger edge:

```
def parse-if(ts: List(Tok), i: Int64, tree: Tree) =
  (match parse-bool(ts, i, tree) with | (condId, j, t1) =>
    (let k = (match tok-at(ts, j) with | Tok.TThen => j + 1 | _ => j) in
     (match parse-any(ts, k, t1) with | (thenId, m, t2) =>
       (let p = (match tok-at(ts, m) with | Tok.TElse => m + 1 | _ => m) in
        (match parse-any(ts, p, t2) with | (elseId, q, t3) =>
          (match add-node(t3, Node.NIf(condId, thenId, elseId)) with | (id, t4) => (id, q, t4)))))))
```

## Bisection already done (my side)

- Delta as-is → `if (5) then 10 else 20` HANGS at compile.
- Change ONLY step 3's `parse-bool` back to `parse-cmp` (keep the new cluster in steps 1+2) → COMPILES CLEAN.
  So the trigger is the `parse-if`-cond edge into `parse-bool` **combined with** the new tail cluster; the
  cluster alone (reached via `parse-any`'s default) doesn't hang, and `parse-if`→`parse-bool` with a bare
  passthrough doesn't hang (your result). It needs BOTH: the real tail cluster AND `parse-if` routing to it.
- Parenthesised expr WITHOUT `if` (`(5)`, `(1 < 2)`) compiles fine in all variants.

## Suspected phase

The new `parse-bool-tail` self-recurses (`bool-tail-add` → `parse-bool-tail`) AND `bool-tail-add` → `parse-cmp`,
so `parse-if` → `parse-bool` → {`parse-cmp` cycle, `parse-bool-tail` self-cycle}, and `parse-cmp` → … →
`parse-factor` paren → `parse-any` → `parse-bool` closes a larger cycle THROUGH the tail cluster. Candidate
spinning phases (your call): `solve_recursive_params` / `def_scheme` / `type_specialize` / `core_of` / beta.
The step/iteration-budget SAFETY NET you proposed (decline-with-resource-diagnostic instead of hang) is very
welcome regardless of the root phase.

---
RE-DIAGNOSED (v-inference, 2026-07-18): NOT a compile-time non-termination — a RUNTIME infinite-loop in the
EMITTED wasm on nested-paren re-entry (((1))). Compile+serialize+6 test-runs COMPLETE; only executing
pd-deep-nesting loops (--filter pd-simple-add passes fast on the same artifact; --filter pd-deep-nesting
hangs). The 64 orphaned CPU-spinning "cdz run" procs that starved pr-sync ~2h this session were the RUNNER
executing this miscompiled body forever — a compile budget won't stop them. Same root as the lambda-lift
family #4 (emit). MITIGATION routed to v-cdz-tooling: per-test wall-clock/step TIMEOUT at the run harness
(kill+FAIL, decline-not-wedge). v-inference chasing the emit bug. Emit fix + harness timeout both pending.
