# Decision — Module Pragmas

**The decision.** The surface of a module's compiler directives — the `(pragma …)` channel through
which a module tells the compiler how to compile it — and the **pinned set of pragma keys** that
channel admits. The modules capability fixes the *behavior* — a directive is drawn from a fixed set, an
unrecognized directive is rejected rather than ignored, a meaning-changing directive is part of the
canonical form, and a directive is compile-time only (modules-and-namespaces.md §"Module Directives").
It does not fix the *surface* — the concrete form a directive is written in, and which keys exist. That
surface, and the registry of keys, is the choice this decision pins.

**Why the language wants it.** Some compilation choices are naturally module-scoped rather than
per-expression: the integer type bare literals default to (numeric-model.md), and — as the language
grows — other codegen and lint settings. Threading each such choice through every expression is
friction; a single per-module declaration is the ergonomic answer. But an open-ended directive channel
is a classic source of drift: C's `#pragma` is *advisory* — a compiler ignores a pragma it does not
understand — which means source carrying a meaning-changing pragma compiles to **two different
meanings** on two toolchains. That is precisely the drift this language's one-executable-semantics and
canonical-form principles (constitution §IX, §X) exist to prevent. So the channel is deliberately
**strict**: extensible by a governed act, but never permissive at compile time.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- A directive's key MUST be drawn from a set fixed by the specification, not invented per program, so a
  directive has one fixed meaning across generations (modules-and-namespaces.md §"A Module Directive Is
  Drawn From A Fixed Set").
- An **unrecognized** directive key MUST be **rejected** at compile time, not ignored, so a directive
  can neither silently change a program's meaning on a toolchain that understands it while being
  dropped by one that does not, nor silently fail to take effect (modules-and-namespaces.md §"An
  Unrecognized Module Directive Is Rejected"). **This is the load-bearing rule** — the whole reason the
  channel is not C's `#pragma`.
- A directive whose key is recognized but whose **arguments do not match the shape that key defines**
  MUST be rejected with a machine-readable diagnostic (modules-and-namespaces.md §"A Module Directive Is
  Drawn From A Fixed Set", 2nd sentence).
- A **meaning-changing** directive MUST be carried in the module's canonical form, so the module's
  meaning is determined by its canonical form alone (modules-and-namespaces.md §"A Meaning-Changing
  Directive Is Part Of The Canonical Form"; constitution §X).
- A directive MUST be **compile-time only** — resolved during compilation, introducing no runtime
  representation and crossing no boundary (modules-and-namespaces.md §"A Module Directive Is Compile-Time
  Only").

**Why this is an isolated decision.** A pragma is a compile-time instruction resolved before types
erase; it adds no value form, no trap, and no boundary type. It reuses the language's existing
"pinned registry, unknown entry rejected" discipline — the same shape as the diagnostic-code registry
(`options/diagnostics-schema/`) and the trap-reason registry — so adding a pragma key is a governed
spec act, not an ad-hoc escape hatch. Adding the channel needs two new diagnostic codes (`CDZ0601`
unrecognized key, `CDZ0602` malformed arguments) and no new trap. It touches no frozen contract: the
directive never survives into the component.

## Choices

- [`keyed-registry-strict`](./keyed-registry-strict.md) — a directive is written `(pragma <key>
  <arg>…)` at the top of a module; `<key>` is a bare identifier drawn from a **pinned registry** of
  keys, each fixing its argument shape and meaning; an unrecognized key is `CDZ0601` and a malformed
  argument list is `CDZ0602`; a meaning-changing pragma is part of the canonical form and every pragma
  is compile-time only. The registry's initial key is `default-integer` (numeric-model.md). **The
  default.**

DEFAULT: keyed-registry-strict
