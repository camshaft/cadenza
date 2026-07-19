# common-operator if-arm hoist reorders a trapping shared operand past a trapping cond → WRONG TRAP

**Severity:** correctness/soundness — a MISCOMPILE. Valid wasm, `cdz check`/`compile` clean, but the
program observes the WRONG trap kind. CONFIRMED by running (with a control).

**Where:** `implementation/seed/crates/rcdzc/src/lower.rs`, `hoist_common_arith` — **lower.rs:14028-14032**
(the guard block). Landed in `ba26196c9` ("rcdzc: hoist a common operator out of both if arms").

## The bug

`hoist_common_arith` rewrites `(if cond (op …p) (op …q))` → `(op …(if cond pᵢ qᵢ))`, pushing each
DIFFERING operand into its own `(if cond pᵢ qᵢ)` and sharing a `core_equiv` operand DIRECTLY (outside
the per-operand `if`). A shared operand therefore evaluates BEFORE `cond`.

Its guard only checks the cond-eval COUNT:

```rust
// lower.rs:14028
if diff != 1 && !is_trap_free(db, cond) {
    return None;
}
```

For `diff == 1` this lets a trapping `cond` through with NO ORDER CHECK. But `core_equiv` admits
trapping checked arith (a `/` by a runtime divisor is `core_equiv` to itself via the `Core::Arith`
arm), so a SHARED trapping operand positioned BEFORE the differing one is hoisted OUT and evaluated
before `cond` — preempting `cond`'s own trap with a different trap kind.

This is the EXACT twin of the hazard the sibling `hoist_common_ctor` had, fixed in `3e43b00eb` (batch
#385) by adding an ORDER check: "for a non-trap-free cond, require every shared payload BEFORE the
differing index to be trap-free." `hoist_common_arith` was written WITHOUT that guard — the commit
message even claims soundness citing only the count guard ("0 or ≥2 differing operands require a
trap-free cond"), missing the diff==1 order case its own sibling already learned about.

## Confirmed reproducer (RUN, with a control)

```
(module m
  (def (f (: e Int64) (: d Int64) (: a Int64) (: b Int64))
    (if (< (+ e 9223372036854775807) 0)   ; cond: OVERFLOWS at runtime when e > 0
        (+ (/ 100 d) a)                   ; shared lhs (/ 100 d) traps ÷0 when d=0; differing rhs a
        (+ (/ 100 d) b)))                 ; same shared lhs; differing rhs b  (diff == 1)
  (export f))
```

`cdz run … --call f --arg 1 --arg 0 --arg 5 --arg 7` (e=1, d=0):
- **Observed (buggy):** `trap: integer divide by zero` — the hoisted `(+ (/ 100 d) (if cond a b))`
  emits the lhs `(/ 100 d)` FIRST (arith emit order is lhs-then-rhs, `emit_checked_arith_to`
  select.rs:11010/11014), so ÷0 fires before `cond`.
- **Required (source):** `integer overflow` — the original `if` evaluates `cond` first; the
  `(+ e i64::MAX)` overflows before either arm (and its shared `(/ 100 d)`) is reached.

**CONTROL** — make the divide NON-shared (different divisor per arm, so the hoist can't lift it past
cond): `(if cond (+ a (/ 100 d)) (+ b (/ 101 d)))`, same args → traps **"integer overflow"** ✓. Only
difference vs the buggy case is whether the trapping operand is shared (which triggers the hoist),
proving the hoist is the cause.

Violates `spec/capabilities/core-semantics.md` §"A Trap Halts Execution At A Defined Point": *"The kind
of trap ... MUST be a deterministic function of the operation and its inputs"* — the trap KIND is
observable, so reordering two potential traps changes observable behavior.

## Fix (mirror the constructor-hoist fix `3e43b00eb`)

In `hoist_common_arith`, when `cond` is NOT trap-free, in addition to the `diff == 1` count check,
require every SHARED (core_equiv) operand PRECEDING the single differing operand to be `is_trap_free`.
Then `cond`'s trap remains the first observable one. (For the binary Arith the only preceding position
is lhs when the diff is at rhs; the unary Convert has a single operand so a diff==1 Convert has no
preceding shared operand and is unaffected.) The constructor hoist's block (lower.rs ~13809-13827) is
the template — same `first_diff` + `pairs[..diff_idx]` trap-free scan.

## Verified
Built `cdz`+`cdz-run` at trunk `3ba79db6b`, compiled + RAN both the buggy and control programs above;
trap kinds are as stated. This is a genuine miscompile, not a hunch.

<!-- RESOLVED 2026-07-15 (trunk@00f7d341e): hoist_common_arith order guard added (covers Arith + Compare + Convert via the shared guard point). A trapping shared operand no longer preempts a trapping cond — the overflow-cond case now traps overflow (cond-first), not the shared ÷0. Compare-head face pinned too. -->
