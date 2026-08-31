# Cadenza — the interactive guide

A browser-based tour of the Cadenza language where **every example compiles and runs in the browser**,
and the reader can **switch the display syntax** (conventional ML/Rust surface ↔ homoiconic
s-expression surface) globally.

There is no backend. The Cadenza compiler (`rcdzc`) and front-end (`cadenza-syntax`) are compiled to
WebAssembly (`cdz-wasm`) and run in a Web Worker; emitted WebAssembly **components** are transpiled to
runnable ES modules in-browser with [`jco`](https://github.com/bytecodealliance/jco), composed with the
value-heap runtime when needed, executed, and their results rendered — the same pipeline the native
`cdz-run` reference runner uses, reimplemented on browser APIs.

## Architecture

```
text ──▶ [cadenza-syntax] ──▶ binary AST ──▶ [rcdzc] ──▶ wasm component
                                                              │
   compile worker (Comlink, off the UI thread) ──────────────┘
                                                              │
   run worker (disposable, killable) ──▶ [jco transpile] ──▶ ES module + core wasm
       compose value-heap runtime ──▶ instantiate ──▶ make()/encode() ──▶ render value
```

- **`src/compiler/`** — `cdz-wasm` loaded in a long-lived Comlink worker: `compile`, `renderSyntax`
  (the syntax toggle), `renderValue`.
- **`src/runner/`** — a *disposable* worker that jco-transpiles a compiled component, composes the
  runtime, and runs it. A main-thread watchdog `terminate()`s it on a timeout (infinite-loop guard)
  and spawns a fresh one. `node-stubs.ts` / `oxc-minify-stub.ts` resolve jco-transpile's Node-only
  imports for the browser bundle (those code paths never execute — no minify/optimize/asm.js).
- **`src/syntax/`** — the global surface mode (React Context + header segmented control), persisted to
  `localStorage` + the `?syntax=` URL param.
- **`src/editor/`** — CodeMirror 6 with a Cadenza StreamLanguage tokenizer.
- **`src/content/`** — the tour chapters (TSX embedding `<Runnable>`), registered in `chapters.ts`.

## Authoring conventions

- **Show the concrete VALUE — never hide it behind a length/count/size.** Every example must render the
  actual value a program produces (the tangible thing), not a `List.len` / `Set.len` / `Map.len` /
  `Bytes.len` / `scalar-len` count standing in for it. If a `match` binds a residual collection, return
  the collection — `(Some rest)` yielding `Option(Set(Int64))` shown as `(Some #set(2 3))` — not
  `(Set.len rest)` yielding `2`. `(Some #set(2 3))` is far more tangible to a reader than `2`.
  **Exception:** an example whose *lesson is a length operation itself* (the Lists chapter documenting
  `List.len`, `scalar-len` vs `byte-len`, a counting algorithm) legitimately shows the length — there the
  length is the subject, not a stand-in for a hidden value. **This is an operator directive, repeated: it
  must hold EVERYWHERE, in every new and regenerated example.**

## Develop

Requires **Node ≥ 20.19** (jco's transpiler needs it) and the Rust `wasm32-unknown-unknown` target +
`wasm-pack`.

```sh
# 1. Build the compiler wasm and stage it (+ the value-heap runtime) into src/wasm/.
#    Build the runtime first (once) so the store holds it: `cargo xtask build` at the repo root.
npm run wasm

# 2. Dev server.
npm run dev

# 3. Production build.
npm run build
```

`npm run wasm` runs `wasm-pack build --target web` on `../implementation/seed/crates/cdz-wasm` and
`scripts/stage-wasm.mjs`, which copies the `pkg/` and finds the value-heap runtime whose SHA-256
matches the hash the compiler pins (read straight from the compiler wasm — no hard-coded hash).

## Notes

- **Scalar/unit programs** are self-contained components — they run without the runtime. **Compound
  results** (tuples/records/sums) import the value-heap runtime, which the run worker composes in.
- Snippets are authored once (s-expression by default) and re-serialized to the active surface via
  `renderSyntax` — a single source of truth, since every surface is a lossless projection of one AST.
