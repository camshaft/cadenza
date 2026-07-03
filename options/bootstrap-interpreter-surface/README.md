# Decision — Bootstrap Interpreter Surface

**The decision.** The concrete set of language constructs, reflection primitives, reader/printer, and
derived-component packaging that a generation must provide so the reference interpreter can be authored
*in Cadenza* — realizing `spec/capabilities/bootstrap-interpreter.md` (which states the invariants
technology-neutrally) and the interpreter-authoring requirement of
`spec/capabilities/self-hosting-and-bootstrap.md`.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- The interpreter is a pure function from a program's canonical representation to its observable
  behavior, needing no host capability to evaluate (bootstrap-interpreter.md §"The Interpreter Is A
  Pure Function To Observable Behavior").
- A program's syntax tree is reachable as an ordinary value; the language provides the constructs to
  walk it (bootstrap-interpreter.md §"The Abstract Syntax Is Reachable As A Value").
- A reader and a printer round-trip: `read(print(v))` equals `v` (bootstrap-interpreter.md §"Text Is A
  Projection Reached By A Reader And A Printer").
- The component boundary carries bytes, not interpreter values; a derived component interprets inside
  itself and emits events only through the manifest's capabilities (bootstrap-interpreter.md §"The
  Component Boundary Carries Bytes, Not Interpreter Values").

This decision is deferred past the seed: the seed is the foreign-language interpreter; a later
generation realizes this surface (`options/realized-capability-set/seed-ignition-set.md`).

## Choices

- [`minimal-reflective-surface`](./minimal-reflective-surface.md) — the smallest surface that lets a
  dynamic tree-walking interpreter be authored in Cadenza: sum types with payloads, mutual recursion,
  structural equality, a small string/list primitive set, `Node`/`Value`/`Behavior` as ordinary
  values, a re-readable reader/printer pair, and a bytes-boundary packaging that embeds the
  interpreter over an AST. **The default.**

DEFAULT: minimal-reflective-surface
