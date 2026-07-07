# Runtime strings landed — the keystone unblocked, and the front rung now resolves a name to a code

*2026-07-07*

**What happened.** Runtime `String` — flagged for weeks as the keystone Tier-0 blocker of a
self-hosting front end ([[2026-07-06-authoring-the-compiler-surfaces-gaps-a-corpus-grown-from-a-floor-misses]]) —
landed in the seed. Every Tier-0 probe that previously declined now compiles and runs: a string as a
function parameter (`String.byte-len s` on `"hello"` → 5), runtime string equality dispatch
(`(if (= s "def") 1 0)` → 1), a string returned across a call (identity → `"hello"`), and a string
carried as a runtime sum-variant payload (`(Node.NSym "hi")`, bound by a `match` → measured). With
strings available, the compiler-in-Cadenza spike rewrote its front rung to resolve a form's head by
**name** rather than by a pre-assigned integer opcode: `main` now compiles `(+ 20 22)` from a
**string-headed** surface node `(NPrim (tuple "+" (NInt 20) (NInt 22)))`, and `resolve` maps the head
`"+"` to a typed `Prim` variant via `head-prim` (a chain of `(= s "+")`, `(= s "-")`, … comparisons),
then an exhaustive `Prim → Core` match selects the constructor. An unrecognized head resolves to
`PUnknown`, on which `resolve` declines — a real front-end diagnostic point. The whole pipeline
(resolve-a-name → fold → lower → serialize → frame) now runs end-to-end from a name-headed tree to the
89-byte component, folding `(+ 20 22)` to `i64.const 42`.

**Why.** This is the moment the "resolve names to codes before selecting instructions" property
(compiler-pipeline.md §Representation) stopped being an aspiration expressed over hand-assigned
integer opcodes and became the *actual* front-rung behavior over the strings a reader produces. The
significance is threefold. First, **name dispatch is what a front end fundamentally is** — a reader
hands the compiler head *symbols*, and turning `"+"`/`"def"`/`"module"` into typed codes is the first
real pass; without runtime strings the compiler could only be driven from pre-coded input, which is
not a front end. Second, `head-prim` is the honest realization of the resolve seam: the head string is
looked up *once*, at the resolve boundary, and no string survives into `Core` — so everything
downstream reads a resolved `Prim`/`Core` constructor, never re-inspects a head, exactly as the
resolved-IR direction requires ([[2026-07-06-lower-through-a-resolved-ir-so-emission-is-a-serializer]]).
Third, `PUnknown → decline` extends reject-don't-miscompile to the *surface*: an unrecognized head is
a front-end rejection, not a silent fall-through to a wrong operation. Note the spike also sidestepped
the still-open nested-binder blocker (Tier 2b / SPEC-BACKLOG item 1) by giving the surface node a
**flat** payload `(NPrim (Tuple String Node Node))` — a string head with two flat sub-node operands —
rather than the nested `(tuple op (tuple a b))` that declines; so runtime strings unblock the *name*
resolution while the *nested-payload* decode remains blocked, and both are needed before a reader
producing arbitrary-arity forms can be decoded. This landing closes the keystone but not the whole
front end.

**The requirement it drove.** A conformance case in `13-strings.sexp` — *"a multi-way string-head
dispatch resolves an operator name to its operation"* (`(eval-head "+" 20 22)` mapping the head string
through a chain of comparisons to the arithmetic, → 42) — pins the compiler's front-rung idiom
end-to-end: a multi-way head resolution over runtime strings, distinct from the existing
single-comparison dispatch case in that several head names each select a distinct operation (what a
real head resolver is), with the fall-through default standing in for the front end's decline on an
unknown head. It PASSES, joining the runtime-string parameter / equality-dispatch / sum-payload /
return-across-boundary cases a sibling session already pinned (all now green) to witness that Tier 0 is
closed. The keystone learning it records: **runtime strings were the gate to name-based dispatch and
the symbol table, and clearing them turns "resolve names to codes" from an integer-opcode stand-in
into the real front rung.** The remaining front-end work — decoding arbitrary-arity forms (needs the
nested-payload binder, backlog item 1) and the CBOR reader / symbol table — is now the critical path,
recorded in the spike handoff.
