## 69. ✅ RETIRED as a seed gap (2026-07-08, operator direction) — the new compiler OWNS its parser; no runtime `Ast.decode`

> **RESOLUTION (operator directive):** "implement the parsing in the new compiler and not have an ast-decode
> thing — that's cleaner." So `cdzc.cdz` decodes its input bytes with its OWN recursive-descent parser (over
> the seed's `Bytes.at` + recursion + its own IR sum), rather than the seed growing a runtime `Ast.decode`
> host op. This keeps value-decoding IN the compiler (where the rest of the pipeline lives) and avoids the
> runtime learning CBOR/AST structure (which would break its tag-free design).
>
> **Seed-side VERIFIED (2026-07-08): the seed already supports the full recursive-descent-parser idiom** — no
> new seed gap the moment `cdzc.cdz` starts parsing. Probed and all compile to VALID components:
> - recursive byte-walk + accumulate: `(match (Bytes.at b i) ((Some x) … (recurse (+ i 1) acc)) ((None _) acc))` → sums a byte run;
> - a self-built RECURSIVE sum `(Node (Leaf Int64 | Pair (Tuple Node Node)))` with `(node, next-pos)` tuple
>   threading + MUTUAL recursion, then consumed by a nested match → a real `dec`/`sum` recursive-descent parser;
> - multi-byte int decode via `(| (<< hi 8) lo)`.
>
> So the compiler can hand-roll the CBOR (or any) byte format into its own `Ast`/`Hir` sum with existing
> primitives. The const-fold `Ast.decode` over a LITERAL still works for tests; it is simply not required at
> runtime. Move to `done/`. (Original blocking analysis retained below for history.)

---

## 69. [HISTORY] 🔴 (seed) `Ast.decode` of a runtime PARAMETER declines ("unsupported dotted-application") — blocks the compiler rewrite's front end

**Status: BLOCKING.** The from-scratch compiler rewrite (`cdzc.cdz`, plan: `Ast → Hir → Mir → Lir`) decodes
its input program bytes into the built-in `Ast` sum via `Ast.decode`, per `compiler-pipeline.md`
§Representation ("receive the program as an AST value obtained via quote or **decode from the binary
form**"). A compiler's entry is `compile-bytes b` / `compile inputs` — the input bytes arrive as a
**runtime parameter**. But the seed cannot lower `(Ast.decode b)` when `b` is a parameter; it declines
`unsupported dotted-application`. Phase 0 of the rewrite cannot proceed without this.

**This is NOT a compiler.cdz-vs-native gap — native declines it too.** So it is a genuine seed capability
limit, affecting any Cadenza program that decodes runtime bytes to an `Ast`.

**Minimal reproducers (run on the stable seed; `emit`):**

```
; DECLINES — Ast.decode of a parameter (runtime bytes), matched in the fn:
(module m
  (def (f b) (match (Ast.decode b) ((Ok a) 1) ((Err e) 0)))
  (def (main) (f (Bytes.of (list 24 42)))))
;   native: decline "unsupported dotted-application"

; DECLINES the same way even if the result is returned and matched by the caller:
(module m
  (def (dec b) (Ast.decode b))
  (def (main) (match (dec (Bytes.of (list 24 42))) ((Ok a) 1) ((Err e) 0))))

; COMPILES — Ast.decode of a LITERAL bytes value in main (const-folded), 89 bytes:
(module m
  (def (main) (match (Ast.decode (Bytes.of (list 24 42))) ((Ok a) 1) ((Err e) 0))))
```

So the discriminator is exactly **literal (const-foldable) argument vs runtime parameter**: `Ast.decode`
is only realized as a compile-time fold over a literal; there is no runtime lowering. (For contrast,
`Ast.encode`, `Bytes.len b`, and matching a runtime `Ast` value already bound as a param all compile — it
is specifically the `Ast.decode` *operation* on runtime bytes that has no lowering.)

**What the compiler needs.** `Ast.decode : Bytes → Result<Ast, _>` realized at RUNTIME (on a parameter),
producing a heap `Ast` value the program then pattern-matches. This is the front-end keystone: the whole
rewrite's premise is "decode bytes to the built-in `Ast`, then walk it as an ordinary sum." Verified that
once an `Ast` value exists it flows through functions and re-matches (`(id-ast (quote 7))` → 7) and that
`Ast.decode` of the full program bytes const-folds correctly (`(module m (def (main) 42))`'s 32 AST bytes
decode to an `Ast.List` of 3) — so the DECODER logic exists; it just isn't lowered for a runtime input.

**Related seed gaps found while probing (lower priority, not blocking Phase 0):**
- Bare `quote` still does not flow as a value through a function call: `(f (quote 42))` → "unbound name:
  quote". A compiler decodes rather than quotes, so this is not on the critical path, but worth a fix for
  metaprogramming.
- `Ast.decode (Ast.encode X)` composed in one expression tripped the same `quote`-unbound error in one
  context — may share a root cause with the above.

**Discipline note.** Per the rewrite plan (user-approved), the decision is to **lean on built-in
`Ast.decode` and block on this seed fix** rather than build a hand-rolled CBOR→Ast fallback in the
compiler. The current `compiler.cdz` avoids `Ast.decode` entirely (it hand-walks CBOR by offset) — which is
precisely the brittle offset-based design the rewrite exists to replace. So "work around it" would mean
rebuilding the thing we're trying to eliminate; instead we wait for the runtime `Ast.decode` lowering.

**Priority.** 🔴 HIGH — it is the go/no-go gate for the compiler rewrite's front end. Until it lands, `cdzc.cdz`
Phase 0 (four-layer spine for a scalar `main`) cannot decode its input, and the rewrite is paused here.

**Acceptance signal.** The first reproducer above `emit`s a valid component (and, wired through a real
`compile-bytes`, `(module m (def (main) 42))` → the byte-identical 89-byte scalar component through the
`Ast.decode → resolve → lower → select → serialize` path). Related: ask-39 (runtime AST
construction/encode-decode — this is its decode half on a runtime input), ask-20 (self-inclusion frontier).
