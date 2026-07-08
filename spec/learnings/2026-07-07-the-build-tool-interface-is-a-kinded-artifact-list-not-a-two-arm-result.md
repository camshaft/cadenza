# The build-tool interface is a kinded-artifact list, not a two-arm result

*2026-07-07. Rationale for Amendment 0.8.0 (constitution) and the reshaping of the frozen
build-tool-interface contract's derivation entry.*

## What changed

The build tool's derivation entry was pinned as a two-arm result:

```
compile: func(ast: list<u8>) -> result<list<u8>, list<diagnostic>>
```

It is now a **kinded-artifact interface**:

```
compile: func(inputs: list<artifact>) -> compile-output
record compile-output { artifacts: list<artifact>, diagnostics: list<diagnostic> }
record artifact       { kind: string, bytes: list<u8> }
record diagnostic     { severity, code, message }        // severity: error | warning | …
```

The derived component is **one artifact** in the output list, identified by its `kind`. Success is
"a component artifact is present and no diagnostic has error severity"; failure is "no component
artifact and at least one error diagnostic."

## Why the two-arm result was too narrow

Three limits, each a real build-tool need the mutually-exclusive `result<ok, err>` cannot express:

1. **Warnings alongside a module.** `result` is either/or: you get bytes *or* diagnostics, never both.
   A derivation that succeeds *and* wants to report warnings has nowhere to put them — it must discard
   one. Diagnostics belong in a channel that is *always present*, with the error-vs-warning distinction
   carried by a per-diagnostic **severity**, not by which arm of a union was taken.

2. **More than one byte output.** A real toolchain emits more than a component: DWARF/debug information,
   a source map, the capability manifest, a type-info sidecar. `result<list<u8>, …>` hard-codes a
   *single* output blob. A list of kinded artifacts makes every byte output the same shape, and the set
   of output kinds is **open** — a new kind is an additive change, not a new return type.

3. **More than one input.** A single `list<u8>` AST assumes one source unit and no side inputs. A
   multi-unit program with imports needs several source artifacts; an incremental build wants to pass a
   **cache**; a separately-derived dependency is another input. Making the input a `list<artifact>` too
   admits all of these without changing the entry's arity — the canonical source tree is just the
   artifact of kind `ast`/`source`.

The unifying insight: **compilation is artifacts-in, artifacts-out, with a diagnostics channel that is
always live.** The component is not a privileged return value; it is the artifact of kind `component`.
Both directions become one open, extensible list, and severity-on-diagnostics replaces the ok/err split.

## Why symmetric (a list on *both* sides)

Once the output is a kinded-artifact list, making the input the same shape is the natural symmetry: the
same `artifact { kind, bytes }` record describes a source unit, a cache, a dependency, DWARF, a source
map. One vocabulary for everything that crosses the tool boundary, selected by `kind` rather than by
position or by a fixed arm. A kind the tool does not recognize is a diagnostic, not a silent drop —
consistent with reject-don't-miscompile.

## Governance

This is a **non-additive** change to a frozen contract (the derivation entry's signature changes), so
under the Governance Floor "A change to the component ABI that alters the bytes produced from unchanged
source MUST carry a version increment" it carries an ABI version increment (Amendment 0.8.0). It does
not touch a never-downgradable floor: determinism and capability-safety are unaffected (the entry is the
tool's *interface*; no already-derived program's emitted bytes change).

**Migration path.** The realized seed interface stays `compile: list<u8> → result<list<u8>,
list<diagnostic>>` — the degenerate single-input / single-output case of the general shape (input = one
`ast` artifact; success = the `component` artifact with no error diagnostics; failure = the diagnostics
list) — until the artifact-list interface is realized end-to-end (seed envelope + wrapper + the
Cadenza-authored compiler's entry). A consumer of the old signature reads exactly the
component-or-diagnostics outcome the new record expresses, so the transition is mechanical.

## What is already realized (the stepping stone)

The seed already emits the diagnostics half of this — a `(def (compile b) …)` whose body returns a
`Result<Bytes, list<diagnostic>>` is emitted as `compile: list<u8> → result<list<u8>, list<diagnostic>>`,
so the Cadenza-authored compiler can return a coded rejection instead of trapping (the immediate
self-hosting unblock: ill-typed programs reach byte-parity with native's `CDZ####`). The full
artifact-list interface — a `list<artifact>` on both sides, the component as one artifact, warnings
alongside a module, DWARF/source-map/manifest as further artifacts — is the target this amendment pins;
its realization is tracked as an implementation ask. The reshaping is done at the contract level first
(this amendment) so the interface is designed before it is built out, not retrofitted after the compiler
depends on the narrower shape.
