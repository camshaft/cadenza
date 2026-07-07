# The whole-module reader is wired — the compiler reads a multi-def module's canonical AST and compiles it

*2026-07-07*

**What happened.** The spike extended the reader from a single expression to a **whole module** and
wired it as `main`: `compiler.cdz` now compiles `module bytes → component`. The bytes of
`(module m (def (main) 42))` — deterministic CBOR `[version 1, prelude [def,m,main,module],
root [module-idx, name, def[def-idx, sig, 42]]]` — run through the complete pipeline `read-module →
resolve-module → fold → lower → serialize → frame` to a valid 89-byte component whose `run` returns 42.
This is the multi-`def` self-hosting shape that the single-expression path
([[2026-07-07-the-reader-is-wired-bytes-to-component-end-to-end.md]]) pointed at: the compiler reads a
whole program's own canonical AST and compiles it.

The new reader machinery is `read-module`/`read-defs`, and its defining move is reading **CBOR array
lengths as structural counts**: the module root is `[module-idx, name, def…]`, so the number of `def`s
is the root array's length minus 2 (`mod-ndefs` = `cbor-arg` on the root head, minus the head and
name); each `def` is `[def-idx, sig, body]` where `sig = [fn-name, params…]`, so a def's parameter
count is the signature array's length minus 1 (`def-nparams-of`). `read-defs` walks the root's def
elements into a `DList` of `Def`s, reading each body with the existing `read-node`, producing exactly
what `resolve-module` consumes. This is a different reader capability from the head-index dispatch the
single-expression path exercised: there the reader read a *scalar* head from an array and dispatched on
it; here it reads the array *length* as data and drives a variable-length walk by it.

**Why.** The lesson is continuity of the composition thesis: the whole-module reader added no new
primitive — `mod-ndefs`, `def-nparams-of`, and `read-defs` are all `cbor-arg` (already verified) applied
to array heads to recover their lengths, plus `skip-elems` (already verified) to locate elements, plus
`read-node` (already verified) for bodies. The step from "read one expression" to "read a whole module"
was assembling proven pieces at one more level of structure, exactly as the reader itself was assembled
from head-decode + navigation + name-match. This is the self-hosting front end reaching its full shape
by accretion of small verified operations, not by a new mechanism — the same way the backend reached
its full shape (multi-function assembly over the LEB128/section primitives). It also sharpens what
"reading the AST" means: a canonical-AST reader is fundamentally two things — *dispatch on a decoded
scalar* (a head index selects an operation) and *iterate by a decoded count* (an array length is how
many defs/args/elements follow) — and both are now built and gate-witnessed. The honest caveat carried
forward: this is still the *architecture* proven at small scale (a one-def module folding to a
constant); the true self-hosting test — the compiler compiling *its own* multi-thousand-line source —
needs scale (the bounded wasm stack, [[deep-recursion-traps-at-host-stack-limit]], will bite a deep
tree-walk before then), so `module bytes → component` for a small module is proven, `compiler compiles
compiler` is not.

**The requirement it drove.** A conformance case in `10-bytes.sexp` — *"a CBOR reader walks a
variable-length array using its decoded length as the element count"* — pins the new capability: a
reader reads a CBOR array's length from its head (`cbor-arg`) and uses that length to drive a walk over
its elements, summing `[10 20 30 40]` (bytes `84 0A 14 18 1E 18 28`) to 100. It is deliberately distinct
from the head-index-dispatch case (which reads a scalar head and dispatches): this reads the array
*length as data* and loops by it — the count half of `bytes → AST`, the shape `read-module` uses to find
a module's def list and each def's parameters. It **PASSES**. Together with the head-decode, navigation,
head-index-dispatch, and resolver-join cases, the executable semantics now witnesses both halves of a
canonical-AST reader (scalar dispatch and length-driven iteration) over the full `bytes → AST → typed-IR
→ component` path. No new backlog item — the self-hosting front end is architecturally complete and
gate-witnessed for both single-expression and whole-module reads; the remaining work is scale (TCO for
deep sources) and the two non-blocking backlog items (12 symbol-table `from-bytes`, 13 list patterns).
