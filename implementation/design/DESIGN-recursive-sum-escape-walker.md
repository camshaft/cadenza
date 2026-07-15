# Design — the recursive-sum `encode()` escape walker (render a runtime recursive sum as the boundary result)

**Status:** design (2026-07-13, sum vertical). The value-form escape for a RUNTIME-built RECURSIVE sum —
a linked list, a tree — the last shape of "user sums of all shapes" that still declines. Follows
`DESIGN-runtime-bytes-escape-walker.md` (the first looping walker) and reuses the R2 resource-escape
envelope. This is a LARGER walker than any prior one: the output is a whole binary-AST document for an
unbounded-depth tree, not a fixed hole-template nor a flat length-prefixed leaf.

## The problem

A single nullary export returning a runtime-built recursive sum declines:

```lisp
(type IntList (Cons (Tuple Int64 IntList)) Nil)
(def (count n) (if (< n 1) ((. IntList Nil) ()) ((. IntList Cons) (tuple n (count (- n 1))))))
(def (main) (count 3))     ; ⇒ "returning a IntList from `main`: rendering this compound as the host
                           ;    result needs a value-form walker that loops to a runtime-determined depth"
```

`count 3` builds a heap spine `Cons(3, Cons(2, Cons(1, Nil)))` correctly — the DECLINE is only at the
BOUNDARY render (`wasm/mod.rs` ~line 304). `sum_form_template` (`lower.rs:3684`) builds one
`ValueFormTemplate` per variant, but a variant whose payload TYPE is (or transitively contains) the sum
ITSELF cannot produce a fixed-shape template: `template_value_ast_flagged` (`lower.rs:3820`) handles
`Int`/`Bool`/`Tuple`/`Record` and returns `None` for a `Ty::Sum` payload. A finite nested sum
(`Outer(Inner(5))`) escapes today ONLY because it FOLDS to a constant (`constant_value_form` bakes the
bytes); a value whose depth is runtime-determined has no constant form and no fixed template.

## Why this is hard: the value form is NOT compositional

The canonical value form is the binary AST codec (`codec.rs`): an 8-byte header, a LEAF POOL
(count + each leaf), then a STRUCT TABLE (count + each struct as tags + INDICES into the leaf pool and
into earlier structs). Indices are ABSOLUTE positions in those two tables.

```
(: (Cons (tuple 3 (Cons (tuple 2 (Cons (tuple 1 (Nil unit))))))) IntList)
```

renders as ONE document: a single leaf pool holding every distinct leaf (`:`, `Cons`, `tuple`, the
Int magnitudes `3`/`2`/`1`, `Nil`, `unit`, `IntList`, `.`, …) and a single struct table whose entries
reference those leaves and each other by absolute index. So the sub-value `Cons(2, …)`'s rendered bytes
CANNOT be spliced into the parent's bytes by a rope `bytes-concat` — the child's leaf/struct indices
would collide with the parent's. **The bytes-rope trick that made the flat Bytes walker cheap does not
apply.** The walker must assemble ONE leaf pool + ONE struct table for the whole tree.

## The three candidate approaches

### (A) Runtime document-builder walker (hand-emitted wasm) — faithful but the largest
Emit a recursive/iterative core function that walks the heap spine and appends to a growing leaf pool +
struct table in linear memory, tracking the running leaf-count / struct-count / index back-references,
then emits the header + both tables. This is a runtime re-implementation of `codec::encode` for the
sum's shape. Correct and self-contained, but ~300–500 lines of hand-emitted wasm with the index
bookkeeping the codec does — the highest-risk path (an off-by-one in an index reference is an invalid
document the host can't parse).

### (B) A runtime `encode` RUNTIME OP — move the walk into `cdz-runtime` (Rust)
Add a heap op `value-encode(handle) -> bytes` to `cdz-runtime` that walks the tagless heap node and emits
the canonical document in Rust (where `codec::encode` already lives, and recursion + a `Vec` growable
buffer are free). The compiler's escape path then just calls it. DRAMATICALLY simpler emit side (one
op call, no hand-emitted codec), and the Rust walk is easy to get right. **Cost:** it changes the frozen
runtime (a new op → `REQUIRED_RUNTIME_HASH` bump, the WIT + `runtime_abi` codegen), and the runtime must
know the VALUE-FORM rules (variant head spelling qualified `(. T V)` vs bare, the `(: value Type)` frame,
nullary `unit`) — today those live in the COMPILER (the template builders), per the WIT's own note
("the type-directed renderer the compiler bakes into the program walks a value of KNOWN shape"). Moving
render rules into the runtime crosses the compiler/runtime boundary the design deliberately keeps: the
runtime is TAG-FREE and shape-agnostic; only the compiler knows a heap node is an `IntList`. So (B) needs
the compiler to pass the shape (a small descriptor: per-disc head bytes + payload layout) INTO the op, or
the op stays generic over "render this heap value as its structural form" WITHOUT the nominal type name —
but the value form REQUIRES the type name (`… IntList)` / the qualified variant head), which the runtime
does not have. So (B) as a pure runtime op cannot produce the nominal value form; it would need a
shape-descriptor argument, re-introducing much of (A)'s complexity at the ABI.

### (C) Emit a recursive DEFINED function in the program that returns the value-form Bytes — RECOMMENDED
Between (A) and (B): keep the render rules in the COMPILER (as today), but instead of a fixed
hole-template, emit — for the escaping recursive sum — a real recursive CORE FUNCTION
`t-encode-<Sum>(rep) -> bytes_handle` that builds the value-form document. Crucially, sidestep the
non-compositionality by having the walker build the document in a SINGLE pass with a runtime leaf/struct
appender exposed as a SMALL set of new runtime ops that are GENERIC (no nominal knowledge):

  - `doc-new() -> builder`
  - `doc-leaf-name(builder, ptr, len) -> leaf_ix`   (append a NAME leaf, return its index)
  - `doc-leaf-int(builder, magnitude-bytes…) -> leaf_ix`
  - `doc-struct-atom(builder, leaf_ix) -> struct_ix`
  - `doc-struct-list(builder, child_struct_ix…) -> struct_ix`
  - `doc-finish(builder, root_struct_ix) -> bytes`  (emit header + pool + table)

The compiler emits the recursive walk (it knows the shape — which head, where the payload sub-value is),
calling these generic builder ops to assemble indices; the RUNTIME owns the index bookkeeping + final
byte layout (the error-prone part), but knows NOTHING nominal (the compiler passes the head NAME bytes).
This splits the concern cleanly: compiler = shape/recursion, runtime = document assembly. Cost: a handful
of new runtime ops (hash bump) + the recursive emit, but each op is trivial and testable in Rust, and the
emit is a structural walk (no hand-rolled index math). **This is the recommended path.**

## Recommended plan (approach C), incremental

- **RS-0** — runtime `doc-*` builder ops in `cdz-runtime` (Rust): a builder struct holding `Vec<Leaf>` +
  `Vec<Struct>`, the append ops, and `doc-finish` reusing `codec::encode`'s serialization. Unit-test the
  ops in Rust against `codec::encode` of a known document (byte-identical). Hash bump + WIT + codegen.
- **RS-1** — the compiler's recursive emit for a LINKED-LIST-shaped sum (one recursive payload position):
  `t-encode-<Sum>(rep)` switches on `sum-disc`, and per variant emits `doc-leaf`/`doc-struct` calls for
  the head + the `(: … Type)` frame, RECURSING (`call self`) on the self-referential payload position and
  splicing the returned root struct index. Gate on the `IntList` spine case.
- **RS-2** — generalize to MULTIPLE recursive positions (a binary tree: `Node (Tuple Tree Tree)`), and to
  a payload TUPLE mixing scalar + recursive positions (`Cons (Tuple Int64 IntList)`). Gate on the tree case.
- **RS-3** — mutual recursion (two sums referencing each other, `Node`/`Core`) — emit one encode fn per
  sum, cross-calling. (Only if a corpus case needs it; the resolver case also needs runtime strings, a
  separate vertical.)

## Corpus (already present, currently `todo`/decline)

- `05-compound-types.sexp:1672` "a recursively-built linked list renders its full runtime spine" —
  `(count 3)` ⇒ `(: (Cons (tuple 3 (Cons (tuple 2 (Cons (tuple 1 (Nil unit))))))) IntList)`.
- `05-compound-types.sexp:1694` "a recursively-built binary tree renders its full runtime structure" —
  `(build 2)` ⇒ the full `Node`/`Leaf` tree value form.

Both are the exact witnesses; RS-1 closes the first, RS-2 the second.

## Boundaries / non-goals

- **Runtime strings** (a String discriminant materialized through recursion — the two resolver cases)
  are a SEPARATE vertical (the runtime-string looping walker, #46). A recursive sum whose payload is a
  runtime String needs BOTH; do the sum walker over scalar/heap payloads first.
- **Ownership:** like the flat walkers, `encode` owns `own<t>` and must `drop` the walked spine exactly
  once after rendering (the value heap is acyclic; a single `drop` of the root cascades). The recursive
  walk BORROWS as it descends (`sum-disc`/`sum-payload`/`arr-get` borrow) and drops the root at the end.
- Keep the render rules (head spelling, `(: v T)` frame, nullary `unit`) in the COMPILER — the runtime
  `doc-*` ops stay nominal-agnostic (compiler passes NAME bytes). This preserves the WIT's tag-free
  runtime invariant.

## Related
`DESIGN-runtime-bytes-escape-walker.md` (the first looping walker, the flat precedent);
`DESIGN-value-heap-rcdzc.md` §3a (the R2 resource escape); `sum_form_template`/`variant_form_template`/
`template_value_ast_flagged` (`lower.rs`, the fixed-template builders this replaces for the recursive
case); `emit_runtime_sum_resource` (`wasm/mod.rs:1037`, the non-recursive sum escape); `codec::encode`
(the document format the walker reproduces).
