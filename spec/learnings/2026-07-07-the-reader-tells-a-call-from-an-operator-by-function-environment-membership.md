# The reader tells a call from an operator by function-environment membership — two namespaces, one lookup

*2026-07-07*

**What happened.** The reader's application decode (`read-app`) grew to distinguish a **user-function
call** from a **primitive operator**. It now carries a *function environment* (`fenv`) — the module's
`def` names' prelude indices — alongside the parameter environment. When it decodes an application head,
it first checks the arity-special forms (`let`/`if`/`not`), then looks the head index up in `fenv`: if
present, the head names a module function and the application is a **call** to that function's index
(`read-call`); otherwise it falls through to a **binary operator** (`read-op-name` maps the head bytes
to `+`/`<`/`and`/…). The reader's recognized operator set also filled out to the full surface
(`+ - * / % < > = <= >= != and or not`), so a decoded program can use any of them. Verified the
disambiguation logic in isolation (it is the same index-environment lookup as scope resolution): a head
index in `fenv` resolves to its function slot, a head not in `fenv` returns -1 (the "it's an operator"
signal).

**Why.** The interesting point is that a canonical AST head is **untyped as to what it names** — the
same syntactic position holds `+` (a primitive), `f` (a user function), and `if` (a special form), each
just a prelude index. The reader must resolve which *namespace* the head belongs to, and it does so with
**one mechanism reused across two environments**: the very same `ienv-pos` index-list lookup that
resolves a name reference to a local slot ([[2026-07-07-the-reader-resolves-names-to-local-slots-with-lexical-shadowing]])
resolves a head to a function slot — the difference is only *which* environment it searches (the
parameter env for a value name, the function env for a head). This is a small but clarifying design
fact: **scope resolution and call-vs-operator resolution are the same operation (membership in an
ordered index environment) applied to different environments**, so a self-hosted reader needs no
separate call-detection machinery — it needs the right environment threaded to the right lookup. The
disambiguation is clean because the namespaces don't overlap: operator symbols never appear in `fenv`
(only `def` names do), so a head that isn't a declared function is unambiguously an operator, and
lookup-fails-means-operator is a total rule, not a heuristic. This is the reader reaching the point
where a multi-`def` module's functions can **call each other** (not just call operators) — the last
structural piece of the surface a real multi-function program uses.

**The requirement it drove.** No new corpus case. The disambiguation is a *reader-internal* step over
the pre-parsed byte format: at the source level, a call `(f x)` and a primitive `(+ x y)` are already
unambiguous (the head is a name vs. an operator symbol), so there is no source-level observable that
isolates the byte-decode disambiguation without reconstructing the reader — and the mechanism it uses
(ordered-index-environment membership) is already pinned by the shadowing scope-resolution case, while
the behaviors it enables (a `def` calling another `def`; each operator) are already covered in
`09-functions.sexp` and the operator/connective cases. Pinning it again would be redundant. This
learning records the design — two namespaces resolved by one environment lookup — as the durable
takeaway, consistent with the standing rule that a *reader-internal completeness step realizing
already-witnessed behaviors* earns a learning, not a duplicate corpus case (only a genuinely new
observable or a decline-don't-miscompile gap earns a case). The reader now decodes the full surface a
multi-function program over the arithmetic/comparison/boolean/`if`/`let`/call subset uses; the standing
frontier is unchanged — the compiler *emitting* `match` on user sums, and scale (TCO).
