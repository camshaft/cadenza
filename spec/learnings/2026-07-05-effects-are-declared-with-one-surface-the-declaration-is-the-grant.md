# Effects are declared with one surface, and a host-bound declaration is the grant

*2026-07-05*

> **SUPERSEDED IN PART (2026-07-06).** The "one declaration surface" and "performed/handled as
> `<Name>.<op>`" findings still hold. But **"a host-bound declaration is the grant" no longer does**:
> the `(host)` marker was removed from the effect *declaration* (which is now a routing-agnostic
> contract), and host-binding became an **entrypoint routing decision** — an entrypoint delegates a set
> of effects with a `(host (<effect>…) <body>)` form, the boundary counterpart of `handle`. The grant is
> the *delegation*, not the declaration; the manifest is the union of the entrypoints' delegations; and
> `CDZ0401`/`CDZ0402` merged into one "reached with no handler and no delegation" check (`CDZ0404` added
> for a delegation naming an unreached effect). Reason: wasmtime fibers make host suspension free, so the
> only thing `(host)`-on-declaration bought was coupling an effect's contract to its routing — the exact
> thing effects exist to decouple. See `spec/capabilities/capabilities-and-effects.md`
> §"An Effect Is Routed By A Handler Or By Host Delegation" and the 2026-07-06 memory note on
> per-entrypoint capability routing.

**What happened.** Authoring the compiler in Cadenza
([[2026-07-05-authoring-the-compiler-in-cadenza-surfaces-the-language-gaps]]) reached a wall the spec
had not anticipated: the language had a way to *handle* effects (the `handle` form, witnessed by ad-hoc
`choose`/`get` operations in the corpus) but **no way to *declare* an effect and type its operations**.
A compiler carries several intra-program effects — a fresh-name supply, diagnostic accumulation, a
unification store — and each needs a named, typed operation set before it can be performed or handled.
The declaration surface was designed and landed, and the design forced two further decisions in the name
of the language's uniformity tenet:

- **One declaration surface for every effect.** An effect is declared with
  `(effect <Name> (op <op> (-> <T>… <R>))…)`, which binds `<Name>` in its scope as a **record of
  operations**. An operation is performed and handled as `<Name>.<op>` — reached by the *same* member
  access `.` that every other namespace uses ([[2026-07-03-one-accessor-modules-are-records]]), so an
  operation is qualified by its effect and two effects may each declare a `resolve` without collision.
  Performing an operation type-checks its arguments against the declared parameter types, yields the
  declared result type, and adds the effect to the performing function's inferred row — reusing the row
  machinery already in place ([[2026-07-04-records-are-rows-open-by-default]],
  [[2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects]]).

- **A host import is the same declaration with a `(host)` marker — and the declaration is the grant.**
  A host function is declared `(effect log (op emit (-> String Unit)) (host))`. The `(host)` marker is
  the *only* difference between a boundary effect and an intra-program one: it fixes that the effect is
  discharged at the component boundary by suspend-and-replay
  ([[2026-07-05-host-calls-suspend-as-replay-from-the-hosts-log]]) rather than by a lexical handler, and
  that it is enumerated in the manifest. Critically, **the host-bound declaration *is* the manifest
  grant** — the two prior forms, `(import (host …))` (signature) and `(use (capability …))` (grant),
  were both removed. There is now exactly one way to declare a host function, and no separate
  capability-granting form.

**Why.** The unifications each remove a "several ways to do one thing" that the spec had been carrying:

- **Host imports and intra-program effects were already "one concept" in the spec prose** — a capability
  *is* a boundary effect ([[2026-07-04-effects-are-algebraic-capabilities-are-boundary-effects]]) — but
  were declared by two unrelated syntaxes. Marking host-binding as a *property of the effect's
  discharge*, declared on the effect, makes the prose true of the surface too. The rejected alternatives
  were a parallel `host-effect` keyword (reintroduces two concepts) and "an un-handled effect silently
  becomes a host capability" (unsafe — a forgotten handler would silently grant authority, violating the
  compile-time capability floor).

- **Collapsing `(use (capability …))` into the declaration is what the spec already demanded.**
  capabilities-and-effects.md §"Undeclared Capability Is A Compile-Time Error" requires the compiler to
  *derive* required capabilities from the operations a program reaches, "rather than from a
  separately-asserted list that could understate them." A separate `use` grant *was* exactly such a
  second, assertable list; removing it makes the manifest a pure projection of the host-bound effects the
  program declares and reaches — more internally consistent, not less. `CDZ0401` sharpens to "reaches a
  host-bound operation for an effect the program does not declare."

- **Two rejections keep the floor a compile-time property.** `CDZ0402` rejects an operation performed at
  a point with neither an enclosing handler nor (for a host-bound effect) a manifest entry — so an effect
  can never silently escape undischarged. `CDZ0403` rejects a handler arm naming an operation its effect
  does not declare — so a declaration is the closed set of an effect's operations. "No ambient authority"
  is therefore checked, never inferred from whether a handler happens to be present.

**The requirement it drove.** `spec/capabilities/capabilities-and-effects.md` gained two normative
sections — §"An Effect Is Declared With Its Operations" (declaration, typed-performance, handler-arm
membership) and §"An Effect Is Host-Bound Or Handled, By One Declaration Surface" (the `(host)` marker,
manifest-as-projection, and §"An Undischarged Effect Is A Compile-Time Error"). The concrete surface is
pinned in `options/effects-model/algebraic-one-shot.md`. `options/diagnostics-schema/` registers
`CDZ0402` and `CDZ0403`. `options/code-shape/homoiconic-decoupled-display.md` replaces the `use`/
`capability` core symbols with `effect`/`op`/`host`/`handle` (both display projections updated), and
the glossary and pragma option drop their references to the removed forms. The corpus was migrated: all
host cases across 04/03/11 now use `(effect … (host))` and `<name>.<op>`, and 14-effects gained five new
compiler-idiom cases (a typed operation, record-and-continue, collision-free qualified names, and the
`CDZ0402`/`CDZ0403` rejections). Because the seed's reader parses only the old forms, the migrated
realized-floor cases carry `(needs effects)` so the seed skips them and the behavior gate stays green —
the capability floor 04-capabilities used to realize is deferred until a generation teaches the seed
reader the `(effect … (host))` surface. This composes with, and does not disturb, the frozen
`host-interface-binding.md`, which fixes only the *mechanism* (an import is a WIT-typed host function the
manifest enumerates) and is silent on the declaration syntax
([[2026-07-05-host-functions-are-un-named-the-language-binds-any-wit-function]]).

**Open, for the effects-realization generation.** Two items the migration surfaced but deferred: the
corpus `(host-calls …)` / `(host-responses …)` fixtures now key on a dotted operation name (`log.emit`),
which the reader produces as a `(. log emit)` node the harness parser does not yet accept; and the
mapping from an effect operation to a concrete WIT import name (by effect, by operation, or by
`effect.op`) is undecided. Both belong to wiring host-bound effects to component imports, not to this
declaration-surface change.
