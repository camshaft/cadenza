# One accessor, everything is a record: `.` unifies fields, exports, and qualified names

*2026-07-03*

**What happened.** The seed interpreter first handled dotted names with a **lexical heuristic**: it
read `p.x` and `Sign.Neg` both as a single dotted atom, then guessed the meaning from the case of the
first segment — a lowercase base (`p`) meant *field projection*, an uppercase base (`Sign`) meant a
*qualified variant name*. The operator flagged this as exactly the parsing ambiguity the homoiconic
principle exists to eliminate: the meaning of a construct was re-derived from an atom's spelling rather
than encoded in the tree. The resolution unified three things that were secretly one — a record's field
access (`p.x`), a namespace's qualified name (`Sign.Neg`, `Int64.max`, `List.at`), and a module's
exported definition — into a **single concept, a record (a value with a fixed set of named fields), with
`.` as the sole accessor `(. <record> <key>)`**. The dotted display `a.b` becomes pure sugar the reader
expands to `(. a b)`, so the canonical tree carries only the explicit form and there is no ambiguity to
resolve downstream. `Sign`, `Int64`, `List`, `Bytes`, `Option` became ordinary prelude bindings to
record values; the interpreter's variant-name / builtin-constant / dotted-name heuristics were all
deleted. A module became a scope-builder whose value is the **record** of its exports, with its
capability manifest and entry reachable through a distinct metadata channel — `(. m (meta capabilities))`
— so metadata can never collide with an export.

**Why.** The specification described records, modules, and qualified names as three separate constructs,
and pinned member access only as an informal "`<expr>.<field>`" surface note with no canonical tree form.
That left the interpreter to disambiguate a dotted atom by heuristic, which is fragile (a lowercase
module name or a capitalized binding breaks it) and, more fundamentally, violates code-shape's own
premise that structure lives in the tree, not in re-parsed text. Records and maps were also conflated,
which would have cost the type system later: a **record** has a fixed, statically-known, possibly
heterogeneous field set (so `(. r f)` resolves statically and a type-checker can reject an unknown
field), whereas a **map** is a dynamic, homogeneous key→value association whose lookup can fail — a
distinction worth pinning before typing is realized.

**The requirement it drove.** `spec/capabilities/core-semantics.md` §"Records, Maps, And Member Access"
(a record has a fixed named-field set distinct from a map's dynamic homogeneous entries; member access
projects the named field, and traps on a non-record or a missing field) and §"Modules" (a module
evaluates to a record of its exports; each definition registers its name and value as a field; a module
carries its manifest and entry as metadata reachable by a metadata key distinct from every export name).
The concrete form is pinned in `options/code-shape/homoiconic-decoupled-display.md`: the `.` and `meta`
core symbols, the record-versus-map distinction, and the dotted-name-is-sugar rule. Witnessed by the
member-access and module-record cases in `spec/semantics/05-compound-types.sexp`, discharged by
execution (the behavior gate).
