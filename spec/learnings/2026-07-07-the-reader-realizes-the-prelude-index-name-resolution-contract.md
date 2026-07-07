# The reader realizes the prelude-index name-resolution contract — head names resolve by byte-comparing prelude symbols, no runtime String

*2026-07-07*

**What happened.** The reader's name-resolution seam matured to directly realize the AST-encoding
contract. `ast-encoding.md` requires (§"A stored binary AST MUST carry a prelude that lists every
symbol its nodes reference" and §"A node MUST name its kind by referencing a symbol in the prelude by
index rather than by carrying the symbol inline") that the canonical AST is `[version, prelude, root]`
where the prelude is the list of distinct symbol names and each application head is a *prelude index*,
not an inline string. The reader realizes exactly this: `prelude-entry b k` locates the Nth prelude
entry (a CBOR text symbol) by `skip-elems` from the prelude's first element, and `name-eq` byte-compares
that entry's payload against a known operator name (`b"+"`, …) — **length first, then each byte** — to
turn a head index into an operator identity. No runtime `String` is involved on this path: the head
resolution is pure `Bytes`/byte-equality over the prelude text, which is why "resolve names to codes"
falls out of the format at read time. (Separately, the reader's *symbol-table materialization* — turning
a prelude byte-slice into a `String` value — is the in-flight `bytes-is-utf8` / `String.from-bytes` work,
[[2026-07-07-string-from-bytes-validates-in-the-runtime-a-string-is-a-utf8-bytes-leaf]], still unbuilt at
this snapshot; head resolution does not need it.)

The load-bearing detail is that `name-eq` checks **length before bytes**. Comparing the prelude symbol
`"++"` (CBOR text-of-2) against the operator name `b"+"` (length 1) must be **false** — but a byte loop
without the length guard would see the first byte match (`+` = `+`) and wrongly resolve `"++"` to `+`.
The length-prefix check is what makes a prefix not be mistaken for the whole name.

**Why.** This is the AST-encoding contract paying off exactly as designed, and worth recording as the
*realization* of a spec property rather than a new discovery: because the format interns symbol names
into a prelude and nodes carry an *index*, the reader never parses an inline symbol string — it gets a
number, indexes the prelude, and byte-compares. The "resolve names to codes" step (the resolve-before-
select seam the whole compiler is built around) is therefore not a separate pass the reader must
implement but a property of the *input format*: the codes are already there as indices, and the reader's
only job is to map an index to the operator it knows by comparing the interned name. This is the same
"the input format is an ally" observation the spike made early (the CBOR head is a symbol index the
compiler can match on directly) now fully realized on runtime bytes. The prefix-rejection detail is the
one place a naive realization goes wrong: name resolution by byte comparison *must* be length-anchored,
because operator names are not prefix-free (`+`/`++`, `<`/`<=`, `>`/`>=` all share prefixes), so a
comparison that stops at the shorter length mis-resolves. Length-first is the small invariant that keeps
index→operator resolution correct.

**The requirement it drove.** A conformance case in `10-bytes.sexp` — *"resolving a head against a
prelude symbol rejects a length-mismatched prefix"* — pins the length-anchored symbol compare: `name-eq`
of the prelude text `"++"` (`62 2B 2B`) against `b"+"` (length 1) is **0** (false), because the lengths
differ even though the first byte matches. It realizes `ast-encoding.md`'s prelude-index name-resolution
on runtime bytes and pins the prefix-rejection guard a self-hosted reader needs (without it, `+` would
resolve against a prelude `++`). It **PASSES** and joins the reader's other primitive cases (head
decode, navigation, atom decode, tag skip, length-driven iteration) as the name-resolution leg. No new
backlog item — this is subset-growth realizing an already-specified contract, not a gap; the standing
open work is the compiler emitting its own richer constructs (sum types / `match` / `String` /
recursion) and scale (TCO), plus the in-flight `String.from-bytes` runtime support (backlog item 12's
`from-bytes` facet), to be confirmed by probe once the seed and runtime are rebuilt.
