# The reader is wired — the compiler now reads a program's canonical AST bytes and compiles it, end to end

*2026-07-07*

**What happened.** With every self-hosting seed blocker cleared
([[2026-07-07-the-final-self-host-blocker-is-fixed-the-reader-can-join-the-pipeline.md]]), the spike
**wired the reader into the pipeline** and reached the self-hosting-shaped milestone: `compiler.cdz`'s
`main` now compiles a program **read from its own canonical AST bytes**, end to end. The full chain is
`bytes → Node → Core → component`:

- `read-node : Bytes → Node` recursively decodes the deterministic-CBOR canonical AST — a CBOR array
  (major 4) is an application `[head-index, …children]` → an `NPrim` whose head string is the prelude
  symbol located by `prelude-entry` and matched by `name-eq`, and any scalar is an integer atom →
  `NInt`. It is built entirely from the input-side primitives verified over the preceding cycles
  (`cbor-major`/`cbor-arg`/`cbor-head-len` for head decode, `cbor-skip`/`skip-elems`/`child-off` for
  navigation, `prelude-entry`/`name-eq` for name resolution).
- `resolve` turns the surface `Node`'s names into the typed `Core`, `compile-core` folds → lowers →
  serializes → frames.

Verified end-to-end: the bytes `83 01 81 61 2B 83 00 01 02` — the CBOR encoding of `(+ 1 2)`
(`[version 1, prelude ["+"], root [head-index 0, 1, 2]]`) — compile to a **valid component** whose code
section is `i64.const 1; i64.const 2; i64.add` and which runs. Because the operands arrive at *run
time* (from `Bytes.at`), the tree is not const-folded — the compiler emits the genuine runtime
computation, exactly as a compiler reading arbitrary input must. This is `bytes → component`: the
Cadenza-authored compiler reading a program's own canonical AST and compiling it.

**Why.** This is the payoff of the whole spike arc, and the shape of the payoff is worth recording. The
reader did not require a single large new mechanism — it *composed* primitives that each landed as a
separately-verified, corpus-pinned step: head decode (the input dual of the LEB128 emit spine,
[[2026-07-07-the-reader-decodes-cbor-as-the-input-dual-of-the-output-spine.md]]), structural navigation
([[2026-07-07-the-reader-foundation-is-built-and-gated-on-one-inference-bug.md]]), the name matcher
unblocked by the recursive-Bool fix
([[2026-07-07-the-name-matcher-unblocks-and-the-surface-language-composes.md]]), and the runtime-`Node`
→ `Core` join unblocked by the heap-boxer fix. Each was proven on real bytes before the next was built,
so wiring them was assembly, not invention — the reader "came alive" the moment its last dependency
(the join) landed, exactly as the dead-code staging bet intended. The deeper point: **a self-hosted
front end is not one hard artifact but a composition of small, individually-verifiable byte
operations** — the same lesson the output side taught (the LEB128 encoder composes `&`/`|`/`>>`/concat),
now confirmed symmetrically on the input side. The one honest caveat: this milestone is the
*single-expression* read path (`read-node → resolve → compile-core`); the *multi-def module* path
(`read` a whole `(module …)` into a `DList`, via `resolve-module`) is the remaining wiring, and the true
self-hosting test — the compiler compiling *its own source* — needs that plus scale (a large source
walks deep, where the bounded wasm stack, [[deep-recursion-traps-at-host-stack-limit]], will eventually
bite). So this is `bytes → component` for an expression, not yet `compiler compiles compiler`; the
architecture is proven, the scale is not.

**The requirement it drove.** A conformance case in `10-bytes.sexp` — *"a recursive reader decodes a
CBOR application tree and evaluates it by head index"* — pins the reader's spine as one composed
witness: `ev` decodes the CBOR of `[+ 1 [* 2 11]]` (`83 00 01 83 01 02 0B`) by reading each array's
head-index and recursing into operand offsets located by `child-off`/`skip-elems`, yielding
`1 + (2·11) = 23`. It composes every input-side primitive the earlier cases pin in isolation into the
actual `bytes → value` recursive tree walk, so a single-primitive slip (wrong child offset, wrong head
extraction, a navigation miscount) changes the result — the input dual of the LEB128 known-answer emit
case, and a tighter check on the reader than any primitive alone. It **PASSES**. Together with the
already-pinned head-decode, navigation, and resolver-join cases, the executable semantics now witnesses
the whole `bytes → AST → typed-IR → value` path a self-hosted compiler is built from. No new backlog
item — this consolidates: the self-hosting *architecture* is complete and gate-witnessed end to end;
the remaining work is the multi-def module read wiring and scale (TCO for deep sources), plus the
non-blocking backlog items 12 (symbol-table `from-bytes`) and 13 (list patterns).
