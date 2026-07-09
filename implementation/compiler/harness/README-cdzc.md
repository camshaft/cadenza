# cdzc harness (`cdzc.py`) — the ONE way to exercise the rewritten compiler

`cdzc.py` is the single, documented harness for the Cadenza-authored compiler `cdzc.cdz`. It replaces the
ad-hoc inline-python probes that had accumulated. **Do not hand-roll runtime linking or bare-`wasmtime`
runs** — the seed binary already compiles, links the runtime, and runs `main`.

## The one runner: `cadenza-seed emit`

Every operation goes through `cadenza-seed emit <program.cdz>`, which:
- compiles the program (and prints `VALID component` / `INVALID: …` / `declined: …`),
- **runs `main`** with the runtime linked (see `seed/crates/cadenza-seed/src/host.rs`), printing
  `ran → Value("…")` or `ran → Trap(…)`.

So one subprocess gives us compile status AND the run result, for scalars, strings, bytes, heap values, and
traps — no `wasmtime`, no runtime-import wiring, no offset math. (Bare `wasmtime` on a heap-importing
component fails with "box-int has the wrong type" — that path is a dead end; don't use it.)

## Operations (library + CLI)

| CLI | function | what it does |
|-----|----------|--------------|
| `cdzc.py self` | `self_compiles()` | does cdzc compile through its real pipeline entry? (drives `compile-bytes` so the whole chain is reachable — a trivial `main` trips a seed whole-module DCE quirk, don't use it) |
| `cdzc.py eval  '<prog>'` | `eval(src)` | run a plain Cadenza program (no cdzc) — seed-gap probes |
| `cdzc.py probe '<expr>'` | `probe(expr)` | run `<expr>` **inside cdzc's module** (inject `(def (main) <expr>)`) — cdzc's own defs (`decode`/`resolve-program`/`lower`/`select`/`serialize`/…) are in scope. Inspect one pipeline stage. |
| `cdzc.py compile '<prog>'` | `compile(prog)` | compile `<prog>` with cdzc end-to-end → `Outcome.kind='bytes'` is the emitted component |
| `cdzc.py run '<prog>'` | `compile_run(prog)` | compile with cdzc, then RUN the emitted component → the program's value/trap (true end-to-end oracle) |
| `cdzc.py oracle` | `run_backend_oracle()` | run the arithmetic BACKEND oracle (hand-built Mir → select/serialize/frame → run): 15 cases, +/-/* value+overflow-trap |
| `cdzc.py astbytes '<prog>'` | `ast_bytes(prog)` | the CBOR AST-bytes hex of a program (via the seed's own `Ast.encode∘quote`) |

### The `<BYTES-OF "…">` macro (in `probe`)

Inside a `probe` expression, `<BYTES-OF "<program>">` is replaced by the literal `(Bytes.of (list 0x.. …))`
of that program's AST bytes — so probes needn't hand-transcribe hex:

```
cdzc.py probe '(match (decode <BYTES-OF "(module c (def (main) 42))">) ((Ast.List xs)(List.len xs))(_ -1))'
```

## Outcome kinds

`value` (rendered scalar/string) · `bytes` (a Bytes value, decoded — cdzc output components arrive here) ·
`trap` (runtime trap: overflow/div0/an internal decline→unreachable) · `decline` (seed refused to compile —
a seed gap) · `invalid` (seed emitted an invalid component — a bug) · `error` (emit failed / no output).

## ⚠ Settledness discipline (READ THIS)

The stable toolchain (`implementation/stable/{cadenza-seed,cdz_runtime.wasm}`) is rebuilt by the compiler
agent concurrently. A seed being rewritten mid-read gives **non-deterministic** results — a probe can flip
between passing and a spurious `decline`/`invalid` on the SAME cdzc source. Before trusting any run:
1. `stat -f '%Sm %z %N' -t '%H:%M:%S' implementation/stable/cadenza-seed` — poll until mtime AND size are
   stable across a few seconds;
2. run the probe **twice** and only trust an identical result (the CLI is cheap);
3. cross-check the seed hash against the most-recent `SEED-GAPS-FOR-SELF-HOSTING.md` entry's `seed \`…\`` pin.
A converging count on a live file is noise, not signal.

## Relationship to `run_corpus.py`

`run_corpus.py` targets the OLD shipping `compiler.cdz` against the whole `spec/semantics` corpus (the
value-first differential gate). `cdzc.py` targets the NEW `cdzc.cdz`. They share the same `emit`-based
oracle idea; `cdzc.py` is the cdzc-specific, self-documenting successor for this rewrite.
