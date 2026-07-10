# The implementation's design directions fold into the durable architecture — and records-everywhere is the foundation to build first

*2026-07-10*

**What happened.** The `implementation/` tree had accumulated a dozen design documents written against the
native reference compiler `rcdzc` as it was being built — a records-everywhere resolver refactor, binding
patterns, collection-and-binary patterns, effects, a CHAMP map, inline-handle tagging, a bytes rope, integer
widths, a configurable non-wasm backend, and several migration/retirement plans. Each was an
implementation-dated spec with line numbers, struct names, and staged increments — the ephemeral form. The
operator's instinct was that the *durable* content of these should be lifted into the durable compiler
architecture (`spec/architecture/`) so a future generation is built to the target shape from documentation
alone rather than refactored into it, and that **records-everywhere is not one direction among many but the
foundation the others rest on** — it fixes how everything is evaluated at compile time. A survey of all
twelve, sorting durable architecture from ephemeral implementation detail and from already-covered
declared-default choices, confirmed that instinct and produced a clean fold.

The survey's findings, by direction:

1. **Records-everywhere is the keystone, and it is a *mechanism*, not a new principle.** The reference
   architecture already fixes the *principle* — the resolver recognizes values, not names; a type, a
   constructor, and a pattern are ordinary values
   ([reference-compiler.md §Nothing Is Privileged By Name](../architecture/reference-compiler.md)). What the
   records-everywhere design adds is the enforceable *mechanism*: resolution is exactly two generic operations
   over one map — a single ordered lookup that returns the bound value verbatim, and one generic member
   projection that never inspects its key — plus a fixed, closed set of grammar names; a name is resolved
   under an explicit *mode* (value / key / pattern) so a name's interpretation is a stated rule of its
   position, not a shape inspection of its surroundings; and a built-in type is an ordinary *record carrying a
   meta channel* — reserved fields, distinct from its operation fields, that answer "what does this name mean
   as a type," read lazily by the type pass at the use site rather than rewritten by the resolver. This is
   what makes the principle enforceable: the honest invariant is that the set of spellings the resolver
   matches is the fixed grammar set and *does not grow* when a built-in is added — every new named thing is a
   map entry, every new kind of compile-time knowledge is a meta field. It is the foundation because it fixes
   how a name acquires meaning, which every later pass reads; it belongs built first.

2. **Integer widths are a worked example of records-everywhere, not a separate feature.** An integer type is
   one type-record whose meta channel carries `(signed, width)`; `Int64` is `(Int 64)`, a width is one prelude
   entry, and an unusual width like `(UInt 48)` is a type the compiler *computes* rather than a hand-written
   case. The signedness meta *resolves the arithmetic-versus-logical right-shift question* the earlier
   IR-shape research flagged as open: signedness selects the machine operation (signed vs. unsigned shift and
   comparison) and the range an overflow check tests. The durable rules — overflow traps per width as the
   width-parametric generalization of the checked-integer core, checked-versus-wrapping conversions, and *no
   implicit promotion* (two integer types unify only at equal width and signedness) — are numeric-model
   semantics; the record-carries-its-meta shape is the architecture that makes them one model.

3. **Effects are records too, and their manifest is computed, not declared.** An effect resolves to a record
   of its operations reached by the one generic projection; a performed operation is a value the pipeline
   carries. This slots directly under the already-normative classify-first/monomorphize story
   ([reference-compiler.md §Effects Are Classified First](../architecture/reference-compiler.md)). The
   genuinely new durable content the survey surfaced was the *declaration surface and boundary*: an effect is
   declared routing-agnostically (no host/in-program marker), a capability *is* the declared interface of a
   delegated effect, and the component's import manifest is the *computed union* of the effects a program both
   delegates to the host and reaches from an entrypoint — an effect discharged by a nearer in-program handler
   never escapes and is absent from the manifest, and each delegated effect crosses as its own named interface
   so two effects sharing an operation name do not collide. The suspend/resume story reduces to a durable
   principle already half-stated: the language guarantees only deterministic re-execution; *how* a host
   suspends is host policy the emitted bytes never encode, which is exactly why a reified intra-program
   continuation must not span a host call.

4. **Patterns are one engine, and a binding is a single-arm match.** The binding-patterns and
   collection-and-binary-patterns designs both confirm and *extend* the "a pattern is an ordinary expression,
   distinguished only by a binder and a wildcard leaf" principle: there is no pattern node type, and every
   kind of match — sum, product, string, list, map, bit-string — runs through *one engine* whose arm is a
   conjunction of *probes* (opacity-respecting observations that succeed or fail) and *binders* (extractions),
   with a match a top-to-bottom disjunction of arms. A binding position is that engine applied to a single
   irrefutable arm. The foundations a fresh build must get right early: the accept/reject/decline triad at the
   point the match is decided (a coverage defect and a shape defect are distinct machine-readable codes; an
   unbuilt category declines rather than rejects), first-match order preserved under probe sharing, binders
   scoped to the path their probes succeeded on, and linearity checked across the *whole* nested pattern. A
   new category is a new kind of probe, never a parallel matching path.

5. **A backend is a function of the typed core and a neutral layout — and the flat rung is a *backend's*
   representation.** The backend-retargeting design showed the pipeline is already target-neutral up to one
   seam: everything establishing *what a program means* (resolution, inference, compile-time evaluation, the
   poison and erasure checks) and the boundary *layout* (exports by name with solved types, reachability) is
   computed once and consumed by whichever backend is chosen. The sharp correction it forces on the
   immediately-prior IR-shape learning
   ([[2026-07-10-the-pipeline-is-a-tree-above-and-a-flat-anf-core-below-and-ssa-is-a-property-not-a-fourth-ir]]):
   **the fully-linearized basic-block form is a linearizing backend's representation of the core, not a shared
   rung every target descends through.** The A-normal core names every intermediate *value* but keeps
   *structured* control; a backend whose target has structured control flow consumes that directly and never
   builds the block graph, while a backend targeting a linear instruction stream produces it. A backend
   chooses a value strategy (handles into the shared runtime, or the target's native aggregates) that is
   unobservable to a value-to-value computation and must state where it stops being invisible (cheap
   many-version persistence). A backend inherits the front's decline boundaries and may widen them only where
   its target genuinely expresses more.

6. **The runtime-representation designs are declared-default, save one gap.** CHAMP maps, inline-handle
   tagging, and the bytes rope realize principles already normative in
   [value-heap-runtime.md](../architecture/value-heap-runtime.md) — unobservable representation, canonical
   value forms, deferred materialization behind observable bytes, a small value riding inline so no inline/heap
   twin coexists. Their concrete choices (the trie width, the fixnum window, the flatten-on-read policy) are
   declared-default facts, not architecture. The one durable *gap* they exposed: the runtime's
   no-recursion-in-proportion-to-depth guarantee was stated only for reclamation, but a deep key or deep rope
   must not exhaust the host stack on *hashing, comparison, or materialization* either — a valid value must
   never crash the runtime by the act of being read.

The retirement and one-interface (`xtask`) designs are project-management and dev-ergonomics; they carry no
durable architecture (their few near-architectural notes — the compiler builds components itself, the
build-tool ABI is bytes-to-bytes — are stated normatively elsewhere).

**Why.** These directions read as separate features but are one architecture seen from different sides,
because they all rest on the same two decisions. The first is records-everywhere: once a built-in is a record
carrying a meta channel and resolution is one generic lookup plus one generic projection, a *width* is a
record, an *effect* is a record, a *module* is a record, a *type* is a record — the constructs a naive
compiler special-cases each collapse into the one model, and the compile-time evaluator that reduces records
is the one place their meaning lives. That is why records-everywhere is the foundation built first: it is not
a feature that sits beside the others but the substrate that makes the others cheap, and building it late
means every feature added before it accretes the name-dispatch special cases it exists to forbid. The second
is solve-once on a resolved, typed core: patterns, effects, and backends all consume a meaning fixed above
them and must not re-derive it — a pattern engine reads the type to tell a constructor from a binder, effect
lowering reads the statically-resolved handler, and a backend reads the solved types off the core. The reason
to fold these into the durable architecture *now*, rather than leave them as implementation notes, is the
operator's core observation: a refactor of an existing compiler into this shape is expensive and a fresh
authoring to a documented target is cheap, so the target belongs written as durable, engine-free architecture
that a future generation is built *to*, not migrated *toward*
([overview §16](../overview.md); [constitution §XII](../../constitution.md)).

**The requirements it drove.** Two new architecture documents and targeted additions to three existing ones,
all naming no engine or prototype (per [constitution §XIII](../../constitution.md); the prior-art grounding
lives here):

- [prelude-and-resolution.md](../architecture/prelude-and-resolution.md) — the keystone: resolution is two
  generic operations over one map plus a fixed grammar; a name is resolved under an explicit mode; a built-in
  type is a record carrying a meta channel whose meaning is read at the use site; the enforceable discipline
  that the matched-spelling set does not grow. The mechanism companion to
  [reference-compiler.md §Nothing Is Privileged By Name](../architecture/reference-compiler.md), and the
  worked-example requirements that a numeric width and an effect are each this one model.
- [backends-and-targets.md](../architecture/backends-and-targets.md) — the pipeline is target-neutral up to
  one seam; a backend is a function of the typed core and a neutral layout; the flat instruction rung is a
  linearizing backend's representation, not a shared rung; a backend's value strategy and its inherited
  decline boundaries. Refines
  [reference-compiler.md §Instruction Selection](../architecture/reference-compiler.md) and
  [intermediate-representations.md](../architecture/intermediate-representations.md).
- [reference-compiler.md](../architecture/reference-compiler.md) — a new §Matching Is One Engine Of Probes And
  Binders (the unified engine, binding-as-single-arm-match, the coverage/shape/decline distinctions, whole-
  pattern linearity), and two new effect requirements (an effect declared routing-agnostically and routed by
  delegation; the manifest as the computed union of delegated, reached effects, each its own named interface).
- [intermediate-representations.md](../architecture/intermediate-representations.md) — corrected so the flat
  block form is a linearizing backend's representation rather than a rung every target passes through.
- [value-heap-runtime.md](../architecture/value-heap-runtime.md) — the no-recursion-in-depth guarantee
  generalized from reclamation to hashing, comparison, and materialization.

The CHAMP / inline-handle / rope representation choices, the width trie/window constants, and the retirement
and one-interface plans are recorded as declared-default or project-management, not folded into normative
architecture.
