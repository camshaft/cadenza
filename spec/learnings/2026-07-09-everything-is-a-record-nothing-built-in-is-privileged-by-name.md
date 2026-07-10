# Make the special thing an ordinary value reached by the ordinary mechanism — nothing is privileged by name

*2026-07-09*

**What happened.** Building `rcdzc` turned a single principle into the crate's dominant simplification: the
way to kill scattered name-specialization is to make the special thing an **ordinary value reached by the
ordinary mechanism**, so one lookup rule and one projection rule subsume what would otherwise be N special
cases. The [one-accessor learning][[2026-07-03-one-accessor-modules-are-records]] established the shape for
modules and `.`; authoring the whole compiler showed the same move is the *general* solvent, applied
everywhere the seed had a name heuristic:

- **The prelude is one cached map of named `Hir` nodes.** `unit`, every sum constructor, and every built-in
  module (`Int64`, `Bytes`, `List`, `String`, `Map`, `Set`) are entries in a single
  `HashMap<String, Hir>` built once. Resolve clones it, layers the program's own `(type …)` on top, and
  then does **only** name→(scope, then prelude) lookup — there is *no* `if name == "Bytes"` anywhere, and a
  user binding named `Int64` simply shadows the prelude entry.
- **A built-in module is a record; a built-in operation is a first-class value in it.** `Int64` resolves to
  a record; `Int64.max` is an `Int` field, `Int64.wrapping-add` is an `Intrinsic` value, `Map.empty` is the
  empty-map literal. An intrinsic stays an opaque value through `Hir` and `Mir` and becomes wasm opcodes
  only at `select`, via one id→instruction table. There is no built-in-vs-user split: `(. Int64 op)` is the
  same record projection as `(. myModule f)`.
- **A sum type name is a record of its constructors; a constructor is a value.** `Sign` resolves to a
  record whose fields are `Ctor` values, so `(. Sign Pos)` is ordinary projection. `Option`/`Result` are
  *regular* prelude sums, not a built-in special form. `Ast` is [an ordinary prelude sum
  type][[ast-is-an-ordinary-prelude-sum-type]] — "add it to the prelude, don't specialize."
- **A pattern is an ordinary expression, not a `Pattern` construct.** `(Some x)` in pattern position is the
  same tree as `(Some x)` in constructing position — `Apply(Ctor, [Local])` — with `_` a `Wildcard` leaf; a
  pattern lowers exactly like the equivalent construction. `expect` is "just a function that matches and
  traps," not an `Expect` node.

Underneath, the value universe is likewise minimal: a record is a **key-sorted positional product on the
same heap primitive as a tuple** (fields sorted by name, so type-slot order = heap-slot order = render
order), one `Mir::Proj` reads both, and a sum is the one non-product primitive — a `(discriminant,
payload)` pair whose payload is itself a product. Records, tuples, modules, sum-type namespaces, and
built-in modules are therefore all *records*; tuple/record/sum/list/map/set all share the runtime heap
operations, and the tag-free runtime holds no names — the compiler bakes variant and field names from the
static type into a type-directed renderer at the boundary.

One distinction is load-bearing and easy to get backwards: **a not-yet-realized field on a *prelude* module
DECLINES, while an absent field on a *user* record REJECTS.** A prelude module is *open* (an operation the
compiler has not implemented yet is a later-phase method, not an absent field — `Bytes.compact` before it
existed must decline), whereas a user record has a *closed, statically-known* field set (naming a field it
does not have is `CDZ0201`). The same open/closed rule applies to a not-a-variant field on a sum type name.
Getting this wrong is not cosmetic: rejecting the open case once turned 26 not-yet-supported constructs
into false test failures.

**Why.** Every name heuristic the seed carried — lowercase-base means field / uppercase-base means
qualified name, `if name == "Bytes"`, a `qualified_const` path for `Int64.max`, a `Pattern` variant
separate from expressions, an `Expect` node, a built-in-sum path distinct from user sums — was a place two
code paths had to agree about the same thing, which is the same disagree-and-miscompile failure class the
[coarse-kind classifier][[2026-07-08-a-coarse-kind-classifier-re-derived-at-emit-is-the-wrong-inference-and-fails-one-way-at-every-lattice-point]]
and the [fused emitter][[2026-07-06-lower-through-a-resolved-ir-so-emission-is-a-serializer]] embodied at
other layers. Collapsing the special case into "an ordinary value in the one map, projected by the one
rule" removes the second path entirely, so there is nothing to keep in sync. It is the *same* move as
types-are-values ([[2026-07-04-generics-are-type-valued-parameters]]) and constructors-are-values: don't
teach the compiler to *recognize a name*, teach it to *resolve a value*. This is why the sums increment was
allowed only after records existed — "stop hard-coding these access patterns immediately" — because a sum
built on records is "discriminant plus payload record," not a fresh bespoke access path, and the previous
iteration's brittleness came from doing sums *before* the general record machinery existed. The open/closed
decline-vs-reject rule is the reject-don't-miscompile discipline applied to the prelude's own incremental
build-out: an unimplemented prelude operation is a *capability the compiler lacks* (decline), not a
*program that is ill-formed* (reject), and only a decline lets the compiler grow the operation later
without having lied about the program.

**The requirement it drove.** Realizes `core-semantics.md` §"A Module Evaluates To A Record Of Its
Exports," §"A Module Binds Its Name In Its Enclosing Scope," §"A Built-In Module Is A Record Of Its
Operations" (a built-in module is a record indistinguishable in form from a user module, accessed by the
identical mechanism; the language MUST NOT recognize a built-in name in any position a user name would not
be), §"Member Access Projects A Record Field," §"A Sum Type Constructor Is A Single-Arity Function," and
`type-system.md` §"The Abstract Syntax Tree Is An Ordinary Sum Type" and §"Types Are First-Class Values."
The reproduction content **not yet folded**: (1) the *unifying meta-principle* stated once — a construct
that reads like a recognized name (module, built-in, type, constructor, pattern, `expect`) is instead an
ordinary value reached by the ordinary lookup-and-project mechanism, and the resolver recognizes *values in
one map*, never names in special positions; and (2) the *open-prelude-declines vs closed-record-rejects*
rule, which pins that a prelude module's unrealized field is a decline (later-phase capability) while a
user record's absent field is a `CDZ0201` rejection — the reject-don't-miscompile boundary for the
prelude's own incremental realization. That records/tuples/sums share one positional-product runtime
primitive is implementation, not language semantics, and stays in the architecture doc.
