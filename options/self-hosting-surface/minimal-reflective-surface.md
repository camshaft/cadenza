# Self-Hosting Surface — Choice: minimal-reflective-surface

> **The default choice for the `self-hosting-surface` decision** (see [README.md](./README.md)
> for the decision and the requirements a choice must satisfy). It pins the smallest concrete surface
> that lets a compiler be authored in Cadenza — and, equivalently, the surface the *optional*
> Cadenza-authored reference interpreter (a `MAY` independent oracle) would walk. The bootstrap path
> itself is compiler-first — two compilers, not an interpreter (see
> `spec/learnings/2026-07-04-two-compilers-not-an-interpreter-and-a-compiler.md`); the interpreter
> illustration below is retained only because it exercises the same reflective surface a compiler needs.

## The insight: a meta-circular interpreter reuses the language, it does not reimplement it

When the Cadenza-authored interpreter evaluates `(+ 2 3)`, it evaluates the operands and then applies
the language's *own* `+`. So the interpreter needs **no new arithmetic**; it needs only what is
required to **represent and walk** a program and its values, and to **delegate** to the operators the
generation already realizes. That keeps the added surface tiny.

## What the language must provide (the surface `eval` is written against)

| Concern | Concrete requirement | Why the interpreter needs it |
|---|---|---|
| **Sum types with payloads** | declare a sum type whose variants carry data, construct a variant, and deconstruct it by `match` | `Node`, `Value`, `Terminal` are sum types; matching on them is the core of `eval` |
| **Records** | construct a record and read a field | `Behavior`, `Event`, and environment frames are records |
| **Mutual recursion** | top-level functions that call each other, bounded by the resource measure | `eval` ↔ `eval-args` ↔ `eval-match` recur mutually |
| **Structural equality** | `=` over values | evaluating the language's own `=`, and the reader/printer round-trip oracle |
| **Lists** | construct, head, tail, length, index (total-or-trap) | argument lists, environments as assoc-lists, event sequences |
| **Strings** | equality and concatenation | dispatch on a node's head-symbol name; build diagnostic and qualified-name text |
| **Integers/bools** | the arithmetic and comparison the seed already realizes | delegated to when interpreting `+`, `<`, `if` |
| **First-class functions/closures** | `fn` values that capture their scope, applied by `(f arg…)` | authoring a **compiler** in Cadenza needs first-class functions — passing, returning, and storing them (core-semantics.md §Functions); the seed realizes them |

The seed realizes first-class functions and closures because the first Cadenza artifact is a
**compiler**, not merely a meta-circular interpreter, and a compiler is not expressible without them
(higher-order passes, environments of closures, continuation-passing). This is a change from an
interpreter-only rung, which could have gotten by with explicit top-level recursion alone.

## Reflection: the AST is an ordinary value

- `Node` is a Cadenza sum type — code *is* data (homoiconic; `options/code-shape/`):
  `Node = Int(Int64) | Float(Float64) | Str(String) | Bool(Bool) | App(Symbol, List(Node))`, with
  `Symbol = record { namespace: String, name: String }`.
- The interpreter obtains a `Node` to walk **without touching bytes**: the derived component's glue
  decodes the embedded binary-AST bytes into a `Node` value and hands it to `eval`. `eval` is pure
  `Node -> Behavior` and never sees bytes. (When byte primitives mature, the decoder itself migrates
  into Cadenza; not needed to bootstrap.)

## Behavior is data; capabilities enter only at the boundary

```
Behavior  = record { terminal: Terminal, host-calls: List(HostCall) }
Terminal  = Normal(Value) | Trap(String) | Exhausted
HostCall  = record { fn: Symbol, args: List(Value) }
```

- `eval` **returns** `Behavior` as data; it does **not** call any host function itself. So `eval` is a
  pure function and the semantics suite can be run **through it with zero host capabilities** — the
  cheapest possible proof that the interpreter works (self-hosting-and-bootstrap.md §"Both Compilers
  Are Proven Against The Corpus Before They Are Relied On"). At the seed the compiler proof runs
  natively; the boundary shim below applies only where a generation offers the optional interpreted
  derivation as a component.
- Capabilities enter only in the **derived component's boundary shim**: `run(input: list<u8>)` decodes
  the embedded AST and drives `eval` under the suspend-replay boundary — at each recorded host call it
  yields to the host, which resolves it and re-invokes, so the host calls are made through the real
  host imports rather than replayed from a transcript computed ahead of time
  (build-tool-interface.md §"A Derived Component Computes Its Behavior When It Runs";
  capabilities-and-effects.md §"Suspension Is Replay From The Host's Log").

## Reader and printer (in the seed first; the round-trip is the first test oracle)

- **Reader** `read: text -> Node` — a dumb re-readable s-expression reader. Lives in the seed (foreign
  language) first; rewritten in Cadenza later. Not in the component's trusted derivation path.
- **Printer** `write: Node -> text` — the re-readable inverse (quoted/escaped strings, enough structure
  to round-trip). Built right after the reader to unlock the oracle:

  ```
  read(write(v)) == v      // for all Node values
  write(read(s)) == s      // for all canonical text
  ```

  This round-trip is the cheapest high-coverage test available before there is a language to write
  tests in; it exercises the value representation, reader, and printer at once.
- **Floats:** print via the host's shortest-round-trippable formatting and parse back, so `f64`
  round-trips exactly; do not hand-roll a float formatter at bootstrap.
- A human-oriented **`display`** (unquoted, lossy) is **deferred** — build only the re-readable `write`
  and let humans read that too.

## The component boundary is `bytes -> bytes` (no recursive values in WIT)

- The entry is `run(request: list<u8>) -> result<list<u8>, trap>` (`options/execution-model/`), so no
  recursive `Node`/`Value` ever crosses the boundary — values live entirely inside the component. This
  is the same rule as "interpret, not replay" wearing a second hat: keeping values inside is exactly
  what makes the boundary bytes-only.

## Packaging: a real component whose WIT world matches the manifest

- Interpreted derivation produces one content-addressed **real WebAssembly component**: the interpreter
  is compiled to a core module (`wasm32-unknown-unknown`, so its only imports are host capabilities and
  never WASI) and wrapped by `wasm-tools component new` into a component whose **WIT world declares
  exactly the program's granted capabilities**, with the program's AST embedded as component data.
  Because the world *is* the import set, "imports mirror the manifest exactly"
  (host-interface-binding.md) holds natively — a program that grants a host function yields a world
  importing that function; a program that grants nothing yields a world with no import — with no
  per-program import surgery on the core module.
- The **same interpreter** is reused across all derived programs; only the embedded AST and the world's
  import set (which matches each program's manifest) differ — the distinguishing check that behavior
  comes from the embedded program, not from derivation-emitted per-program logic
  (build-tool-interface.md §"A Derived Component Computes Its Behavior When It Runs"). A program that
  grants a host function yields a world importing it; a program that grants nothing yields a world with
  no import.

## The tower and fuel

Because the seed (native) runs the Cadenza-interp which interprets a program, the interpretation tower
is deeper than a single interpreter. The seed's evaluator MUST bound depth by the resource measure
(fuel) so a deep or non-terminating tower halts at a defined point rather than overflowing the host
stack (determinism-and-fuel.md §"Resource Accounting"); a mature generation trampolines its evaluator
rather than relying on host recursion.

## Why this is the minimal surface

- **Reuse over reimplement:** no new arithmetic; the interpreter delegates to the language's operators.
- **Pure core:** `eval: Node -> Behavior` needs no capabilities, so Milestone 1 (prove the interpreter
  by running the whole suite through it) is capability-free.
- **One layer:** the interpreter is written directly over the seed; no interpreter-on-interpreter tower
  is required to be authored.
- **Deferred everything else:** a Cadenza-authored reader/printer, human display, and the
  interpreter's own byte-decoding of an embedded AST (the interpreted-derivation glue) all wait until
  the flywheel is turning. (Closures are **not** deferred — the seed realizes first-class functions
  because the first Cadenza artifact is a compiler; see the surface table above. And the `Bytes` value
  form the seed realizes for the **compiler's output** — options/realized-capability-set/ — is a
  distinct concern from this interpreter surface: it lets the Cadenza-authored compiler build a
  component's wasm bytes as a value, whereas `eval` here still never touches bytes.)
