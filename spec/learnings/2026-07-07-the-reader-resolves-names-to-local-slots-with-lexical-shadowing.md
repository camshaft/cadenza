# The reader resolves names to local slots — lexical shadowing is deepest-position-wins in an index environment

*2026-07-07*

**What happened.** With multi-argument calls unblocked, the spike grew the reader's **scope
resolution**: a bare name in a function body (the `x` in `(* x 2)`) is resolved to a **local slot** by
carrying a *parameter environment* — the prelude indices of the in-scope names, in binding order — and
mapping a name reference's prelude index to its position in that environment. Two pieces landed:
- **Name references decode as CBOR tags.** A bare name encodes as tag 39 (`d8 27 <idx>`) wrapping the
  name's prelude index; `read-node` now takes an `env` argument and, on a tag, resolves the index to
  `NLocal <position>` (or a placeholder for an unbound name).
- **`let` extends the environment.** `read-let` reads a `let`'s bindings and appends each bound name to
  the env (`ienv-snoc` — at the end, so its slot is the env's old length), so the body resolves the new
  names to fresh slots past the parameters.

The load-bearing detail is how the environment search handles **shadowing**: `ienv-pos` searches
**deepest-first and returns the last (highest) matching position**. For an environment `[5, 7, 5]`
(name 5 bound at slot 0, then shadowed at slot 2), looking up 5 yields **2** — the innermost binding —
not 0. Verified in isolation (it is ordinary recursion over an Int cons-list): shadowed name → deeper
slot, non-shadowed name → its only slot, absent name → -1.

**Why.** This is the "resolve names to bindings" step — the front rung's *scope* resolution, the
companion to the "resolve names to codes" *operator* resolution
([[2026-07-07-the-reader-realizes-the-prelude-index-name-resolution-contract]]) — realized on runtime
bytes. The durable point is that **lexical shadowing is deepest-position-wins in an ordered environment,
and a first-match search silently gets it backwards.** A name resolver that returned the *first*
matching binding would resolve a shadowing `let`'s name to the *shadowed outer* slot — a scope bug that
produces a valid component computing the wrong thing (it reads the wrong local), exactly the kind of
silent miscompile the corpus's value-level `let`-shadowing cases exist to forbid, now guarded at the
*resolution* layer the compiler implements. The environment-as-ordered-list with append-at-end and
search-deepest-first is the minimal correct realization: append-at-end assigns each new binding the next
slot (matching how wasm locals are numbered), and search-deepest-first makes the innermost binding win —
the two together give lexical scope with shadowing over a flat slot array, no separate scope-nesting
structure needed. This is also the reader reaching the point where it decodes *functions with
parameters and `let`* (not just closed constant bodies), which is most of the surface a real program —
and the compiler's own source — uses.

**The requirement it drove.** A conformance case in `02-binding-and-control.sexp` — *"resolving a name
in a shadowing environment returns the innermost binding's slot"* — pins the resolution idiom: a
deepest-first environment search over `[5, 7, 5]` looking up 5 returns 2 (the shadowing binding), not 0.
It is deliberately the *compiler-internal* resolution step (a name environment as an index list, the
`bytes → local-slot` mapping a reader performs), distinct from the value-level `let`-shadowing cases
above it (which pin the observable that shadowing works); this pins *how a name resolver realizes it*,
and that a first-match search would return the wrong slot. It **PASSES** (ordinary recursion, testable
today). No new backlog item — this is subset-growth realizing lexical scope on the read path, an
already-specified semantics (core-semantics.md §Shadowing Is Well-Defined) now witnessed at the
resolution layer. The reader now decodes parameters, `let`, and name references with correct shadowing;
the standing frontier remains the compiler *emitting* its own richer constructs (sum types / `match`)
and scale (TCO).
