# Decision — Self-Hosting Surface

**The decision.** The concrete set of language constructs, reflection primitives, reader/printer, and
compiled-component packaging that realize the self-hosting surface — what a Cadenza-authored **compiler**
needs to walk a program as data and emit a component — stated technology-neutrally by
`spec/capabilities/self-hosting-surface.md`. The same surface is what an *optional* reference
interpreter, if a generation chooses to author one, would also walk (a `MAY` independent oracle, never
a step on the bootstrap path — bootstrap.md §"A Reference Interpreter Is An Optional Independent
Oracle").

**Requirements any choice must satisfy (from the spec — do not weaken):**
- A program's syntax tree is reachable as an ordinary value; the language provides the constructs to
  walk it (self-hosting-surface.md §"The Abstract Syntax Is Reachable As A Value").
- A reader and a printer round-trip: `read(print(v))` equals `v` (self-hosting-surface.md §"Text Is A
  Projection Reached By A Reader And A Printer").
- A run's observable behavior is a value whose canonical byte form is that behavior, so two compiler
  implementations agree exactly when they produce equal behavior (self-hosting-surface.md §"Observable
  Behavior Is A Value").
- The component boundary carries bytes, not a toolchain's internal values; a derived component computes
  its behavior inside itself and makes host calls only through the manifest's capabilities
  (self-hosting-surface.md §"The Component Boundary Carries Bytes, Not Internal Values").

## Choices

- [`minimal-reflective-surface`](./minimal-reflective-surface.md) — the smallest surface that lets a
  compiler (and, optionally, a tree-walking reference interpreter) be authored in Cadenza: sum types
  with payloads, mutual recursion, structural equality, a small string/list primitive set,
  `Node`/`Value`/`Behavior` as ordinary values, a re-readable reader/printer pair, and a bytes-boundary
  packaging.  **The default.**

DEFAULT: minimal-reflective-surface
