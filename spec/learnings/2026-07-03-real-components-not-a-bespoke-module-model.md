# Use real WebAssembly components for the bootstrap, not a bespoke core-module model

*2026-07-03*

**What happened.** The first cut of the seed's interpreted derivation compiled the reference
interpreter to a bare `wasm32-unknown-unknown` **core module** with a hand-reserved "AST slot" (a fixed
`static` byte buffer plus a length cell), and derived a program by splicing its binary AST into that
slot and hand-trimming the module's import table to the program's manifest. This worked mechanically —
the interpreter ran, imported `emit_event`, and re-derived byte-identically — but it was **inventing a
parallel model** of what a runnable artifact is, in a project whose end goal is precisely the
WebAssembly **Component Model** (`options/execution-model/wasm-component-model.md`). Two concrete
problems exposed it: (1) removing a wasm import to trim to the manifest renumbers every function and
`call` in the module; (2) worse, trimming *per program* makes two programs of different capability
classes produce different **code** sections, which breaks the ignition anti-modeling invariant that
"the same interpreter is reused across programs, differing only in the embedded AST"
([decouple the interpreter-wasm from the host](./2026-07-02-decouple-interpreter-wasm-from-host.md)).
That dead end drove a short-lived constitutional amendment (0.3.0) letting the interpreter carry a
fixed dispatch import set with capability-safety "relocated" to compile-time-plus-runtime mediation —
an amendment that existed **only** to prop up the bespoke model.

**Why.** Real components already solve every problem the bespoke model was working around, because a
component declares its imports and exports in a **WIT world**: a program that grants `emit-event`
yields a component whose world imports `emit-event`; a program that grants nothing yields a world with
no import. So "imports mirror the manifest exactly" (host-interface-binding.md) holds **natively**, with
no per-program import surgery and no code-section divergence — every derived component is byte-identical
except its embedded AST, the *strongest* form of the anti-modeling guarantee. Because real components
make capability-safety hold through the ordinary import mechanism, the 0.3.0 amendment became
unnecessary and was **reverted** (constitution back to 0.2.0 on Core Principle IV; the frozen
host-interface-binding contract restored to the single "Imports Mirror The Manifest Exactly" rule). The
lesson generalizes the standing one that meaning must not drift into a bespoke stand-in: do not invent a
private runnable-artifact model for the bootstrap when the target model is a first-class, tooling-backed
artifact.

**The de-risk spike (recorded so it is not re-run).** A throwaway spike under
`implementation/spikes/component-check/` (gitignored) proved the real-component path end to end,
offline, in this environment. Its findings:

- **A `wit-bindgen` guest → `wasm-tools component new` → wasmtime component host works offline.** The
  wasmtime component host (`wasmtime::component::{Component, Linker}`, features
  `runtime,cranelift,component-model`) instantiated the component, bound the `emit-event` import at the
  world root via `Linker::root().func_wrap`, called the `run` export, observed the emitted event, and
  returned status 0.
- **Build to `wasm32-unknown-unknown`, NOT `wasm32-wasip2`.** Building the `wit-bindgen` guest to
  `wasm32-wasip2` emits a component directly but drags in a pile of `wasi:io/*` and `wasi:cli/*`
  imports (because `std` on wasip2 links WASI), which would then need binding/stubbing and muddy the
  capability story. Building to `wasm32-unknown-unknown` yields a **core module importing only the WIT
  world's own imports and zero WASI**; `wasm-tools component new <core>.wasm` then wraps it into a real
  component whose world is exactly `import emit-event; export run`.
- **`wit-bindgen` needs `std`.** A `#![no_std]` guest fails with a panic-handler lang-item conflict
  against wit-bindgen's `std` dependency. On `wasm32-unknown-unknown`, `std` adds no host imports, so
  using `std` costs nothing on the capability axis — the core module still imports only the world's
  operations.
- **`wasm-tools component new` is byte-reproducible.** Wrapping the same core module twice produced a
  byte-identical component (one distinct SHA-256), so reproducible derivation
  (reproducible-derivation.md) survives the wrapping step for free.

**The requirement it drove.** No new normative requirement — the frozen contracts already require a
content-addressed component whose imports mirror the manifest, and real components satisfy that
directly. It **reverted** a requirement change: constitution Amendment 0.3.0 and the paired
host-interface-binding edit were undone, restoring the single "Imports Mirror The Manifest Exactly"
rule. The concrete realization is recorded in the declared defaults:
`options/execution-model/wasm-component-model.md` §"Interpreted derivation produces a real component
whose world matches the manifest", `options/bootstrap-strategy/rust-seed-interpreted-first.md`
(interpreter artifact = core module wrapped by `wasm-tools component new`), and
`options/bootstrap-interpreter-surface/minimal-reflective-surface.md` §"Packaging". The spike code is
throwaway and gitignored; these findings are the durable record so a later build starts from proven
ground.
