# Spec backlog — items the compiler-in-Cadenza spike surfaced, for operator review

**What this is.** A running, review-ready list of things the compiler spike found that need a
*specification* decision or edit — distinct from `SEED-GAPS-FOR-SELF-HOSTING.md` (which is *seed
implementation* work) and `RUNTIME-REQUESTS.md` (WIT/runtime work). Each item states the finding, why
it touches the spec, a proposed resolution, and its current status, so the operator can approve,
amend, or decline in one pass. Nothing here has been applied to normative spec text without an
explicit decision; where a decision was already taken it is marked DONE with the landing site.

Legend: 🔴 **open — needs operator decision** · 🟡 **proposed edit, awaiting approval** · 🟢 **DONE
(landed; recorded for history)** · ⚪ **deferred**.

---

## 1. 🟡 Pattern binders must compose (nest) — SEED BEHAVIOR NOW LANDED; only the spec MUST remains

**Finding.** `core-semantics.md` §Tuples requires `(tuple a b)` in pattern position binds the
elements, and §Sum Types requires a sum pattern has the form `(Ctor binder)`. Neither says a
*binder may itself be a compound pattern* — i.e. that patterns **nest**: `(Ctor (tuple a b))`,
`(tuple a (tuple b c))`, `(Ctor (tuple op (tuple a b)))`. Yet the corpus already leans on nesting
(`((T.Pair (tuple a b)) …)`), and the compiler's own `resolve`/`lower` front rung is written
`((Node.NPrim (tuple op (tuple a b))) …)` — a tuple nested inside a sum payload. The seed today
binds a *flat* payload tuple but declines a *nested* one (SEED-GAPS Tier 2b), and there is no
in-language workaround (a bare runtime-tuple match arm also declines).

**Why it touches the spec.** Pattern nesting is currently *implied* by the corpus but *not required*
by a MUST. A generation could bind flat patterns, pass every flat-pattern case, and still be unable
to compile the compiler — with no requirement it violated. The staple "a tagged node carrying a tuple
of sub-nodes, destructured in one arm" is the shape every tree-walking pass takes; it should be a
normative requirement, not folklore.

**Proposed resolution (🟡).** Add to `core-semantics.md` §Pattern Matching a requirement that
*patterns compose*: a constructor pattern's binder and a tuple pattern's element MAY themselves be
any pattern (a wildcard, a name, a tuple pattern, or a constructor pattern), matched recursively;
binding is the union of the sub-patterns' bindings (still linear per `CDZ0102`). This is a pure
clarification of intended behavior, not a new feature.

**Status.** Corpus case already added and pinned: *"a match arm binds a nested tuple inside a sum
payload"* in `05-compound-types.sexp` (→ 34, tagged `sum-type-declaration`; scores *todo* — the seed
declines cleanly today, turns green when `bind_sum_payload` recurses). The **spec MUST is the open
item.** Seed fix tracked as SEED-GAPS Tier 2b. Learning:
`spec/learnings/2026-07-06-the-front-rung-of-a-resolved-ir-compiler-needs-nested-payload-binders.md`.

**Update (2026-07-06, late):** this is now demonstrably the *only* thing between a working backend and
a compiler driven from a real surface tree. The spike proved the backend by ROUTING AROUND it — `main`
hand-builds the folded `Core`/`Func` list `resolve` would produce and drives the multi-function
assembler, emitting a valid 2-function component (`main = dbl(21)` → 42 via a real `call`). So the
downstream (assemble → frame, calls, params, runtime conditionals) is proven from the resolved IR
inward; only the surface→`Core` front rung stays stubbed behind this nested-binder gap. Fixing it (make
`bind_sum_payload` recurse into a nested `(tuple …)` binder) unblocks the front end end-to-end. Learning:
`spec/learnings/2026-07-06-the-compiler-emits-a-multi-function-module-with-a-real-call.md`.

**Update (2026-07-07) — SEED FIX LANDED; downgraded 🔴→🟡.** `bind_sum_payload` now recurses into a
nested `(tuple …)` binder (via `bind_tuple_elems` + a new `sum_payload_types` map carrying per-slot type
nodes), so a nested payload binds to any depth. The corpus case *"a match arm binds a nested tuple inside
a sum payload"* flipped **todo → PASS** with no edit to the oracle (reject-don't-miscompile working as
designed). The spike's front end is now **closed end-to-end**: a `Def`/`DList` multi-def surface +
`resolve-module` compiles `(module m (def (main) (+ 20 22)) (def (dbl x) (* x 2)))` to a valid 2-function
component. **What remains for the operator is only the spec MUST** — whether `core-semantics.md` §Pattern
Matching should carry an explicit requirement that patterns compose (a constructor/tuple binder may
itself be any pattern, matched recursively), now that the behavior exists and is gate-pinned. Purely a
"make the requirement explicit" call, no behavior change. Learnings:
`spec/learnings/2026-07-07-the-nested-payload-binder-fix-closes-the-front-end.md` (and the seed-side fix
in memory `nested-tuple-binder-in-sum-payload`).

---

## 2. 🔴 M-ordering tension: effects are the #1 self-host blocker but scheduled M6

**Finding.** The spike counted `DECLINE` markers across the flagship compiler: **effects = 10**
(ahead of numeric = 5, sum-decl = 3) — the compiler's own ambient state (`Fresh`, `Diag`, `Unify`)
is expressed as intra-program effects. The roadmap schedules effects at **M6**, after numeric (M4)
and traits (M5). So the single largest blocker to authoring the compiler in the intended style sits
two milestones out.

**Why it touches the spec/roadmap.** This is a sequencing decision only the operator can make. Two
coherent options, cost now visible:
- **(a) Pull effects earlier** (before/at M4) — unblocks authoring the compiler in the effectful
  style the spec's `compiler-pipeline.md` §"Phases Recover From Errors" already implies (record-and-
  continue is elegant *as* an effect).
- **(b) Keep the ladder** and author the compiler's state as **threaded immutable context** — the
  option already refined by [[dynamic-extent-is-an-effect-lexical-extent-is-a-parameter]]: lexical
  data threads as a parameter, only genuinely dynamic-extent state (diagnostics, fresh supply, unify
  store) needs effects. Under this refinement much of the "10" collapses to parameter-threading and
  effects at M6 may be fine.

**Status.** 🔴 **Operator call.** Note the spike has since *partially* de-risked this: Stages 0–3 of
effect lowering **landed in the seed** (tail-resumptive + state-threading + cross-fn inlining +
recursive-effectful monomorphization), so effects are further along than the roadmap's M6 implies.
The tension may already be softening in practice; the operator should decide whether to formally
re-order or let the flywheel resolve it. Recorded in
`spec/learnings/2026-07-05-authoring-the-compiler-in-cadenza-surfaces-the-language-gaps.md`.

---

## 3. 🟢 Typed instruction sum for the backend (not string-tagged quasiquote)

**Finding (spike FINDING #1).** The backend IR should be a **typed sum** (`Instr`/`Lir`) matched
exhaustively, not `(Ast.List (list (Ast.Name "i64-const") …))` — a string tag in a `Name` payload
forfeits exhaustiveness (extends "reject, don't miscompile" to the backend: a missing opcode arm is a
compile error). Quasiquote stays for the genuinely-`Ast`-valued frontend/macro layer.

**Status.** 🟢 **DONE.** Landed in `compiler-pipeline.md` §Representation ("MUST represent instructions
as values of a typed sum type… serialize… exhaustively over its variants… an instruction variant the
serializer does not handle is a compile-time error") and §"The Compiler Constructs AST Values Via
Quasiquote" (quasiquote reserved for AST-valued frontend/macro; instruction sum built by ordinary
constructors). Learning:
`spec/learnings/2026-07-05-the-internal-ir-is-a-typed-sum-the-public-ast-stays-homoiconic.md`.

---

## 4. 🟢 Boolean connectives (`and`/`or`/`not`) — the spec had none

**Finding.** A routine compiler predicate (the signed-LEB128 terminator) needs `(and …)`/`(or …)`;
they were absent from seed, corpus, AND spec.

**Status.** 🟢 **DONE end-to-end.** Requirement in `core-semantics.md` §"Boolean Connectives
Short-Circuit"; 6 corpus cases in `02-binding-and-control.sexp`; seed lowering landed (desugar to
short-circuit `if`). Own learning:
`spec/learnings/2026-07-06-a-language-with-conditionals-still-needs-boolean-connectives.md`.

---

## 5. 🟢 Effect-declaration surface + capability routing at the entrypoint

**Finding (spike FINDINGS #4/#2).** The corpus only ever *handled* ad-hoc ops; no way to *declare* an
effect. And the env/scope is a threaded map, not a State effect (dynamic-extent → effect;
lexical-extent → parameter).

**Status.** 🟢 **DONE.** Unified `(effect Name (op …))` declaration; routing-agnostic; discharged by a
lexical `(handle …)` or an entrypoint `(host (Eff…) body)` delegation; manifest computed as the union
of entrypoint delegations; `CDZ0401`(merged)/`CDZ0403`/`CDZ0404`. Landed across
`capabilities-and-effects.md`, `host-interface-binding.md`, `component-abi.md` (v4: entry = plain fn),
`14-effects-and-handlers.sexp`. Learnings:
`2026-07-05-effects-are-declared-with-one-surface-the-declaration-is-the-grant.md`,
`2026-07-05-dynamic-extent-is-an-effect-lexical-extent-is-a-parameter.md`, and memory
[[capabilities-routed-per-entrypoint-at-boundary]].

---

## 6. ⚪ Byte-identity target must account for optimization depth (cdz-rustc needs DCE)

**Finding.** The Cadenza compiler folds `(+ 20 22)` to `KConst 42` at the **Core layer before
emission**, so its component is 89 bytes with no dead code. cdz-rustc emits **128 bytes** for the same
program — it folds `run`'s body but leaves a **dead** overflow-check helper. The two agree on the
*result* and on `run`'s body, but not byte-for-byte. This is not a bug in either backend; it is the
two compilers folding at different depths.

**Why it touches the spec.** `self-hosting-and-bootstrap.md` §(the two compilers produce components
satisfying identical guarantees) and the component-check gate treat byte-identity as the convergence
signal. The finding reframes that target: **byte-identity is gated on both backends running the same
optimization depth** — specifically cdz-rustc gaining dead-code elimination (a Core→Core concern
separable from folding), or the Cadenza compiler matching cdz-rustc's shallower fold. Near-term
verification stays "validates + runs to the right answer"; byte-identity is a *named later milestone*,
not a silent expectation.

**Proposed resolution (⚪ deferred).** No spec edit needed now; recorded so the byte-identity gate is
read correctly. When DCE lands in cdz-rustc, revisit whether component-check can require byte-identity
on the folded path. Learning:
`spec/learnings/2026-07-06-the-front-rung-of-a-resolved-ir-compiler-needs-nested-payload-binders.md`.

---

## 7. ⚪ Runtime `String` is the keystone front-end blocker (spec is fine; seed + realized-set work)

**Finding.** Name dispatch (comparing a head against `"def"`, `"+"`, …) and the reader's symbol table
both need runtime `String`; all `String.*` is const-fold-only today. Not a spec *gap* (the string
capability is specified), but the operator should know it is the gate to a true `bytes → bytes` front
end, and that the built-in `Ast`/`quote` is a **dead end for self-hosting** (`quote` won't flow
through a function call; `Ast.*` ctors are unusable at runtime) — so the compiler decodes the CBOR
input into its **own user-declared `Node` sum**, which recurses through calls fine.

**Status.** 🟢 **DONE (2026-07-07) — the keystone landed.** Runtime `String` now works in the seed: all
four Tier-0 probes compile and run (string fn parameter → `String.byte-len` = 5; runtime `=` dispatch →
1; string returned across a call → `"hello"`; string sum-payload bound by a `match`). The compiler's
front rung now resolves a head by NAME (`head-prim` maps `"+"` → a `Prim` code; no string survives into
Core; unknown head → `PUnknown` → decline). Never a spec *gap* (the string capability was always
specified) — it was seed + realized-set work, now complete. Pinned by the runtime-string cases in
`13-strings.sexp` (all green) plus the new multi-way head-dispatch case. Learning:
`spec/learnings/2026-07-07-runtime-strings-unblock-the-name-based-front-rung.md`. **Remaining front-end
critical path:** decoding arbitrary-arity forms (needs the nested-payload binder — backlog #1) and the
CBOR reader / symbol table.

---

## 8. ⚪ No tail-call optimization / bounded wasm stack (self-host ceiling, not a blocker)

**Finding.** Non-tail recursion traps at the host wasm stack (~15–20k frames); no `loop`/tail-call
lowering. A tree-walk over a large source (the compiler compiling *itself*) will trap.

**Why it touches the spec.** `determinism-and-fuel.md` already says bounded execution is the host's
concern and a stack-limit trap is a defined halt — so this is **spec-consistent**, not a gap. The
open question is only whether to *require* `return_call` (Wasm 3.0 tail call) emission for
self-tail-recursive functions to raise the self-hosting ceiling. Not on the critical path.

**Status.** ⚪ Deferred; flag not block. Memory [[deep-recursion-traps-at-host-stack-limit]].

---

## 9. 🔴 Should a provable-certain trap be a compile-time rejection? (fold stays meaning-preserving either way)

**Finding.** The Cadenza compiler's `fold` pass guards division/modulo folding with `foldable-divisor`:
it collapses `(/ c d)` / `(% c d)` to a constant ONLY when the divisor is a non-zero, non-overflowing
constant; otherwise it leaves the primitive in place so the trap happens at run time as written. This
is the correct floor for the *fold* — an optimizer must preserve meaning, and `(/ 5 0)`'s recorded
meaning today is `(trap "division by zero")` (a trap is a legitimate terminal condition, constitution
§"exactly one terminal condition"), so folding it to a value would be a miscompile. Writing the guard
raised a policy question the operator flagged: **if we can PROVE at compile time that a program will
unconditionally trap, shouldn't we REJECT it at compile time rather than ship a program that traps?**

**Why it touches the spec.** This changes the *recorded outcome* of a whole class of cases — `(/ 5 0)`,
`(% 5 0)`, `(/ Int64.min -1)`, a constant OOB index, `expect` on a constant `None` — from `(trap …)`
to `(error CDZ…)`. It is a real semantics decision, not an implementation detail, and it interacts with
the corpus's existing recorded traps. The two questions are **separate** and must not be conflated:
1. *May the fold fold a trapping op into a value?* — **No, never** (settled; the guard is right, and it
   is independent of everything below).
2. *May/should a SEPARATE pass reject a program proven to trap?* — the open decision.

**Two forces push rejection OUT of the initial Core/fold pass** (the operator's "defer to a later pass"
instinct is right):
- **Reachability.** `(if false (/ 5 0) 42)` never traps; a bottom-up fold sees the subexpression but has
  no reachability analysis, so rejecting on sight would reject correct programs. Sound rejection needs
  "unconditionally reached AND always traps" — a dataflow pass, not a rewrite.
- **The ragged boundary.** If rejection fires "wherever the analysis happens to be strong enough," then
  `(/ 5 0)` is a compile error but `(/ 5 (id 0))` compiles and traps — same bug, opposite outcome,
  decided by how much constant propagation ran. That unpredictability is why most languages do NOT
  reject arbitrary provable traps.

**Prior art (crisp-boundary designs).** Rust and Zig reject provable traps only in contexts where
compile-time evaluation is ALREADY mandatory — a `const`, a type-level value, an array length — where
the value is *required* to exist, so a trap producing it is a genuine "no such value" error. In ordinary
runtime position it stays a predictable runtime trap. This keeps the boundary principled and small.

**Options for the operator:**
- **(a) Reject a certain-trap only in compile-time-mandatory-eval contexts** (crisp, principled, small
  blast radius; matches Rust/Zig). Ordinary runtime position keeps the trap.
- **(b) A dedicated "certain-trap" diagnostic pass with reachability, uniform over ALL trap kinds**
  (div-by-zero, overflow, OOB index, `expect` on `None`). To avoid the ragged boundary, make it a
  **warning** in runtime position (surface the bug without a coverage-dependent hard gate), escalating
  to a hard error only in mandatory-eval position. Larger, but catches the whole class.
- **(c) Status quo** — traps stay runtime traps; the fold guard is the only obligation. Simplest;
  ships programs that trap.

**Status.** 🔴 **Operator decision.** No corpus/spec change made pending the call. If (a) or (b): a new
diagnostic code + corpus cases flipping the affected constant-trap cases from `(trap …)` to `(error …)`
in the mandatory-eval (and/or warning) contexts, and a `compiler-pipeline.md` requirement that a
Core→Core rewrite is meaning-preserving (a runtime trap stays a runtime trap; a rewrite may not
manufacture NOR erase a trap) — which formalizes what the fold guard already does and is worth landing
under ANY option. Related: the seed's known const-fold over-eager trap on `(% Int64.min -1)` (corpus
`06-numeric-model.sexp` line ~500, gated so it does not FAIL) is the mirror bug — the fold trapping a
case that should NOT — and the same "rewrites preserve traps exactly" requirement governs both
directions. Learning:
`spec/learnings/2026-07-06-constant-folding-must-preserve-runtime-traps.md`.

**Update (2026-07-06):** the meaning-preservation requirement now has a THIRD witness beyond partial
arithmetic — CONTROL flow. Folding a constant-condition `(if c t f)` must drop the untaken branch so a
trap/effect in it does not occur (`(if (< 1 2) 7 (% 5 0)) → 7`), the dual of the erase/manufacture
arithmetic faces. This strengthens the case for landing the `compiler-pipeline.md` "a Core→Core rewrite
is meaning-preserving" requirement independent of the certain-trap-rejection decision (a/b/c above): the
requirement demonstrably governs division folding, modulo-overflow folding, AND conditional folding, and
the same short-circuit-shielding reasoning governs `and`/`or` desugaring. Pinned by
`02-binding-and-control.sexp` "a conditional whose condition folds to a constant still drops the untaken
trapping branch" (PASS). Learning:
`spec/learnings/2026-07-06-folding-a-constant-condition-preserves-short-circuit-shielding.md`.

---

## 10. 🟡 A spike's "verified byte-correct" claim must become a corpus case, not stay a probe

**Finding.** The spike's handoff docs repeatedly certify emit paths as "verified byte-correct" —
LEB128 (`uleb 624485 → E5 8E 26`), signed LEB128 boundaries, the core-module framing "byte-identical
to cdz-rustc for main=42", the component envelope. These were verified by ephemeral `emit` probes in
the gitignored `implementation/` tree, not by gate obligations. The corpus pinned each *primitive*
(`&`, `|`, `>>`, `Int.to-byte`, `Bytes.concat`) but not the *composition* — so the compiler's
byte-emitting spine was protected only by a scratch buffer that vanishes when the spike is cleaned.

**Why it touches the spec/process.** The two-compilers differential gate only protects what the
corpus pins; a hand-run probe is exactly the drifting parallel verification the
one-executable-semantics discipline exists to prevent. Verifying primitives separately does not verify
they compose to the right bytes — a single-primitive slip (wrong mask, dropped continuation bit) is
invisible per-primitive yet miscompiles the emitted component.

**Resolution (🟡 — partially applied; the rest is a standing rule).** Applied: two `10-bytes.sexp`
cases now pin the unsigned-LEB128 encoder to its known answer (`624485 → b"\xe5\x8e&"` + base-case
`100 → b"d"`, both PASS). Standing rule for the operator to bless as practice: **every "verified
byte-correct" claim in a spike handoff must be promoted to a known-answer corpus case before it counts
as durable.** The outstanding claims to promote as their paths stabilize: the signed-LEB128 encoder
(`-300 → D4 7D`, boundary values), the section/vector length-prefix framing, and the core-module /
component envelope byte shape (currently only exercised end-to-end via ignition, not as a
byte-asserting corpus case). Learning:
`spec/learnings/2026-07-06-the-compilers-byte-emitting-spine-needs-a-known-answer-corpus-case.md`.

---

## 11. 🟢 The front end's unknown-head path needs a real diagnostic, not a placeholder trap — RESOLVED (honest trap) 2026-07-07

**Finding.** With the front end now closed end-to-end (item 1), the compiler resolves a form's head from
its name string (`head-prim`). An **unrecognized head** resolves to `PUnknown` — a genuine front-end
error (the reader produced a form the compiler does not know) — but the spike currently "declines" it by
constructing an out-of-range `Bytes` value to force a **runtime trap** (`unknown-head-trap`), a
placeholder because the compiler-in-Cadenza has no diagnostics channel yet.

**Why it touches the spec.** This is a *compile-time rejection* masquerading as a *runtime trap*. An
unknown head is exactly the reader/front-end error class that should carry a `CDZ` diagnostic code and be
the program's recorded `(error CDZ…)` outcome — not a component that builds and then traps when run. It
also connects to the effects/diagnostics work: `compiler-pipeline.md` §"Phases Recover From Errors"
already envisions a diagnostics effect (record-and-continue), which is the natural home for this once the
compiler-in-Cadenza can perform effects. Interim behavior is honest (it halts rather than miscompiles),
but the end state is a front-end diagnostic.

**Status.** 🟢 **RESOLVED 2026-07-07 (honest trap) — and the placeholder was actively harmful.** The
`unknown-head-trap` (an out-of-range `Bytes.of (list 256)`) was replaced with a proper `Core.KError`
variant that lowers to `unreachable` — a defined trap, no Bytes hack. This was not just cleanup: the
out-of-range-Bytes placeholder was a `Never`-typed value that, on the runtime-heap path, made the whole
runtime-called `resolve` emit an invalid component — it was the true cause of the "cannot box" decline I
mis-diagnosed as a seed scale limit (see item 16, withdrawn). So the honest form both removed a real bug
and is the correct design. **Remaining (deferred, not blocking):** a proper `CDZ` diagnostic code (rather
than a bare `unreachable` trap) once the compiler-in-Cadenza grows a diagnostics channel (the `Diag`
effect) — but the miscompiling placeholder is gone and the interim behavior is now an honest defined
trap. Learnings:
`spec/learnings/2026-07-07-the-workaround-was-the-bug-correcting-the-scale-limit-diagnosis.md`,
`spec/learnings/2026-07-07-the-nested-payload-binder-fix-closes-the-front-end.md`.

---

## 12. 🟢 The built-in Option/Result loses its payload kind across a function boundary (the reader gate) — RESOLVED 2026-07-07

**Update (2026-07-07) — 🟢 RESOLVED, all facets.** The seed rebuilt and closed every facet at once: the
bare `(Some 42)` through a helper (the general payload-kind-recovery facet, the deepest, untouched by the
earlier per-accessor fixes) → 42, and `String.from-bytes` through a helper (the reader's symbol-table
decode; real `gen_runtime_string_from_bytes`, a total fallible UTF-8 decode that validates with the
existing runtime — the in-flight `bytes-is-utf8` op was not needed on this path) → 2 (ill-formed → None
arm → -1). Both corpus cases withheld/todo in earlier cycles flipped **todo → PASS**: `05` *"a built-in
Option is unwrapped by a helper that binds its payload"* and `13-strings` *"a helper decodes bytes to a
string and consumes the fallible result"*. Confirms this item's thesis: per-accessor patching closed
symptoms; the general kind-recovery fix closed the class. Learning:
`spec/learnings/2026-07-07-the-reader-gate-closed-and-list-at-on-a-payload-list-is-the-next.md`.

**Finding.** A built-in `Option` payload-binding `match` declines "runtime sum match arms differ in
kind" once the `Option` crosses a function boundary. Sharp boundary: `(match (Some 42) ((Some x) x)
((None _) 0))` at the entrypoint compiles, but the same match in a helper (`(unwrap (Some 42) 99)`)
declines — as does the reader's idiom `(match (Bytes.at b i) ((Some x) x) (None …))` on a runtime
`Bytes`. Yet `List.at`'s `Option` and every **user-declared** sum compile in the identical shape (the
Tier-2b fix), and `Option.expect` works. So the gap is the **built-in `Option`/`Result` constructors
carrying no per-slot payload type** (the `sum_payload_types` a user `type` populates); their payload
kind is recoverable only where local type context supplies it, and is lost across a boundary.

**Why it touches the spec/seed.** This is the **current gate on the reader**, hence on true
`bytes → bytes` self-hosting — the reader passes `Option`s between helpers on every byte. The spike's
SEED-GAPS Tier 2c framed it as `Bytes.at`-specific; the probe set shows it is broader (a literal
`(Some 42)` through a helper also declines), so the fix must **register the built-in polymorphic sums'
payload types the way a user sum's are**, not patch `Bytes.at`. Not a spec *gap* (the behavior is
already what the corpus records); it is seed inference work. Recorded here so the operator sees it as
the reader gate.

**Status.** 🟢 **DONE (2026-07-07, seed side).** Both corpus cases now PASS: *"a built-in Option is
unwrapped by a helper that binds its payload"* (→ 42) and the new *"a generic unwrap helper consumes a
fallible Bytes.at result"* (→ 20), both `05-compound-types.sexp`. The fix was NOT registering the
built-in sums' payload types (they are genuinely polymorphic — `Some a`'s payload has no fixed kind);
it was RECOVERING the concrete kind at the match site by **unifying the arm result kinds**. A new
fallback `infer_sum_payload_override` (in `gen_match_runtime_sum`) — used when the scrutinee's static
shape can't pin the payload (an opaque `Heap` param) — seeds a shared `InferCtx` with the arm binders +
enclosing locals, infers/unifies/back-propagates the arm results, and reads back each binder's solved
concrete scalar kind, so `bind_sum_payload` unboxes it. This is the parameter-boundary twin of Tier 2c's
scrutinee-shape override; together the built-in `Option`'s payload survives a match anywhere a user
sum's does. Gate 521/0, ignition byte-identical, component-check 527/0.
See [[sum-match-payload-kind-recovered-by-arm-unification]].
Learning: `spec/learnings/2026-07-07-the-built-in-option-loses-its-payload-kind-across-a-boundary.md`.

**Update (2026-07-07, later) — being closed ACCESSOR-BY-ACCESSOR; the class is still open.** The seed
fixed the `Bytes.at` facet: `(match (Bytes.at b i) ((Some x) x) (None …))` through a helper now compiles
(the reader's per-byte idiom; sibling pinned the passing cases in `10-bytes.sexp`). But the fix is
accessor-specific, confirming this item's thesis. Current map of the gate:
- `List.at` through a helper → ✅ works; `Bytes.at` through a helper → ✅ works (fixed this cycle).
- `String.from-bytes` through a helper → ❌ declines *"unsupported dotted-application"* (a DIFFERENT
  message — it needs its own runtime lowering, not just payload-kind unify). The reader's **symbol-table
  decode** idiom. Now pinned: `13-strings.sexp` *"a helper decodes bytes to a string and consumes the
  fallible result"* (→ 2, **todo**).
- a bare literal `(Some 42)` through a helper → ❌ still declines *"arms differ in kind"* (the general
  built-in-`Option` facet, untouched).

The reader uses all of these at once, so it compiles only when the LAST accessor lands. The general fix
(payload-type registration for the built-in sums) closes all facets uniformly; accessor-by-accessor is a
sequence of symptom fixes to the same end. Learning:
`spec/learnings/2026-07-07-the-reader-gate-is-being-closed-accessor-by-accessor.md`.

**Update (2026-07-07, later) — the `String.from-bytes` facet's runtime support is IN-FLIGHT.** The
`from-bytes`-through-a-helper facet needs a runtime `String.from-bytes` (its own lowering, not just the
payload-kind unify the other accessors needed). The spike's in-flight fix (WIT append + codegen,
mid-landing — binary not yet rebuilt at this snapshot): a runtime `String` IS the same Bytes-backed UTF-8
leaf, so `from-bytes` is a validity CHECK (new runtime op `bytes-is-utf8`, WIT idx 54, the Unicode
validator — rejects overlong/surrogate/>U+10FFFF) plus a zero-cost retag, not a decode/copy. Design +
correctness requirements recorded in
`spec/learnings/2026-07-07-string-from-bytes-validates-in-the-runtime-a-string-is-a-utf8-bytes-leaf.md`;
strict-UTF-8 requirement pinned by two new `13-strings.sexp` cases (overlong `C0 80` + surrogate `ED A0
80` → None). Not yet confirmed by probe (binary not rebuilt); verify when built. The bare-`(Some 42)`
facet (the general payload-kind class) remains the deepest fix.

---

## 13. 🔴 The built-in `list` has no pattern-matching surface — spec addition needed

**Finding.** The built-in `list` cannot be pattern-matched at all: `(cons h t)`/`nil`, positional
`(list a b c)`, and empty `(list)` all decline "unsupported list pattern"; `(List.Cons …)` gives
"runtime sum match on an undeclared variant". `core-semantics.md` §Pattern Matching specifies tuple
and sum-constructor patterns but says **nothing about lists** — so this is unspecified, not just
unimplemented. A `list` is consume-only via `List.at`/`List.len` + index recursion.

**Why it touches the spec.** Pattern matching is a core-semantics surface, and "how is a list
deconstructed" is a hole in it. The gap shapes *every* list-consuming pass a compiler writes (module
def list, code stream, CBOR array children), forcing each to hand-roll a custom cons-sum (`(type FList
(FNil | FCons …))`) that duplicates the persistent sequence the language already has — the single
biggest ergonomic gap for authoring the compiler idiomatically. It is a language-surface decision (a
MUST about how `match` sees a list), the operator's to make, not seed-only work.

**Proposed design (element patterns with a rest binder — keeps representation opaque).** NOT Lisp
`Cons`/`Nil` (a `list` is a persistent tree, not cons cells; exposing cells leaks a hidden
representation). ML/Rust-style instead:
```
(match xs
  ((list)           empty)               ; exactly zero elements
  ((list x)         one x)               ; exactly one (sugar for a length check)
  ((list x .. rest) first x, tail rest)) ; first element + the rest AS A LIST
```
An exhaustive fold needs the empty case and a rest-pattern case; fixed-arity cases are length-check
sugar. The matcher asks `len`/`first`/`rest` (expressible over existing `List.at`/`List.len`/`List.slice`),
so the representation stays opaque. Spec: a new `core-semantics.md` §Pattern Matching clause *"A List
Is Deconstructed By Element Patterns With An Optional Rest"*; plus corpus cases + seed lowering.

**Status.** 🔴 **Operator decision (spec addition).** Pinned by `05-compound-types.sexp` *"the built-in
list is folded by an element-with-rest pattern"* (`(match xs ((list) 0) ((list x .. rest) (+ x (sum
rest))))` → 60), tagged `(needs list-patterns)` so it **skips** until specified+realized. Once landed,
the compiler's `Code`/`FList`/`DList` cons-sums collapse to the built-in `list` — a real simplification.
Learning: `spec/learnings/2026-07-07-the-built-in-list-cannot-be-pattern-matched.md`.

---

## 14. 🟢 Kind inference is branch-order-dependent for a recursive Bool return — FIXED 2026-07-07

**Finding.** A self-recursive `Bool`-returning function declines when its `if` body has the self-call
in the `then` branch and a `Bool` literal in the `else` (`(if (< i n) (go (+ i 1) n) true)` → "if
condition is not Bool" as a cond, "branches differ in kind" when returned). The mirror (self-call in
`else`) compiles, and an `Int`-returning version compiles — so it is `Bool`-specific and
branch-order-specific: the self-call's placeholder kind and the `Bool`-literal sibling unify in an
order that locks the return kind non-Bool.

**Why it touches the seed (not a spec gap).** This is the **same order-dependent kind race as Tier 00**
(which was the `Heap` instance — a threaded compound accumulator inferred scalar), now on `Bool`. It is
a seed inference bug, not a language-surface question — the corpus already records the correct behavior.
The fix is the proven Tier-00 one, generalized: a concrete-kind branch (a `Bool` literal, or any
concrete sibling) must **pin the `if`/`match` result kind regardless of branch order**, and a self-call
placeholder yields to a concrete sibling. The lesson for the operator: kind-inference order-independence
is a property *every* result kind needs, so the fix belongs at the general result-unification, not as a
per-kind patch — worth stating once in the seed's inference rather than re-patched per kind.

**Why it matters.** The reader's head resolver is a recursive `Bool` `name-eq` ("all bytes equal so
far, else false") in exactly the failing shape, so this is a current gate on the reader → self-hosting.

**Status.** ⚪ Seed work (SEED-GAPS "Tier 2d" recursive-Bool note; mislabeled — a distinct item). Pinned
by `09-functions.sexp` *"a self-recursive Bool-returning function whose recursive call is the
then-branch"* (`(go 0 3) → true`). Learning:
`spec/learnings/2026-07-07-recursive-bool-return-kind-inference-is-branch-order-dependent.md`.

**Update (2026-07-07) — 🟢 FIXED.** The seed made `if`/recursive-return kind inference order-independent
for Bool (a concrete-kind branch pins the result kind regardless of order), the generalization this item
called for. The corpus case flipped **todo → PASS** with no oracle change. This unblocked the reader's
`name-eq` byte-comparator (was dead code in exactly this shape) → the reader's name matcher is now live.
Third confirmation the order-independence rule is one property, not per-kind (Heap=Tier 00, Bool=this).
Consolidated in `spec/learnings/2026-07-07-the-name-matcher-unblocks-and-the-surface-language-composes.md`.

---

## 15. 🟢 `tuple.N` on a named-def's runtime-tuple result (no `let`) emitted an INVALID component — FIXED 2026-07-07

**Finding.** `tuple.N` applied directly to a **named-def** function's runtime-tuple result, with no
intervening `let`, emits an **invalid component** (`component failed validation`), not a clean decline
or a defined trap. Sharp boundary:
- `(let ((r (dec 4))) (tuple.0 r))` → ✅ works (the Tier-2e fix: the `let`-bound `Local` carries the
  tuple's `Shape`).
- `(tuple.0 ((fn (x) (tuple x 9)) 7))` → ✅ works (a **lambda** result is compile-time-reduced, shape
  statically resolvable; a corpus case already pins this).
- `(tuple.0 (dec 4))` where `dec` is a **named def** → ❌ **INVALID component**.

**Why it matters.** An invalid component is the category the whole two-compilers gate exists to forbid
— strictly worse than a clean decline or a defined trap, and the gate scores it as a FAIL
(disagreement), not a todo. Note the spike's SEED-GAPS Tier 2e records this as "a VALID component that
TRAPS at the renderer" — direct measurement shows **INVALID** (fails wasm validation), so the handoff
under-states the severity; a decline-don't-miscompile violation was recorded as the milder valid-but-traps
state. It is the same lambda-vs-named-def asymmetry that governs HOF inlining (a lambda inlines, a
named-def HOF declines): where compile-time reduction does not reach, the emitter must **decline**, not
emit invalid code.

**Status.** 🟢 **DONE (2026-07-07, seed side) — and it COMPILES, not just declines.**
`(def (main) (tuple.0 (dec 4)))` now runs → 40 (corpus case *"a scalar element is projected DIRECTLY
from a named function's runtime tuple result"* in `05-compound-types.sexp`; gate 514/0, component-check
521/0, ignition byte-identical). Root cause was NOT in `tuple.N` — it was `gen_runtime_ctor`: its
scalar-path decline (`call_base == 0`) was gated on **all elements being const**, so a tuple with a
runtime element (`(tuple (* n 10) 9)`, `n` a param) emitted `arr-alloc`/`box-int` into an import-free
scalar module → INVALID. Fix: `gen_runtime_ctor` now declines UNCONDITIONALLY on the scalar path (a
runtime tuple/record cannot build without the value-heap imports), so `compile_module` either
dead-stubs the function (when `main` structurally projected a scalar out of it and never calls it at
runtime) or RETRIES in runtime mode where the imports exist. The `tuple.N` projection then recovers the
scalar at the projection site via the operand's structural shape. Same decline-don't-miscompile gate
the sum constructor already had; the ctor's was just too narrow.
See [[runtime-compound-ctor-declines-unconditionally-on-scalar-path]].
Learning: `spec/learnings/2026-07-07-runtime-tuple-projection-needs-a-let-and-the-direct-path-miscompiles.md`.

**Update (2026-07-07) — 🟢 FIXED.** `tuple.N` now recovers the operand's shape at the projection site
regardless of binding form, so the `let`-free named-def case compiles: `(tuple.0 (dec 4)) → 40`, and
thoroughly (tuple.1 → 5, consumed → 140, compound element matched → 7). The corpus case withheld last
cycle (it FAILed the gate as an invalid component) now lands **green**: `05-compound-types.sexp` *"a
scalar element is projected directly from a function's runtime tuple result"*. Clean lifecycle for a
decline-don't-miscompile violation: invalid → withheld → fixed → pinned green. ⚠ SEED-GAPS Tier 2e still
carries a stale "still produces a VALID component that TRAPS" note — the seed runs it correctly now; the
handoff lags. Consolidated in
`spec/learnings/2026-07-07-the-invalid-component-violation-fixed-and-the-handoff-lags-the-seed.md`.

---

## 16. 🟢 The real `resolve` on a runtime-built `Node` declined "cannot box" — RESOLVED (was MIS-FRAMED as a seed scale limit) 2026-07-07

**⚠️ Correction (2026-07-07):** this item was **mis-framed**. It was NOT a seed scale limit in the
runtime heap-boxer. The "cannot box" decline was **self-inflicted**: `resolve`'s `PUnknown` arm used an
out-of-range `Bytes.of (list 256)` as a placeholder trap (item 11's stub), a `Never` value that poisoned
the whole runtime `resolve`. Replacing it with an honest `Core.KError → unreachable` fixed it — see item
11. My "scale limit, no minimal witness" bisection was wrong because it rebuilt a clean structural
analogue and dropped the culprit (the Bytes hack); the correction and its meta-lesson (reduce the failing
program by deleting its arms, not by rebuilding a clean one) are in
`spec/learnings/2026-07-07-the-workaround-was-the-bug-correcting-the-scale-limit-diagnosis.md`. A real but
differently-shaped seed invariant DID exist underneath (a `Never` value on the runtime-heap path emitted
invalid code), now hardened and pinned ([[never-typed-value-on-the-runtime-heap-path]]). Net: `resolve` on
a runtime `Node` compiles, the reader→pipeline connects, `bytes → component` works end-to-end. The
original (mis-framed) analysis is kept below as the historical record.

---


**Finding.** `compiler.cdz`'s real `resolve : Node → Core` declines **"runtime compound element of a
kind the runtime cannot box yet"** when applied to a `Node` built at runtime (what the reader produces)
and forced to a runtime value. This is the last link: `read-node : Bytes → Node` is built and verified
(`read (quote (+ 1 2))` builds the right Node), but `read → resolve → fold → lower → serialize → frame`
cannot connect because `resolve` on a runtime `Node` declines.

**It is a SCALE limit, not a shape gap.** It does not reduce to a minimal case — every structural
feature works at runtime in isolation: a 3-variant resolver runs, a 6-variant heterogeneous
`KConst`/`KBoolC`/`KAdd`/`KLt`/`KNot`/`KIf` resolver runs (verified → 4), runtime `(Tuple String Node
Node)` build+match works, `head-prim` on a runtime String works. Only the **full 18-variant `Core`
returned by the full `resolve`** declines, and even `resolve` on a runtime `(NInt 42)` (a scalar arm)
declines — so it is a full-FUNCTION property (some arm's Core construction poisons every call), a
specific element-kind combination in the 18-variant union the runtime heap-boxer rejects on this path.

**Why it touches the seed (not the spec).** The language clearly permits a recursive `Node → Core`
resolver over runtime input — every sub-shape compiles. It is a runtime heap-boxer limitation at the
union/scale of the full variant set. Seed fix: trace which `gen_runtime_*` / heap-box path
`resolve`-of-a-runtime-`NPrim` hits and reports "cannot box", and admit that element-kind combination.

**Status.** ⚪ Seed work (SEED-GAPS Tier 2f). **No corpus case** — deliberately: a scale limit has no
minimal witness (every tractable resolver passes; the failing one is the full 18-variant `resolve`, too
large and threshold-specific to pin durably). Its regression guard, when fixed, is the whole
`compiler.cdz` connecting `read → resolve → … → frame` and compiling — the two-compilers gate on the
whole compiler. **This is the single remaining hard blocker on `bytes → bytes` self-hosting** (items 12
and 13 remain, but the reader routes around 12 for structure and 13 is ergonomic). Learning:
`spec/learnings/2026-07-07-the-final-self-host-blocker-is-a-scale-limit-not-a-shape-gap.md`.

**Update (2026-07-07) — 🟢 FIXED.** The runtime heap-boxer now admits the full 18-variant `Core` union
on the `resolve` path: the full-shape `resolve` on a runtime-built `Node`, scalar-consumed, runs (→ 1).
Now that it is fixed, a **representative** corpus case IS pinnable (the scale-limit rule flips: no minimal
witness while broken, but a natural-size representative guards it once fixed): `05-compound-types.sexp`
*"a recursive resolver transforms one runtime sum tree into another, then consumes it"* (`resolve : Node →
Core` then `eval : Core → Int64`, → 42, **PASS**). With this, **every self-host seed blocker
(Tier 00/0/2b/2c/2d/2e/3a/2f) is cleared** — the remaining work is WIRING the `read-node → resolve` join
in `compiler.cdz` (kept uncommitted until 2f landed) plus non-blocking items 12/13. Learning:
`spec/learnings/2026-07-07-the-final-self-host-blocker-is-fixed-the-reader-can-join-the-pipeline.md`.

---

## 17. 🟢 `List.at` on a list bound from a sum payload declines (blocks the natural multi-arg-call rep) — FIXED 2026-07-07

**Resolution (2026-07-07, seed side).** Root cause was NOT payload-specific: there was simply NO runtime
`List.at` emitter at all — `gen_dotted_apply` had runtime `List.push`/`update`/`len` but not `at`, so
any non-const-folding `List.at` fell to "unsupported dotted-application". (A top-level `List.at (list …)
i` "worked" only by const-folding the literal list; a payload-bound list is a genuine Heap handle that
can't fold.) Added `gen_runtime_list_at` (mirrors `gen_runtime_bytes_at` over `vec-len`/`vec-get`): a
FALLIBLE index → `(Some elem)` / `(None unit)`. A list element is ALREADY a boxed handle (stored via
`box_scalar` at construction), so `vec-get` returns it directly as the `Some` payload — the caller's
match unboxes it via the payload-kind override. Also wired `List.at` in `infer_list` (list→Heap,
index→Int64, result→Heap Option) and `shape_of_list` (→ `Option<element-shape>` for rendering). The
multi-arg-call idiom `KCall (Tuple Int64 (List Core))` lowered by `List.at args i` now works — verified
by an `ev` that recurses over a payload arg list summing `KConst`s → 42. Pinned by corpus *"indexing a
list bound from a sum payload yields the element"* (→ 10) and *"a multi-argument call node is evaluated
by iterating its payload arg list"* (→ 42). Gate 527/0, component-check 532/0, ignition byte-identical.
See [[runtime-list-at-fallible-index]].

**Original finding (for history).**

**Finding.** `List.at` on a `list` **bound out of a sum-type payload** by a `match` arm declines
"unsupported dotted-application", for any element type. `List.len` on the same payload-bound list
**works**, and `List.at` on a **top-level** list parameter **works** — so the gap is specifically
element-access on a payload-bound list. Same shape as the earlier payload-kind gaps (Option payload,
runtime `tuple.N`): a value bound from a sum payload is an opaque `Heap` handle whose *element-access*
lowering isn't wired for the payload-bound case though its *length* is.

**Why it touches the seed.** The natural representation of a multi-argument call is `KCall (Tuple Int64
(list Core))` — a fn index plus an argument *list* — and lowering iterates that list with `List.at args
i` per argument. But the arg list is a sum-payload field, so `List.at` on it declining means multi-arg
calls can't be lowered by iterating a payload-stored list (unary calls, `Tuple Int64 Core`, work).
Not a spec gap — the language plainly permits it (top-level `List.at` works); it is seed lowering.

**Status.** ⚪ Seed work (SEED-GAPS Tier 3h). Pinned by `05-compound-types.sexp` *"indexing a list bound
from a sum payload yields the element"* (`(K.KK (tuple 7 (list 10 20 30)))` matched, `List.at xs 0` →
10), scores **todo** (declines cleanly today). Fix: make `List.at` on a payload-bound list lower like a
top-level list (both are the same runtime array handle). Note the overlap with item 13 (list patterns):
if list patterns land, a compiler destructures an arg list by pattern rather than `List.at`-iterating
it, so 17 and 13 are two routes to the same multi-arg-call capability. Learning:
`spec/learnings/2026-07-07-the-reader-gate-closed-and-list-at-on-a-payload-list-is-the-next.md`.

---

## 18. 🟢 A recursive `List.push`-accumulator loses its list return kind — FIXED 2026-07-07

**Finding.** A function that is (a) recursive, (b) threads a `list` accumulator parameter, and (c) grows
it with `List.push` in the recursive call has its RESULT kind inferred as non-list, so `List.len` /
`List.at` on the returned value declines "…of a non-list value". Boundary is exactly the conjunction of
the three — drop any one and it works (non-recursive push, an int accumulator, or a no-push identity
thread all compile). Verified: `(def (build n acc) (if (< n 1) acc (build (- n 1) (List.push acc n))))`
then `List.len` declines.

**Why it touches the seed.** It is now THE blocker for multi-argument user-function calls: the reader
accumulates a call's operands into a `(list Node)` with exactly this push-loop shape (`(read-args … i
out) = (read-args … (+ i 1) (List.push out (read-node …)))`), so the arg list can't be built. Same
inference family as Tier 00 (a base-arm-returned accumulator seeds scalar; `List.push`'s list result
must UPGRADE it, not be collapsed) — a `list`-return instance of the order/position-independent
recursive-result inference the arc keeps hitting. Not a spec gap; seed inference.

**Status.** ⚪ Seed work (SEED-GAPS Tier 3i). Pinned by `05-compound-types.sexp` *"a recursive list
accumulator grown by push and returned in the base arm stays a list"* (`build 3 (list)` → `List.len` =
3), scores **todo** (declines cleanly today). Fix: infer the list/heap return kind for a recursive
function whose accumulator is grown by `List.push`, aligning with the non-recursive `List.push` case and
the push-as-first-argument recursive builder (both already infer list). **Unblocks multi-arg calls.**
Learning: `spec/learnings/2026-07-07-a-recursive-push-accumulator-loses-its-list-return-kind.md`.

**Update (2026-07-07) — 🟢 FIXED.** A recursive push-accumulator now infers a list return (`build 3
(list)` → `List.len` = 3); the todo case flipped **todo → PASS**. Both halves of arg-list handling now
work: build (this, #18) + read (#17). Round-trip verified + pinned: `05-compound-types.sexp` *"a list
built by a recursive push-loop is then iterated by index"* (build `[0 1 2]`, sum by `List.at` → 3).
**Remaining to multi-arg calls is pure WIRING** — `compiler.cdz`'s `read-call` still handles only unary
calls (with a now-stale "blocked" comment); updating it to build the arg list with the push-loop and emit
an N-ary call is not a seed gap. Learning:
`spec/learnings/2026-07-07-the-arg-list-round-trip-works-build-by-push-read-by-index.md`.

## 19. ⚪ A nested constructor pattern under `Some` declines when the matched list is a parameter

**Finding.** `(match (List.at xs i) ((Some (E.Lit n)) …) (None …))` — a nested constructor pattern
inside the `Some` arm — declines "runtime sum match: unsupported payload binder" when `xs` is a
function parameter. It works when `xs` is an in-place literal list, and works with a two-step bind
(`(Some e)` then inner `match e`). So the boundary is: nested ctor under `Some` + payload element kind
arriving through a parameter (erased to opaque `Heap`).

**Why it touches the seed.** Destructuring a heterogeneously-typed list element in one pattern (matching
a `Node`/`Core` element with its constructor directly) is the ergonomic way to write the reader's/
lowering's list walks. Same family as the sum-match payload-kind-override fixes already landed, extended
one level deeper (through `Option`-of-a-parameter-list-element). Lower priority — the two-step
bind-then-match workaround is clean.

**Status.** ⚪ Seed work (SEED-GAPS Tier 3j), lower priority (has a workaround). Not pinned as a corpus
case yet (the two-step form works, so the idiom is expressible; pin the one-step form when the nested
payload-kind recovery lands). Fix: extend the payload-kind override to a nested constructor pattern
under a sum arm when the list is a parameter. Learning:
`spec/learnings/2026-07-07-a-recursive-push-accumulator-loses-its-list-return-kind.md` (noted alongside #18).

## 20. ⚪ The self-inclusion frontier: what the compiler's emit path must grow to compile its own source

**Finding.** Self-hosting is no longer seed-blocked — every seed gap the spike surfaced is fixed, and
the compiler compiles `module bytes → component` for the multi-def / params / `let` / N-ary-call /
full-operator subset over Int64/Bool. What remains is **subset growth**: the compiler's *own source*
(`compiler.cdz`) uses constructs its *emit path* cannot yet produce. This is a **coverage inventory**,
not a defect — each item is "the compiler doesn't yet emit code for a program that uses X," where the
*seed* compiles X fine (verified: the seed compiles a user-sum `match` → 31; `compiler.cdz`'s emit-side
`Core` has no `KMatch`). Measured against the current source:
- **`match` on user sums** — THE big one: `compiler.cdz` has ~41 `match` expressions over 11 user sum
  types (`Node`, `Core`, `Instr`, `Prim`, `IList`, `Def`, `FList`, …), the spine of every pass. The
  emit-side `Core` has `KConst`/`KAdd`/…/`KCall`/`KIf`/`KLet` but no user-sum-type declaration,
  construction, or `KMatch`. This is the last major emit construct.
- **String / Bytes ops in emitted output** — ~19 `String.*`/`b"…"` uses; the emit path handles Bytes
  building (the output) but the compiler's source also *compares* and *slices* strings/bytes.
- **Deep recursion / scale** — the source is pervasively recursive; compiling it walks deep, where the
  bounded wasm stack bites (item 8 / TCO).

**Why it touches the roadmap (not the spec).** This is the concrete distance to *compiler-compiles-
compiler*, framed as what the emit path must cover, not a language question. It is the
already-recorded reframing ([[self-hosting-gate-shifts-from-seed-capability-to-bootstrapping-subset]])
made into a checklist. Not a spec decision; a scope/sequencing view for the operator.

**Status.** ⚪ Roadmap inventory, not a single fix. Priority order for reaching self-inclusion:
(1) emit `match` on user sums + user-sum construction (the bulk of the source); (2) String/Bytes
comparison ops on the emit path; (3) TCO for deep tree-walks (item 8); (4) a **local-allocating lower
pass** — `compiler.cdz`'s Lir is a pure Core→Code fold with no scratch-local allocation, so any operator
whose faithful wasm lowering needs a **guard** (a runtime check via scratch locals) cannot be emitted:
shifts `<< >>` (count-range trap + left-shift overflow guard; an unguarded `i64.shl` miscompiles because
wasm masks the count mod 64) are the first, and are correctly DECLINED (`KError → unreachable`) until this
pass lands — the honest choice, since a bare emit would miscompile. Each is subset growth the loop will
pin as it lands. Learnings:
`spec/learnings/2026-07-07-match-on-user-sums-is-the-last-major-emit-frontier.md`,
`spec/learnings/2026-07-07-a-no-scratch-local-lir-must-decline-ops-that-need-guard-locals.md`.

## 21. ⚪ Over-applying a user function declines as "needs closures", not the CDZ0201 the corpus says it mirrors — and head-position name classification is fragile

**Finding.** `(f 5 9)` on a unary `f` declines *"call to f with 2 args, expected 1 (partial application
needs closures)"*. But the corpus records the parallel constructor over-application `(Some 1 2)` as
`(error CDZ0201)` (apply-a-non-function), and `09-functions.sexp`'s prose says a user-function
over-application is "arity-checked the same way" — yet only the constructor case is pinned, and the
seed treats the user-function case as a closure-feature gap rather than the type error it is (`(f 5 9)`
= `((f 5) 9)`, applying the Int64 `6` to `9`).

**Why it touches the seed (not the spec).** The recorded semantics already imply CDZ0201 (the
single-arity desugaring is the same as the constructor case); the seed's divergent "needs closures"
decline is the gap. **A second, deeper signal:** pinning this as a corpus case FAILed the gate via a
cross-case interaction — adding `(f 5 9)` flipped an unrelated passing case (`(let ((ctor None)) (ctor
unit))`, binding+applying the prelude constructor) to a wrong *"CDZ0401: undeclared capability: ctor"*.
So head-position name classification (is a head a bound value / a constructor / a capability / an
over-applied function?) is order-sensitive and destabilizes when a new over-application case is added.

**Status.** ⚪ Seed work. **No corpus case** — pinning `(f 5 9) → CDZ0201` broke the gate (the cross-case
`ctor`-misread-as-capability regression), which the corpus discipline forbids; pin it once the seed
classifies user-function over-application as `CDZ0201` and head-position classification is total and
order-independent. Scope: not just "emit CDZ0201 for over-application" but "make head-position name
classification total across value / constructor / capability / over-applied-function." The trigger was
a transient mid-refactor `compiler.cdz` (a `kind-of` call/def arity mismatch), not a compiler
regression. Learning:
`spec/learnings/2026-07-07-over-applying-a-user-function-declines-as-closures-not-as-an-arity-error.md`.

## 22. 🟢 Seed gap 3l: emit a `compile : list<u8> → list<u8>` component, not only nullary `run` — RESOLVED (seed side) 2026-07-07

**🟢 RESOLVED 2026-07-07 (seed side) — verified by direct probe.** The seed now lifts a def named `compile`
with one `Bytes`/`list<u8>` param → `Bytes` as `cadenza:compiler/compile : func(list<u8>) -> list<u8>`
(codegen selects it over `run` by shape — codegen.rs:1039, 8749). A new dev subcommand
`cadenza-seed compile-run <compiler.cdz> <input.cdz>` builds the compiler as a compile component and drives it
over the input's canonical AST bytes. **Probed end-to-end:** an identity `(def (compile b) b)` builds a VALID
3,059-byte component and returns the input's 32 canonical AST bytes unchanged (the list ABI round-trips through
linear memory: input → runtime `Bytes` handle → user `compile` → result bytes → retptr). The retarea must be
4-aligned; SINGLE export only (the compiler world has one export; a general `(export …)` surface is deferred).

**Two mechanical steps remain (neither a language/correctness gap):**
1. **Rewire `compiler.cdz`'s entry** from the current nullary `(def (main) …hardcoded target bytes…)` to
   `(def (compile b) (compile-bytes b))`. `compile-bytes` (the whole read→resolve→fold→lower→serialize→frame
   pipeline) already exists and takes a `Bytes`. Until this happens, `compile-run` on the real `compiler.cdz`
   fails `expected 0 argument(s), got 1` — the nullary `main` is lifted as `run`, and the host drives it with
   the 1-arg input a `compile` entry expects. (No forcing function yet: the interim harness still works on the
   nullary form.)
2. **Get the value-heap runtime component building again** (currently broken — CHAMP set ops mid-implementation)
   so `cadenza-seed component-check <compiler.cdz-as-compile-component> spec/semantics` can run the whole-corpus
   diff. This is an unrelated in-flight change, not part of 3l.

**When both land:** retire the interim `run_corpus.py` harness (it exists ONLY because 3l was open) and use
`component-check` — the exact clean differential already written.
Learning: `spec/learnings/2026-07-07-a-bytes-to-bytes-compile-entry-unblocks-the-real-differential-harness.md`.
(Original finding kept below.)

**Finding.** The real self-hosting check is running `compiler.cdz` over the whole corpus via
`component-check`, which feeds each case's canonical AST bytes to a component exporting
`cadenza:compiler/compile : func(list<u8>) -> result<list<u8>, list<diagnostic>>` (the `compiler.wit`
world) and diffs against native `cdz-rustc`. The host side already exists (`component-check`,
`run_compiler_component`, `compiler.wit`). But the **seed can only emit an entry as the nullary `run :
() -> output`** — a `main` that takes the input AST bytes and returns the output component bytes (the
`compile : Bytes → Bytes` seam that IS the self-hosted compiler) declines *"the entrypoint `main` must
take no parameters"* (reproducer: `(module m (def (main b) b))`). So `compiler.cdz`'s `main` must
hardcode one program's bytes, and the corpus differential can't be driven the clean way.

**Why it touches the seed.** This is the top-priority self-hosting *verification* infrastructure gap.
Without it, every emit-frontier feature (item 20) is verified by hand-patching bytes into `main` — an
interim harness (`run_corpus.py`) that, as measured, MIS-classifies (it reports ~147 "disagree" / 0
"mine-declines", counts drift between runs, and "mine" component sizes cluster at 88–102B while native
ranges 89–3332B — the patched bytes mostly never reach the compiler's decode path, so a degenerate stub
is scored as a disagreement). Trusting that table would be the modeled-subsystem trap; only its AGREE
set (real byte-identity) is reliable.

**Status.** 🔴 Seed work (SEED-GAPS gap 3l), top priority for verification. **No corpus case** (it is
compiler infrastructure, not a language behavior; and the interim harness's output is not an oracle to
pin). Fix: when `main` takes one `Bytes`/`list<u8>` parameter and returns `Bytes`/`list<u8>`, lift it as
the `cadenza:compiler/compile` export of the compiler world (or a dedicated `(def (compile ast) …)`
entry / flag), matching what `run_compiler_component` looks up (interface `cadenza:compiler/compile`,
then bare `compile`, then `run`). Once it lands, `component-check` runs `compiler.cdz` over the corpus
as the real differential gate, replacing the byte-patching harness. Learning:
`spec/learnings/2026-07-07-verifying-the-self-hosted-compiler-needs-a-compile-exporting-component.md`.

## 23. 🟢 The self-hosted reader miscompiles unsupported constructs instead of declining — RESOLVED (compiler side) 2026-07-07

**🟢 RESOLVED 2026-07-07 (compiler side).** All three facets of the atom-decode leak now route to `KError` →
`unreachable` (a clean decline), verified by reading `read-node` and by the harness (`hard` 3→0, those cases
moved to `decline`; `01-literals` error 1→0): (1) major-7 `read-node` accepts ONLY the two bool encodings
(info 20/21) and sends float/null/other → the unknown-marker → `KError` (compiler.cdz:664); (2) the unbound
name-reference — a name whose prelude index isn't a parameter — declines instead of emitting `NLocal -1` /
`local.get -1` (compiler.cdz:649–653); (3) any other major (bytes/text/map) → the same marker
(compiler.cdz:665). The remaining reader-side decline-don't-miscompile work — a distinct `error` bucket where
the emission is *invalid* rather than a clean trap — is item **25** (entry selection), NOT this atom-decode
family. Original finding kept below.

**Finding.** `compiler.cdz`'s reader **never declines** an unsupported construct — it emits a
valid-but-WRONG component. Verified: a CBOR float `0xfb` (major 7, info 27) hits `read-node`'s major-7
branch, which assumes a boolean (`arg == 21`?), so `arg 27 ≠ 21` → `NBool 0` → the program returns
`false`. Strings / records / tuples / bytes-ops / host calls have no reader node, so they fall through
to `NInt` / an `NPrim`-of-`"?"` stub. The harness's `0 mine-declines` is this: the compiler always
emits something. This is a reject-don't-miscompile violation *inside the Cadenza-authored compiler*.

**Why it touches the seed/compiler.** Decline-don't-miscompile is a core discipline the spec mandates
for every generation; the compiler's *reader* leaks it on the atom-decode path. It is *unsafe* coverage
— a silent wrong-but-valid component passes a naive "did it build?" check, and when the compiler
eventually compiles its own source a miscompiled construct yields a subtly-wrong compiler, not a clean
failure. The reader already declines an unrecognized *operator head* correctly (`PUnknown → KError →
unreachable`); the atom/literal decode must do the same.

**Status.** ⚪ `compiler.cdz` work (the reader), mirrored by SEED-GAPS. **Corpus:** pinned the
discriminating seed-level fact — `10-bytes.sexp` *"a CBOR simple value that is not a known boolean is
classified as not-a-boolean"* (a major-7 decoder must check the value IS `0xF4`/`0xF5`, not merely
`≠ 0xF5`, so a float/null head is not read as false; three-way classify → -90). Fix: route the reader's
unrecognized major-7 (and any unhandled atom kind / node shape) to `KError`, not a defaulted
`NBool`/`NInt`. **Acceptance signal:** the harness's `mine-declines` count rises from 0 to the number of
unsupported constructs as the reader learns to decline them (and DISAGREE falls correspondingly).
Learning: `spec/learnings/2026-07-07-the-self-hosted-reader-miscompiles-unsupported-constructs-instead-of-declining.md`.

**Update (2026-07-07) — a THIRD facet: the unbound name-reference.** `read-node`'s tag branch (major 6)
resolves a name to `(Node.NLocal (ienv-pos env idx 0))`, where `ienv-pos` returns **-1** for a name not
in the parameter/let environment — used directly as a local slot index with no bounds check. So an
*unbound* name-reference decodes to `NLocal -1` → `KLocal -1` → an invalid `local.get` (uleb of -1 is a
huge index; a validation error or a wrong local — a miscompile either way), rather than a decline. This
is the same violation class as the float→false and string→stub facets (a fall-through to a wrong node),
so the fix is the same: when `ienv-pos` returns -1 (unbound), route to `KError`, not `NLocal -1`. Adds
the name-reference to the list of reader paths that must decline rather than miscompile.

---

## 24. 🟢 A monotone fixpoint loop OOMs the seed when a fresh-re-seeded list parameter is consumed as a list — RESOLVED (seed side) 2026-07-07

**🟢 RESOLVED 2026-07-07 (seed side) — verified by direct probe.** Both OOM reproducers below now COMPILE in
under a second (EXIT=0) where Run 47 saw multi-GB OOM at 40s: (a) fresh-`(list)`-re-seed + list result →
2,978-byte component; (b) the monotone-`recompute` fixpoint → EXIT=0. (Run verification needs the composed
runtime-heap host — these import `cadenza:runtime/heap` — so `wasmtime run` alone errors on the missing import;
the point is the compile no longer blows up.) **Consequence — the return-kind fixpoint LANDED in `compiler.cdz`:**
with 3k fixed, the true fixpoint became expressible in the compiler's own source, and `build-ktab`/`ktab-iterate`
(a monotone fixpoint over the FList) replaced the single-pass stopgap. Verified byte-identical to the seed for a
Bool chain at depth 1/2/3 (108/124/140 B, every func framed `result i32`). Pinned by `09-functions.sexp`
*"a boolean result propagates through a three-deep chain of forwarding functions"* (→ true, byte-identical
131 B — depth-3 is what distinguishes a fixpoint from a single pass). Learning:
`spec/learnings/2026-07-07-the-return-kind-table-is-a-monotone-fixpoint-and-it-propagates-bool-to-any-depth.md`.
(Original finding, with the four-control trigger analysis, kept below.)

**Finding.** The self-hosting return-kind machinery's next step is a monotone **fixpoint** (iterate a table
until it stops changing). The single-pass accumulator fix (item 18) shipped the SINGLE-PASS return-kind table,
but a fixpoint loop still blows the seed up to multi-GB RSS and is killed (`emit`, `ulimit -v 4G`, times out at
30–40s). **`compiler.cdz` needs this to iterate its return-kind table to a true fixpoint** (a depth-2 Bool chain
— a helper whose body is only a call to a Bool helper — needs the fixpoint the single-pass fix doesn't reach).

**Corrected trigger (probed 2026-07-07 — narrower than the SEED-GAPS doc's "fresh re-seed" description).** The
blowup is a **conjunction**, not a single condition. Four controls, run directly against the seed:

| # | shape | result |
|---|-------|--------|
| (a) | `(def (iterate ktab passes) (if (< passes 1) ktab (iterate (list) (- passes 1))))`, `(List.len (iterate (list 1 2 3) 2))` — fresh `(list)` re-seed, list result | **OOM** |
| (b) | `match`-driven `recompute` re-seeded `(list)` inside a fixpoint `iterate`, list result | **OOM** |
| (c) | thread the SAME list param unchanged through the fixpoint, list result | compiles (11,971 B) |
| (d) | fresh `(list)` re-seed each round, result consumed as **Int64** (`List.len` inside) | compiles (633 B) |
| (f) | thread the list and GROW it by `List.push` each round, list result | compiles (12,008 B) |

So the necessary conditions are BOTH: (i) the list parameter is re-seeded with a fresh `(list …)` literal each
round — a value NOT derived from the incoming parameter — AND (ii) the recursion's result is consumed as a list.
Threading the incoming list (c/f), even growing it by `List.push`, compiles; re-seeding fresh while consuming
the result as a scalar (d) compiles. The doc's one-variable trigger ("fresh re-seed") over-broadly condemns (d),
which compiles, and misses condition (ii) — **a fix must target both conditions, not the re-seed alone.**

**Likely mechanism (to confirm).** Same class as the fixed `eval_const` let-memoization blowup and the Tier-00
threaded-accumulator inference blowup — an inference/fold fixpoint that fails to reach a fixed KIND and
re-expands. When the parameter is re-seeded with a literal (not threaded), the incoming value gives no kind
constraint at that argument position, so each pass re-derives it; if the result is also a list, the return-kind
back-propagation (the very machinery item 18 added) must reconcile "fresh literal at the call site" against
"heap result at the use site" every iteration, re-triggering the inline/fold expansion instead of converging.
Threading (c/f) pins the parameter's kind once; a scalar result (d) removes the return-kind constraint.

**Acceptance signal.** Reproducers (a) and (b) `emit` to a valid component within seconds (not OOM); `(List.len
(iterate (list 1 2 3) 2))` = 0 (each round discards the incoming list for a fresh empty one → final `()` →
len 0). Then the return-kind table can iterate to a true fixpoint and the depth-2 Bool chain compiles.

**Pinned (passing side of the boundary — the OOMing program can't be a corpus case, it hangs the gate).**
`05-compound-types.sexp` *"a fixpoint loop that threads a growing list accumulator returns that list"*
(`(List.len (loop (list 1 2 3) 2))` = 5, AGREE) — proves threaded list accumulators in a fixpoint are
representable today and marks exactly where the frontier begins.
Learning: `spec/learnings/2026-07-07-a-fixpoint-loops-blowup-is-fresh-re-seed-plus-list-result-not-the-loop.md`.

---

## 25. 🔴 The self-hosted compiler selects the module entry by POSITION (first def), not by the name `main` — blocked on the seed compile-time-evaluator blowup (gap 3m)

**Finding.** `compiler.cdz`'s `read-module` takes the **FIRST def** as the nullary `run` entry (positional);
the native seed selects the def **named `main`** and reorders it to index 0. When they coincide (a main-first
module) the compiler is CORRECT end-to-end — `(def (main) (f 41)) (def (f x) (+ x 1))` emits a valid component
that runs = 42, a forward call to a later-defined helper included. When they don't (a helper-first module, the
common shape), positional func 0 is the parameter-taking helper, which `entry-guard` forces to a nullary
`KError` trap — a clean **decline**, not invalid bytes. So the multi-def user-function CALL machinery is
complete; the only gap is entry SELECTION.

**Verified (current compiler, probed 2026-07-07).**

| module | core func 0 | result |
|--------|-------------|--------|
| `(def (main) (f 41)) (def (f x) (+ x 1))` — main FIRST | main (nullary) | valid, runs = 42 ✅ |
| `(def (f x) (+ x 1)) (def (main) (f 41))` — helper FIRST | f (param'd) | valid, traps (clean decline) |
| `(def (main) (g)) (def (g) 42)` — main first, nullary callee | main | valid, runs = 42 ✅ |

Before the fix landed (caught mid-probe as the spike edited compiler.cdz live) the helper-first case emitted an
**invalid** component — `f` lifted as nullary func 0, `main` doing `i64.const 41; call 0`, `run` exporting func
0 — so the argument stranded: *"values remaining on stack at end of block."* `entry-guard` now makes that a
clean decline (single `run → unreachable`).

**Why it's blocked (not just unimplemented).** The name-based reorder IS written — `find-main` / `visit-def` /
`skip-main-nth` walk the def named `main` to index 0 — but adding those recursive functions to the LIVE compile
path tips the **seed's compile-time evaluator into an exponential blowup** (>1.6 GB OOM at this compiler's
scale): the recursive-inline / compiler-exponential-in-nesting family (SEED-GAPS **gap 3m**, and see
`compiler-exponential-in-nesting-depth`). So the reorder is reverted; positional entry + `entry-guard`'s clean
decline is the correct interim. **The entry reorder is gated on fixing the seed's compile-time-evaluator
blowup, not on any reader or backend capability.**

**Acceptance signal.** With gap 3m fixed, `find-main`/`skip-main-nth` can rejoin the live path; a helper-first
module then compiles (`f` reordered off the entry slot) instead of declining — the harness's helper-first
`decline`s become `agree`/`soft`, and the `error` bucket (invalid emissions) empties as every remaining
unsupported construct traps cleanly rather than emitting invalid bytes.

**Pinned (the working side — a main-first module).** `09-functions.sexp` *"the module entrypoint is the def
named main regardless of its position"* (`(def (main) (f 41)) (def (f x) (+ x 1))` → 42, AGREE) — pins that
entry selection is by NAME (a language requirement every other multi-def case, all main-last, left unpinned)
and doubles as a forward-reference pin.
Learning: `spec/learnings/2026-07-07-the-self-hosted-reader-compiles-a-multi-def-call-but-picks-the-entry-by-position.md`.

---

## 26. 🟠 The differential gate needs a trap-CAUSE discriminator — a decline and a semantic trap are indistinguishable by value alone (measurement gap, not a compiler bug)

**Update (2026-07-07) — interim harness side DONE.** `run_corpus.py` now disassembles the built component's
entry func (`is_bare_decline`) and splits the old `trap-ok` into **`trap-ok`** (RAN and trapped with real logic
before the trap — a verified semantic trap, e.g. `(/ 5 0)` → `i64.div_s`) vs **`trap-dc`** (a bare `unreachable`
decline that only coincidentally lands on a trap oracle). Verified: the four `Bytes.of`/missing-field cases moved
`trap-ok 4 → trap-dc 4`; a real `i64.div_s` semantic trap scores `is_bare_decline=False` (would be `trap-ok`).
So the interim harness no longer overstates conformance — a `trap-dc` reads as `decline` (frontier), and when a
construct gains real support its check moves to `trap-ok` (a wrong check surfaces as `hard`). **Still open:** the
real `component-check` differential (#22, unblocked seed-side) has the SAME blind spot and NO such discriminator
yet — it compares native-vs-mine values/traps in Rust and would count a decline-trap as agreeing with a
semantic-trap. The cheapest fix there is the in-range companion rule (below); the disassembly heuristic is
interim-harness-only. This item stays 🟠 until `component-check` gains a trap-cause check.


**Finding.** A value-first differential comparison cannot tell a **semantic trap** (the compiler executed the
trapping semantic — a byte-range check, a zero-divisor `i64.div_s`) from a **decline** (an unsupported construct
lowered to `KError → unreachable`): both produce the identical observable, a trap. On a *value*-expecting case
the two are distinct (a decline traps where a value is wanted → scored `decline`, the honest frontier); on a
*trap*-expecting case the distinction collapses and a decline scores as a correct trap. Verified 2026-07-07: all
four realized `trap-ok` cases in the interim harness (`Bytes.of` out-of-range/negative/runtime, missing field)
are bare-`unreachable` declines — `compiler.cdz` doesn't support `record`/`Bytes.of`, so it never examines the
byte value (a VALID in-range `(Bytes.of (list 65 66))` also traps). Right observable, wrong reason.

**Why it matters.** Not today's behavior (declining is correct now) but **masking**: when a construct gains real
support, a WRONG trapping check (off-by-one range, or no trap on a valid byte) would still score `trap-ok`/agree
for the out-of-range cases and regress silently — the comparison never distinguished the decline from the check.
A green trap-ok/trap-agree count reads as "these trapping semantics are conformant" when it can mean "these
constructs are unsupported and decline." This applies to BOTH the interim `run_corpus.py` (caveat added to its
README) AND the eventual `component-check` differential (SPEC-BACKLOG #22, now unblocked seed-side) — a
trap-vs-trap match agrees whether the trap is semantic or a decline.

**Fix (cheapest discriminator).** Pair each trap-expecting case with an **in-range companion that must NOT
trap** — e.g. alongside "byte out of range traps" `(Bytes.of (list 256))`, an in-range "byte in range yields a
sequence" `(Bytes.of (list 65 66))` that must produce a value. A decline traps on BOTH (fails the in-range
companion → visible); a correct implementation traps only on the out-of-range one. The in-range companion is the
discriminator a value-only trap oracle lacks. Most such in-range companions already exist as value cases; the
measurement fix is to REQUIRE the companion pass before a trap-expecting case's trap counts as conformance (a
gate rule / harness convention), not new corpus content. Until then, read every trap-agree as "traps, reason
unverified."
Learning: `spec/learnings/2026-07-07-a-decline-that-lands-on-a-trap-oracle-is-coincidental-agreement-not-a-semantic-trap.md`.

---

## 27. 🟢 Seed gap 3n: the `compile`-component RETURN trips "return pointer not aligned" — RESOLVED (seed side) 2026-07-07

**🟢 RESOLVED 2026-07-07 (seed side) — verified by the loop.** The `(p+3)&!3` retarea-alignment fix landed (a
seed rebuild). Re-probing every input that failed last cycle — `(main) 5`/`0`/`1`/`true` (input len 31), `1000`
(33), `(mmm)…42` (34), `if->42` — **all now return `Ok`**, across all mod-4 residues. The self-hosting
`compile-run` loop works for ARBITRARY programs; `compiler.cdz` is byte-identical to native on `(main) 42`/
`(< 3 5)`/depth-2 Bool chain, `soft` on `(+ 20 22)`/`(dbl 21)`. **Follow-on to reach the byte gate is item #28.**
Learning: `spec/learnings/2026-07-07-gap-3n-fixed-the-self-hosting-loop-is-operational-and-the-byte-gate-is-one-step-away.md`.
(Original finding below.)

**🔎 ROOT CAUSE (2026-07-07) — INPUT-LENGTH mod 4, converged with the compiler agent.** The failure is a
deterministic function of the input AST byte length mod 4: `input_len % 4 == 0` → OK, otherwise *"not aligned"*.
(The loop first read this as parity from an under-sampled table — all its even cases were also ≡ 0 mod 4 — then a
len ≡ 2 probe (`(module mmm (def (main) 42))`, AST len 34, FAILS) and a cross-check against the agent's own
`SEED-GAPS` note settled it as mod 4. The agent independently reached the same diagnosis and fix.) Progression of
proxies: "fails at every size" → "value threshold at 24" → "parity" → **mod 4**; 24 was a proxy because it is the
CBOR 1→2-byte int boundary that flips input AST length 31→32.

| input | input-AST bytes | mod 4 | result |
|---|---|---|---|
| `(main) 5` / `23` / `true` | 31 | 3 | FAIL |
| `(main) 42` | 32 | 0 | OK |
| `(main) 1000` | 33 | 1 | FAIL |
| `(mmm) (main) 42` | 34 | 2 | FAIL |
| `(main) (+ 2 3)` / `(< 3 5)` | 36 | 0 | OK |
| `(id 5)(def (id x) x)` | 48 | 0 | OK |

**The bug (agent's diagnosis, loop-confirmed):** the `compile` core wrapper copies the input `list<u8>` into
linear memory at the bump pointer, then allocates the RETURN area (the canonical-ABI `retptr`, which must be
4-aligned) at `bump_ptr` WITHOUT re-aligning — so `retptr = base + input_len`, 4-aligned only when
`input_len % 4 == 0`. **The fix:** align the bump pointer up to 4 (`(p + 3) & !3`) before allocating the return
area, or place the retarea at a fixed aligned offset independent of input length. Minimal repro: `(module m
(def (main) 5))` (31 B, FAIL) vs `(module m (def (main) 42))` (32 B, OK), `cadenza-seed compile-run
<compiler.cdz> <it>`.

---

**Status.** The self-hosting loop is functionally CLOSED: `compiler.cdz`'s entry is now `(def (compile b)
(compile-bytes b))` (gap 3l build path), and `cadenza-seed compile-run <compiler.cdz> <input.cdz>` compiles
`(module m (def (main) 42))` → the correct **89-byte component** through the full pipeline. The ONLY blocker to
adopting `component-check` as a byte-level gate is the seed's `compile`-RETURN marshalling: it trips *"running
the compiled compiler: return pointer not aligned"* for many inputs.

**Status.** The self-hosting loop is functionally CLOSED: `compiler.cdz`'s entry is now `(def (compile b)
(compile-bytes b))` (gap 3l build path), and `cadenza-seed compile-run <compiler.cdz> <input.cdz>` compiles
`(module m (def (main) 42))` → the correct **89-byte component** through the full pipeline. The ONLY blocker to
adopting `component-check` as a byte-level gate is the seed's `compile`-RETURN marshalling: it trips *"running
the compiled compiler: return pointer not aligned"* for many inputs.

**Corrected characterization (probed against the CURRENT seed 2026-07-07 — the SEED-GAPS 3n note is stale).**
1. **The fixed-output repro the doc cites is now FIXED.** `(def (compile b) (Bytes.of (list 0 0 0 0)))` (and
   sizes 0–4, and the identity `(def (compile b) b)`) all return cleanly today — a partial fix landed. The doc's
   "fails at EVERY size 4..128" no longer reproduces.
2. **The real compiler's failure is a SHARP DETERMINISTIC VALUE THRESHOLD, not "allocation-dependent."**
   Compiling `(module m (def (main) N))` for a bare integer `N`: **N ≤ 23 → "not aligned", N ≥ 24 → OK**
   (bisected exactly). Both sides emit an **identical 89-byte** component — same size, differing only in the one
   `i64.const` operand byte. `0`/`1`/`true`/`256`/unfolded-`if` fail; `42`/`(< 3 5)`/`(dbl 21)`/depth-2/3 Bool
   chains succeed. So the SIMPLEST inputs (`0`, `1`) are the minimal reproducer — opposite the doc's implication
   that `42`-class inputs being safe means trivial ones are.

**Minimal reproducer:** `(module m (def (main) 23))` fails, `(module m (def (main) 24))` succeeds — both 89-byte
output, deterministic across runs. `cadenza-seed compile-run <compiler.cdz> <that-input>`.

**Root (to confirm).** Not the compiler (`compiler.cdz` `emit`s all these byte-identically to native) and not
the wrapper's static marshalling (fixed-output is aligned now). It is the seed's marshalling of a **computed**
`list<u8>` whose bytes live in a runtime `Bytes` ROPE: the return pointer's alignment depends on where the
flattened rope buffer lands in linear memory relative to the retarea, and that offset is a function of the bump
allocator's state during `compile-bytes` — which for tiny programs correlates with the operand value, crossing
an alignment boundary at 24. The value threshold is a proxy for an internal allocation count shifting the bump
pointer. **Agent action:** make the `compile` export's computed `list<u8>` return robustly 4/8-aligned
regardless of the returned rope's heap position (align the retarea/return pointer independent of allocator
state), then `component-check` can grade the corpus.

**Consequence.** `component-check` cannot be adopted yet — it fails even the `42` case where native cdz-rustc
passes, confirming the bug is the seed's return wrapper, not the ABI. The compiler's test loop stays the interim
value-first `emit`-based harness (runs the emitted component via `run()`, sidestepping the compile-return path).
Related: #22 (gap 3l, resolved). Learning:
`spec/learnings/2026-07-07-the-self-hosting-loop-runs-end-to-end-but-the-compile-return-trips-on-a-value-threshold.md`.

---

## 28. 🟢 Adopt `component-check` as the byte-level self-hosting gate — WIRING DONE (`--emit-component` landed); gate now RUNS, but its `disagree` count needs a decline discriminator (→ #29)

**🟢 WIRING DONE 2026-07-07.** The seed gained `compile-run <compiler.cdz> --emit-component <path>`, which
persists the Cadenza-authored `cadenza:compiler/compile` component (verified: `compiler.cdz` → 27 KB component).
`component-check <that> spec/semantics` now RUNS the whole-corpus byte diff: **58 agree, 496 disagree, 204
skip**. The gate is live. ⚠️ But the raw `disagree` count is misleading — see #29: 158 of the 496 are the
byte-identical 88-byte `func 0 → unreachable` DECLINE stub, not miscompiles. The `agree` count (58, byte-identical
to native) is the trustworthy signal; the `disagree` count needs the decline discriminator before it means
anything. Original scoping below.

**Finding.** With gap 3n fixed (#27), the self-hosting `compile-run` loop works for arbitrary programs and
`compiler.cdz` is byte-identical to native on the programs where byte-identity is expected. The byte-level GATE
— `cadenza-seed component-check <component.wasm> spec/semantics`, which already does the whole-corpus
native-vs-component byte diff — is now unblocked in principle, but cannot yet be pointed at the Cadenza-authored
compiler: `component-check` reads a compiler component from a fixed crate path (`crates/cdz-compiler-component/…
cdz_compiler_component.wasm` — the RUST cdz-rustc-as-component), and `compile-run` builds the *compiler.cdz*
compile-component in memory but never writes it to disk.

**The wiring step (seed, small).** Add a subcommand (or a `compile-run --emit-component <path>` flag) that
PERSISTS the compiler.cdz-built `cadenza:compiler/compile` component to disk. Then `component-check <that>
spec/semantics` grades the whole corpus at the byte level — the real differential self-hosting gate, replacing
the interim value-first `emit`-based harness. This is pure plumbing (the component already builds and validates;
`compile-run` proves it runs), not a compiler or language capability.

**Remaining after that (separate, later).** Corpus REJECTION cases need the diagnostics ABI — the `compile`
export returning `result<list<u8>, list<diagnostic>>` and a way to construct diagnostics — since compiler.cdz's
only failure channel today is a TRAP (`KError → unreachable`), no CDZ code. So `component-check` grades SUCCESS
cases (byte-identical / value) the moment the component is persistable; rejection cases wait on the diagnostics
gap (already noted in `compiler.cdz`'s entry comment and gap notes).
Learning: `spec/learnings/2026-07-07-gap-3n-fixed-the-self-hosting-loop-is-operational-and-the-byte-gate-is-one-step-away.md`.

---

## 29. 🟠 `component-check` scores an honest DECLINE as a DISAGREE — the byte-level gate needs the same decline-vs-result discriminator as #26 (measurement, not a compiler bug)

**Finding.** The byte-level self-hosting gate `component-check` (now runnable, #28) compares the Cadenza-authored
compiler's output to native `cdz-rustc` BYTE-for-byte and buckets each case `agree`/`disagree`/`skip`. It has NO
notion of a decline: when `compiler.cdz` declines a construct it can't read, it emits a valid TRAPPING component
(`func 0 → unreachable`, `KError`), and `component-check` byte-compares that decline stub against native's real
output and scores `disagree`. First run: **58 agree, 496 disagree, 204 skip** — but **158 of the 496 emit the
byte-IDENTICAL 88-byte component**, which disassembles to a bare `func 0 → unreachable` decline (verified: two
different unhandled programs `(record (x 1))` / `(tuple 1 2)` → same 88 bytes; it traps when run). So the
`disagree` count conflates honest declines (records/strings/floats/effects the reader doesn't decode yet) with
genuine miscompiles (a component that RUNS to wrong bytes). Spot-checked one real non-decline disagreement:
`(effect E (op)) (def (main) 5)` compiles to `i64.const 5` (effect decl dropped) — that RUNS, so it is a true
disagreement, but it is lost among 496.

**This is the byte-level twin of #26** (the interim harness's trap-cause discriminator) and of the trap-oracle
learning — every differential gate inherits the decline-vs-result blind spot: value oracle (decline traps where
a value is wanted — visibly distinct), trap oracle (decline ≡ semantic trap — needs trap-cause check), byte gate
(decline stub ≡ wrong-bytes miscompile — needs entry-func check).

**Fix (cheapest, same shape as #26).** In `component-check`, before scoring `disagree`, check whether the
component's entry core func is a bare `unreachable` (no computational op — arith/cmp/call/const-then-check —
before the trap). If so, classify the case **`decline`** (the honest frontier), not `disagree`. Then the
`disagree` count means genuine miscompiles ONLY — a component that computed and got the bytes wrong. Until then,
read `component-check`'s output as: `agree` (58, trustworthy — byte-identical is unforgeable) is the real signal;
`disagree` = declines + real miscompiles combined, and the real miscompiles must be enumerated separately (the
non-`unreachable` disagreements) before the number is meaningful.
Learning: `spec/learnings/2026-07-07-the-byte-level-self-hosting-gate-runs-and-its-disagree-count-conflates-declines-with-miscompiles.md`.

---

## Cross-references

- Seed implementation gaps: `implementation/compiler/SEED-GAPS-FOR-SELF-HOSTING.md`
- Runtime/WIT requests: `implementation/RUNTIME-REQUESTS.md`
- Design decisions log: `implementation/DECISIONS.md`
- Effect-lowering design: `implementation/DESIGN-effects-lowering.md`
- Spike findings (07-05): `implementation/compiler/SPIKE-FINDINGS.md`
