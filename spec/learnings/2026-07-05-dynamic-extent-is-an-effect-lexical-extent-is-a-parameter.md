# Dynamic-extent context is an effect; lexical-extent data is a parameter

*2026-07-05*

**What happened.** The compiler-in-Cadenza spike
([[2026-07-05-authoring-the-compiler-in-cadenza-surfaces-the-language-gaps]]) began under an operator
directive to model *all* of a compiler's ambient state as algebraic effects — "effects for everything" —
including the lexical environment (the symbol table), on the reasoning that effects exist precisely to
kill state-threading and just express a contract. Writing it out showed the environment is the one piece
that does **not** belong as an effect, and the exercise produced a sharper rule that keeps the
"effects everywhere" ergonomics everywhere it actually helps:

- **The environment is a threaded immutable map, not an effect.** Type-checking a sub-expression under an
  extended scope is `(check (extend env binds) body)`: the callee receives the larger map, the caller
  keeps its own, and the binding is gone the instant the sub-walk returns — lexical scoping expressed
  *structurally*, with zero bookkeeping. Modeling the environment as a `State` effect instead forced a
  `scoped` combinator doing manual `snapshot`/`restore`, and a handler threading the env as handler-local
  state across resumptions — hand-reimplementing exactly the save/restore that argument-passing gives
  away.

- **Genuinely ambient context stays an effect.** Diagnostic accumulation, the fresh-name supply, and the
  unification store remain algebraic effects. Each is alive from some point *until a handler returns* and
  is reached from arbitrarily deep in the walk; threading them is pure ceremony that buries the logic,
  and the "record a diagnostic and continue" handler (resume with unit) is the model at its best — no
  threaded error list, no early return.

**Why.** The distinguishing property is **extent**:

- **Lexical extent** — data visible in a *region of the syntax tree* and invisible outside it. This is
  what argument-passing already models: passing a value down a recursion makes it visible to the subtree
  and nothing else. An environment has lexical extent by definition, so a parameter is not a workaround
  for it — a parameter *is* it.
- **Dynamic extent** — context alive from a point in *execution* until a dynamically-enclosing boundary
  (the handler) returns, reachable by anything in between. Diagnostics, fresh supply, and the unification
  store have dynamic extent, which is exactly what an algebraic handler scopes.

So the tell that an effect is the wrong tool is a handler that has to `snapshot`/`restore` to fake
lexical nesting: `State` models dynamic extent, and using it for lexically-scoped data means
re-deriving scope by hand. This does not retreat from the language's uniformity tenet
([[2026-07-03-uniform-single-arity-constructors]]) — it states it more precisely. The rule
"dynamic-extent context → effect, lexical-extent data → parameter" is uniform; the environment simply
lands on the parameter side of it, and it is the *only* thing that does. It also refines the general
resolution that mutation re-enters as a pure state-passing `State` effect
([[2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects]]): `State` is right for genuinely
mutable dynamic-extent cells, but a lexical environment is not mutable state — it is a value that grows
as you descend and shrinks as you ascend, which is what an immutable map threaded through a recursion
already is. (A `Reader`-with-`local` effect *would* honor lexical extent, unlike raw `State`, but it
still wraps every sub-scope in `local` and hides that the walker depends on scope — for the environment
specifically, the plain parameter wins on both counts.)

**The requirement it drove.** This is a design principle for how a Cadenza program *structures* its
state, not a new capability requirement, so it added no RFC-2119 sentence. It shaped the compiler spike's
state model (the environment threaded, `Diagnostics`/`Fresh`/`Unify` declared as effects per
[[2026-07-05-effects-are-declared-with-one-surface-the-declaration-is-the-grant]]) and is the reasoning
future Cadenza-authored programs should apply when deciding whether a piece of context is an effect or a
parameter. It leaves the effect *mechanism* untouched: nothing here weakens algebraic effects; it draws
the line for when to reach for one.
