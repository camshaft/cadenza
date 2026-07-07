# Reading a field off a runtime record completes "read your own input" — the record twin of runtime tuple projection, and the shape must be threaded per binding form

*2026-07-07*

**What happened.** With the diagnostics pipeline landed (ask-41 the `{artifacts,diagnostics}` return, ask-46 the
compile-entry handler), the next hop for a self-hosted compiler is the most basic one: **read the AST out of its
own input.** `compile`'s input is a `list<artifact>` (each `artifact = record{bytes: list<u8>, kind: string}`),
so reading the program means `(List.at inputs 0)` → an `Option<artifact>`, then projecting `.bytes` off the
artifact record. That projection declined: `runtime compound element of a kind the runtime cannot box yet`.

The fix had three parts, and each is an instance of a pattern the loop has already seen:

1. **A runtime `(. r f)` emits `arr-get` at the field's sorted-key slot, unboxed by the field's shape** — the
   exact twin of the runtime `tuple.N` fix (see
   [[2026-07-07-runtime-tuple-projection-needs-a-let-and-the-direct-path-miscompiles]]). `(. r f)` previously had
   only the compile-time-structural path; a field projected off a genuine runtime record handle emitted
   `unreachable`, which poisoned the enclosing constructor. Records and tuples are one representation (a
   slot-indexed heap array); projection off either at runtime is "index the slot, unbox by the static shape."
2. **A `match` binding a Heap payload to a bare name now carries that payload's Shape** — the same
   shape-through-match mechanism as [[2026-07-07-shape-inference-through-match-unblocks-the-type-driven-emit-spine]].
   `((Some a) …)` over a `list<artifact>` scrutinee gives `a` its `record{bytes,kind}` shape, so `(. a bytes)`
   resolves instead of hitting a `None` shape.
3. **The `compile` entry's `inputs` parameter is given the fixed `list<artifact>` shape** — the ABI-boundary
   seed of the shape that parts (1) and (2) then propagate inward.

I verified all three on the refreshed stable seed: the match-idiom projection (`(match (List.at inputs 0) ((Some
a) … (. a bytes) … (. a kind) …) ((None u) …))`) compiles VALID and echoes a fed input's bytes into a component
artifact. ask-49 (the compound-returning effectful handle on the run entry) still declines — a separate, still-open
frontier.

**Why.** The sharp lesson is the follow-on that DIDN'T get fixed. `(. (Option.expect (List.at inputs 0) "x")
bytes)` — the same field projection, but binding the artifact through `Option.expect` instead of a `match` arm —
still declines `runtime compound element of a kind the runtime cannot box yet`. So the shape-carrying that part
(2) added is **per-binding-form**: the `match` arm threads the payload's shape to the bound name, but
`Option.expect`'s runtime unwrap does not (yet). This is the recurring shape of every runtime-compound landing in
this compiler — a capability isn't "runtime records can be projected," it's "runtime records can be projected
*when the record reaches the projection through THIS binding construct*," and each construct that can bind a
runtime compound (a `match` arm, a `let`, an `Option.expect`/`Result` unwrap, a function parameter, a tuple
destructure) must separately learn to thread the static Shape to what it binds. The shape is the thing that must
flow, and it flows along binding edges one construct at a time. The loop's job at each such landing is to probe
the OTHER binding forms for the same value — the fix that lands via `match` names, by its silence, the ones that
still don't (here, `Option.expect`), which become the narrow follow-on asks. "Runtime record field access works"
is true and false at once: true through a match binder, false through an expect unwrap, and the difference is
exactly which binding forms have been taught to carry the shape.

**The requirement it drove.** No new corpus case this cycle — the record-projection value behavior belongs in the
corpus, but the natural case (`compile` projecting `.bytes` off its `list<artifact>` input) is a compile-entry
ABI shape, not a `run`-entry `(output (: v T))` value the behavior gate expresses, and the compile-entry
self-hosting path is exercised by the byte gate (`component-check`), not the value corpus; a plain runtime-record
`(. r f)` returning a scalar under a `run` entry would be corpus-expressible and is the follow-on to add. The
output is this learning (the record/tuple-projection unification and the per-binding-form shape-threading rule)
and the confirmed follow-on boundary (`Option.expect` field projection still declines — a narrow ask for the
compiler agent). General lesson: **a runtime-compound-access capability is scoped to the binding CONSTRUCT the
value arrives through, not to the access itself; when a fix lands via one binder (`match`), probe every other
binder for the same value (`let`, `expect`, parameter, destructure) — the ones that still decline are the fix's
own map of its remaining follow-ons.**
