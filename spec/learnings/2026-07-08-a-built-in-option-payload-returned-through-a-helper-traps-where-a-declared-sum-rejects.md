# A built-in Option payload returned through a helper traps where a declared sum rejects

*2026-07-08*

**What happened.** The payload-shape-through-a-function-return gap (surfaced by the HOL Light
kernel spike for declared sums) takes a *worse* form on the built-in `Option`. `(tuple.0 (get
(Some (tuple 7 8))))`, where `get` binds `(Some p)`'s payload and returns it, emits a VALID
component that TRAPS at run time — the program is well-typed and its value is 7 (both inline
routes confirm it). The declared-sum companion (`Box.B` carrying a tuple, extracted through
`unbox`) instead REJECTS the projection with CDZ0201 ("tuple access on a non-tuple"). Same root
gap, two different observables — and the built-in one is the worse of the two: a running
component that traps rather than a refusal to compile.

**Why it is a break.** The program is well-typed: `(Some (tuple 7 8))`'s payload is a two-tuple,
and consuming it inline — `(match (Some (tuple 7 8)) ((Some p) (tuple.0 p)) …)` or via a
tuple-pattern `((Some (tuple a b)) a)` — both yield 7. So the recorded outcome is 7. The seed
does not thread the payload's tuple shape through `get`'s bare return, and instead of DECLINING
(scored todo, the reject-don't-miscompile floor) it emits a component whose `tuple.0` traps out
of the tuple's expected slots — an emit-a-broken-component violation of decline-don't-miscompile
(spec/learnings/2026-07-03-decline-do-not-miscompile.md). A trap where the program has a value is
a miscompile, not an honest frontier.

**The two failure modes of one gap.** Both the declared-sum and built-in-sum paths fail to recover
a compound payload's shape when it is returned by a bare `match`-arm binder (`((Ctor p) p)`) and
projected at the call site. But they fail *differently*:
- declared sum (`Box.B t` → `unbox` → `tuple.1`) → the static `tuple.N`-on-a-non-tuple check fires
  because the returned value's static type is unknown/opaque, so it REJECTS (CDZ0201);
- built-in `Some` (→ `get` → `tuple.0`) → the check does not fire (the payload is inferred to carry
  a heap/compound shape well enough to pass the static gate), so codegen proceeds and emits a
  `tuple.0` access that TRAPS at run time.
The divergence means the built-in path slips past the static gate that catches the declared path,
and lands in codegen with a shape it cannot honor — the failure surfaces as a runtime trap instead
of a compile-time rejection. A single "can I thread this payload's shape through the return?" gate
would make both DECLINE uniformly, rather than one rejecting and the other trapping.

**The lesson.** When a shape-recovery gap has both a static-check path and a codegen path, the two
can disagree on the *observable* of the same missing capability — reject vs trap — and the trap is
the dangerous one because it means a broken component was emitted. The inline controls compiling
correctly (7) prove the value is representable; the gap is purely the bare return, and the fix must
route the return through the same decline the declared-sum case should take, not let the built-in
payload fall through to a trapping projection. The give-away that the two paths diverged: the
declared case FAILs the gate as "wrongly rejected a valid program (CDZ0201)" while the built-in
case FAILs it as "trap where output expected" — same gap, two gate verdicts.

**Corpus cases added.** `spec/semantics/05-compound-types.sexp` §"a tuple payload returned through a
helper from a built-in Option must not trap" (`(tuple.0 (get (Some (tuple 7 8))))` → 7) and its
inline control §"a built-in Option tuple payload consumed INLINE in the Some arm projects" (→ 7,
PASSES). Native seed; the behavior gate catches the fn-return case (expected output 7, observed a
trap). Companion to the declared-sum case §"a tuple payload extracted through a helper return must
not be rejected as a type error" the HOL spike added.

**Narrowing (2026-07-08, later cycle) — the `let`-binding workaround pinpoints the missing shape-thread,
and it diverges built-in vs declared.** The shape IS recoverable; the gap is specifically projecting
DIRECTLY on a call-expression operand vs binding it first:
- Built-in `Some`: `(let ((t (get (Some (tuple 7 8))))) (tuple.0 t))` → **7** (works!), while `(tuple.0
  (get (Some (tuple 7 8))))` → traps. Same for a record payload: the `let`-bound form projects `.a` → 7,
  the direct form traps. So for a built-in `Some` payload the returned tuple's shape survives into a
  `let`-bound local — the direct-projection path just needs to do what the let-bound path already does
  (recover the operand's shape before lowering `tuple.N`/`.`).
- Declared-sum `Box`: even the `let`-bound form `(let ((t (unbox (Box.B (tuple 7 8))))) (tuple.0 t))`
  DECLINES "tuple access on a non-tuple" — the shape is not recovered through the `let` at all, a deeper
  gap than the built-in case (the declared-sum payload's shape is lost at the `match`-arm binder return,
  not just at the direct projection).
So the fix has two layers: (1) thread a call-expression's result shape into a directly-applied `tuple.N`/
`.` (built-in `Some` already has the shape in a `let`-bound local — reuse that recovery); (2) recover a
DECLARED-sum payload binder's shape through the `match`-arm return so `Box`-style payloads reach the
same state as built-in `Some`. The built-in `let`-bound success proves layer (1) is a plumbing gap, not
a missing capability.
