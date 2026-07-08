# A module value definition is dropped, not registered as an export field

*2026-07-08*

**What happened.** Adversarial probing of the module/export surface found that a module's *value*
definition (a `(def v 7)`, name-plus-value with no parameter list) is not registered as a member.
`(do (module m (def v 7)) (. m v))` emits a VALID component that TRAPS at run time instead of
yielding 7. And inside the module, a sibling that references the value def sees "unbound name":
`(module inner (def base 10) (def (add n) (+ n base)))` rejects `base` as unbound. Only *function*
definitions (`(def (f …) …)`) are registered as export fields and as mutually-visible siblings; a
value definition is silently dropped.

**Why it is a break.** The glossary defines a Definition as "a named binding introduced by a module:
a value, function, type, …" — a value is a definition. core-semantics.md #A Module Evaluates To A
Record Of Its Exports: "Each definition MUST register its name and value as a field of the module's
record," and "Evaluating a module MUST produce a record whose fields are the names its definitions
export bound to their values." So `(def v 7)` must register `v` as a field bound to 7, and `(. m v)`
must project it to 7 (directly — the field IS the value, not a nullary function to apply). Instead
the export record omits `v`, so `(. m v)` names a field the record does not contain and traps
(core-semantics.md §Member Access traps on a missing field). But the trap is on a well-typed
projection of a field the spec says MUST be present — a decline-don't-miscompile violation of the
emit-a-broken-component kind: the module compiled to a component whose valid export access traps.

**The scope inconsistency behind it.** The same value-definition form resolves three different ways
by scope:
- `do` block: `(do (def x 5) (+ x 1))` → 6 (works — corpus-pinned; a value def binds for the forms
  that follow).
- module top level: `(module m (def x 5) …)` → REJECTED "def without a signature" (the module
  definition parser accepts only the `(def (name …) body)` function form).
- inner module member: `(module inner (def v 7) …)` → silently DROPPED (not a field, not visible to
  siblings), so access traps.
A value definition is legal in a `do` block, rejected at module top level, and silently dropped as a
module member — three behaviors for one form. Per the glossary and #A Module Evaluates To A Record Of
Its Exports it should be a registered field everywhere a module accepts definitions.

**Root cause (likely).** The module-member collector recognizes only the function-definition shape
`(def (name params…) body)` and either rejects (top level) or ignores (inner) the value shape
`(def name value)`. Since `do`-scoped `def` already handles the value form (it binds `x` for the
following forms), the module collector needs the same value-form handling: register `(def name
value)` as a field/binding, not only `(def (name …) …)`.

**The lesson.** A definition form that one scope supports (a value `def` in a `do` block) must be
supported by every scope that admits definitions, or the same syntax means different things — and the
worst outcome is not the reject (top level, at least honest) but the silent drop (inner module),
which emits a component that traps on a valid access. When adding a new binding shape, audit every
scope that collects definitions, not just the one the feature was written for; the module collector
was written for function exports and never grew the value-export case the glossary's Definition
requires.

**Corpus case added.** `spec/semantics/11-modules.sexp` §"a module value definition registers a
reachable export field" — `(do (module m (def v 7)) (. m v))` MUST yield 7, as the value-definition
companion of the function-export case §"each definition in a module registers a reachable export
field". Native seed; the behavior gate catches it (expected output 7, observed a trap). A generation
that does not yet register value definitions MUST decline rather than emit a component whose export
access traps.
