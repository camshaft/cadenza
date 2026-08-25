# General compile-time const-evaluation — closing P4 Path 2 by interpreting total functions, not unrolling shapes

**Status:** design/scoping only — nothing landed. Written 2026-08-25 by the `v-compiler-primitives` fleet
agent, non-interactively, on the operator's standing directive to "drive the compiler's general const-fold
to completion — keep pushing on this as the direction." Doc-only PR (platform lane, direct to `main`); it
changes no compiler code. Line numbers are landmarks at trunk `d8b8a6eb9`.

> **Why now.** The incremental "recursive const-fold" (`#3344` const-list unroll, `#3365` filter/let-inline,
> `#3370` nested-shape gate) made real progress — single-type contract-id folds byte-exact to the `#3238`
> golden, and each *atom* (unconditional build, search, filter, tail-recursion, comment-unwrap) now folds.
> But `v-platform` (the P4 consumer) verified on merged `main` that the atoms **do not COMPOSE**, and their
> real self-reflected transform inherently needs a composition. The boundary has shifted three times under
> incremental fixes (const-list → scalar/sum → non-tail → composition), which is the signature of a missing
> *capability*, not a missing *shape*. This doc proposes the capability: a bounded interpreter that
> const-evaluates any total function applied to compile-time-constant arguments to a constant value.

---

## 1. The problem: the unroll-and-refold does not compose

The current mechanism (`lower.rs` ~`1404` `Resolved::Apply` `Err(msg)` arm) is **unroll-and-refold**:

1. gate — the callee has a `const` parameter of a foldable shape (`has_const_foldable_param`) and every arg
   `is_const_value`;
2. `apply_lambda_one_level_recursive` (β-substitute the args into the body once, `eval.rs:1064`);
3. `core_of` the reduced body (which re-enters this same arm for any residual self-call);
4. accept only if `core_is_const_value(folded)` (`lower.rs` ~`24056`), else fall through to a runtime
   call / decline.

This mixes **AST-level substitution** (step 2) with **Core-level folding** (step 3) and leans on
`should_keep_binding`'s const-let inline (`#3365`, gated by `Db::const_fold_unroll`). It folds each atom, but
**re-entrant composition** — one unroll evaluated *inside* another unroll's `core_of` / `should_keep_binding`
— fails in a growing set of cases:

| shape | standalone / top-level / runtime fn | inside an outer const-fold unroll |
|---|---|---|
| tail recursion (`dec`, `unwrap`) | folds | **folds** (`#3370`) |
| non-tail accumulation (`tri = n + tri(n-1)`) | folds | **declines** (CDZ0201) |
| nested recursion result **let-bound + carried through a filter** (`let g = peel(h) … prepend(tail, g)`) | folds | **declines** (`Ast.encode of a runtime AST value`) |
| a recursion consuming another recursion's const result over `Ast.module` (two-pass) | folds over a literal | **declines** |

Minimal reproductions (all type-check via `cdz check`; the decline is const-fold-only, surfaced by the
const-demanding `Ast.encode` / a `const` param):

- **non-tail nested** (`spec/semantics/09-functions.sexp`, the `todo` pinned by PR `#3374`): a `const (List
  Int64)` build calling `tri` (self-call inside `(+ n …)`) per element declines, though `tri` folds
  standalone and `(+ (tri 2) (tri 3))` folds at top level.
- **let-bound nested result in a filter** (`/tmp/tnest_g.cdz`, `v-platform`): `collect` binds `let g =
  peel(h)` (a nested tail-recursion result) and `let tail = collect(t)`, then filters using both and prepends
  `g` — declines over a literal, though `peel` alone folds (`28`) and `peel` in an *unconditional* build
  folds (`25`).
- **two-pass over `Ast.module`** (`/tmp/p4twopass.cdz`, `v-platform`): `keep-types(unwrap-all(forms))` — the
  same composition that folds over a literal declines when the input is `Ast.module`-derived.

The common cause is structural: **a const result produced by one bounded unroll does not reliably flow as a
constant into an enclosing unroll**, because the enclosing frame reasons over the *unsubstituted AST call*
(which still looks recursive) and/or `should_keep_binding` calls `core_of(init)` on a nested recursive init
that re-enters the unroll in a context the one-level scheme cannot complete. Patching each shape (a 5th, 6th
gate) is diminishing-returns whack-a-mole; `v-platform`'s comment-tolerant type-collector *fundamentally*
needs unwrap-per-element composed with a type-filter over `Ast.module`, so no single-recursion formulation
avoids the composition.

## 2. The capability: bounded const-evaluation of a total function

Replace "unroll one β-level and refold" with **evaluate to a value**: given a call whose callee is a total
function and whose arguments are all compile-time constants, *interpret* the function body directly to a
constant `Core` value (or decline), recursing into nested calls, `match`, `let`, arithmetic, and the
collection/`Ast` constructors — bounded by the existing reduction budget so a non-terminating or
explosively-growing evaluation declines rather than hangs.

The key inversion: the interpreter works on **values** end-to-end. A nested call `peel(h)` is *evaluated to a
constant value* and that value is what the caller sees — there is no AST call left to "look recursive," and no
`should_keep_binding` decision to get wrong, so composition is automatic: `collect` evaluates `peel(h)` to a
constant `Ast` value, binds `g` to it, evaluates the predicate on it, and conses it — all as values.

### 2.1 Entry point and gate (unchanged surface)

Keep the exact activation conditions the incremental work established, so nothing outside a genuine const
fold is affected:

- fires only in the `Resolved::Apply` `Err(msg)` arm (a call the ordinary β-reducer declined as recursive),
- only when the callee declares a `const` parameter (the const-DEMAND signal — the author marked it
  compile-time; ordinary recursive-generic / RRB / dict-consumer producers have no `const` param and are
  untouched),
- only when every argument `is_const_value`.

On success it returns the constant `Core`; on decline (budget exhausted, a runtime value reached, a partial
value) it falls through to today's runtime-call / decline path **unchanged**. So this is strictly a
*completeness* improvement on the accept path — it never changes a program that compiles today.

### 2.2 The evaluator

A tree-walking interpreter over the resolved form (reusing `eval.rs`'s `beta_reduce` / `apply_lambda` /
`resolved_of` rather than a new IR):

- **literals / constructors** → the corresponding `Core` const (`ConstInt`/`ConstStr`/`ConstBytes`/
  `BytesOf`/`ListNew`/`SumNew`/`Record`/`MapNew`/`SetOf`/`Unit`) — reuse `core_is_const_value`'s value
  algebra as the value domain.
- **`let`** → evaluate the init to a value, bind it in the environment, evaluate the body. (No
  `should_keep_binding` — a const environment binding is just a value; the code-size heuristic is irrelevant
  at compile time and the value folds away.)
- **`match` / `if`** → evaluate the scrutinee to a value, select the arm by structural match, bind pattern
  binders (incl. `(list h .. t)` element/rest) to the sub-values, evaluate the arm.
- **arithmetic / comparison / string / bytes / list ops** → evaluate operands to values, apply the op's
  const semantics (the folds `lower_arith` / `lower_*` already implement — factor the value→value cores out
  or call them on materialized operands).
- **call to a total function** → evaluate the args to values, β-bind the params, evaluate the body
  recursively. Self- and mutual-recursion are permitted (unlike the ordinary β-reducer's blanket
  `is_recursive` decline) because the **budget** bounds it: each evaluation step debits `Db::reduce_nodes`
  (the existing cumulative `REDUCE_NODE_BUDGET`) and nests under `enter_reduction` (the existing
  `REDUCE_DEPTH_LIMIT`); past either, decline. A shrinking total recursion terminates within budget; a
  non-terminating one declines exactly as the current unroll does. (This is the same soundness argument
  `#3344` already relies on — only the driver changes from one-level-refold to direct evaluation.)

### 2.3 What it subsumes

Everything the incremental work added becomes a special case: the const-list unroll, the scalar/sum gate
broadening, the filter let-inline, and the nested-shape gate are all just "evaluate a total function on
const args." The `Db::const_fold_unroll` flag and the `should_keep_binding` const-inline arm (`#3365`) can be
**retired** once the evaluator is the accept path (they exist only to make the refold compose; the evaluator
composes natively). The `has_const_foldable_param` gate stays as the cheap activation pre-check.

## 3. Integration & migration

1. Implement the evaluator behind the existing gate, returning `Option<Core>` (a constant value or `None`).
2. In the `Err(msg)` arm, try the evaluator first; on `Some(const)` return it; on `None` keep the current
   unroll-and-refold as a fallback, then the runtime/decline path. (Belt-and-suspenders during bring-up.)
3. Once the corpus + `v-platform`'s acceptance suite are green **through the evaluator**, delete the
   unroll-and-refold fallback and the `const_fold_unroll` flag / `should_keep_binding` inline arm.
4. Gate on all three targets (`wasm` / `rust` / `rust-async`) — the change is in shared front-end `core_of`,
   so it is backend-agnostic, but the full battery is the regression catch (the const-fold gate has a
   documented multi-regression history when broadened carelessly).

## 4. Risks

- **Budget starvation** — a broad evaluator could burn the shared `reduce_nodes` on a large-but-terminating
  fold, or a mistaken activation could enter the evaluator on a non-const-demanding recursion. Mitigation:
  keep the `const`-param + all-args-const gate exactly as today (the const-DEMAND signal already excludes the
  RRB/dict/generic producers that caused the original 37→0 regression tuning); debit the existing cumulative
  budget so the whole compile is bounded.
- **Op coverage** — the evaluator must cover every op a const transform can use (arith, compare, string,
  bytes, list/map/set, `Ast` constructors/destructors, `Ast.encode`). Mitigation: reuse the existing
  `lower_*` const-fold value logic; decline (fall through) on any op not yet covered, so an uncovered op is a
  clean decline, never a miscompile.
- **Soundness of permitting recursion** — permitting self-recursion in the evaluator is the one place the
  ordinary β-reducer deliberately declines. Mitigation: the budget + depth guards are the same ones that
  already make `#3344` safe; a value is accepted only when evaluation *completes* within budget to a constant.

## 5. Acceptance

Path 2 closes when, through the evaluator:

- `v-platform`'s `general-transform.cdz` folds **byte-exact** to the `#3238` golden (their full self-reflected
  contract-id transform: comment-`unwrap` + type-`filter` + `find-pragma` search over `Ast.module`, then
  `Blake3.of(Ast.encode(...))` with the `0x01` tag);
- `/tmp/tnest_g.cdz` (let-bound nested result in a filter) and `/tmp/p4twopass.cdz` (two-pass over
  `Ast.module`) fold over a literal / `Ast.module`;
- the `todo` corpus case pinned by PR `#3374` (non-tail nested) flips to `pass`;
- the full corpus gate is green on all three targets with zero regressions.

`v-platform` owns the byte-exact verification and has offered to pair on the acceptance suite.

## 6. Ownership & sequencing

`v-compiler-primitives` (this vertical) owns the evaluator (`P2` const-execution, `lower.rs` / `eval.rs`; NOT
`wit_world.rs`). It is a focused multi-tick build: (a) evaluator skeleton + literals/arith/`let`/`match` +
budget wiring, gated behind the existing activation, with the unroll as fallback; (b) call-recursion +
list/`Ast` ops until `tnest_g` + the `#3374` `todo` fold; (c) `Ast.module` two-pass + `general-transform.cdz`
byte-exact; (d) retire the fallback + `const_fold_unroll` flag. Each step is corpus- and
acceptance-suite-gated.
