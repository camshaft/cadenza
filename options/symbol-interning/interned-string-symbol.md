# Symbol Interning — Choice: interned-string-symbol

> **The default choice for the `symbol-interning` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It pins a `Symbol` value that wraps an interned
> `String`, with constant-time content equality, over the existing `String` value form.

## The choice

A `Symbol` is a value that names something and compares in constant time. It is a **nominal value over
`String`** — a structural `String` carrying an orthogonal `Symbol` tag (type-system.md #User Types Are
Declarable As Nominal Or Structural) — with three operations, reached as member access into the
`Symbol` prelude record exactly as `Bytes.of` is `(. Bytes of)`:

| Operation | Shape | Meaning |
|---|---|---|
| `Symbol.of` | `(Symbol.of <string>)` | intern a `String` to a `Symbol` |
| `Symbol.to-string` | `(Symbol.to-string <symbol>)` | recover the `Symbol`'s content as a `String` |
| `=` | `(= <symbol> <symbol>)` | structural equality — true exactly when the contents are equal |

`(Symbol.of s)` is the canonical written form of a symbol value, exactly as `(Bytes.of (list …))` is a
byte sequence's. Two symbols are equal exactly when their underlying strings are equal, so symbol
equality inherits `String`'s **normalized-contents** equality (collections-and-text.md #String Equality
Follows Normalized Contents): two spellings of one source name — a composed and a decomposed `café` —
intern to one symbol. This is the reason a `Symbol` is **String-backed rather than Bytes-backed**: a
symbol table wants two spellings of a name to be the same key, which is String's normalization, not
Bytes' raw-byte identity.

## Identity is content-derived, and that is what makes interning invisible

A `Symbol`'s identity — its equality and its canonical byte form — is a deterministic function of its
**content**. This is the load-bearing rule. The classic interning implementation hands out ids
`0, 1, 2…` in first-seen order and compares those; **that id is forbidden here as an observable**,
because it depends on evaluation order and would make a program's output depend on interning order,
violating deterministic-value-form.md #A Value Has One Canonical Byte Form and core-semantics.md
#Equality Is Structural. The only observations a program can make of a symbol are content-equality
(`=`) and `Symbol.to-string` — both content-defined — so an allocation-order id has nothing to attach
to: it is not merely forbidden but unobservable by construction.

That is precisely what lets the runtime intern **invisibly**. A realization keeps a dedup table
(`content → shared handle`) so that `Symbol.of` returns the existing handle for content already
interned, and `=` short-circuits to a handle compare (constant time) before — or instead of — a byte
scan. Because equal content always maps to the same interned handle, the handle compare is sound and
complete for equal symbols and a fast reject for unequal ones. A symbol built from a computed string
and one built from a literal of the same content are **one value**, indistinguishable by every
operation (memory-and-resource-model.md #Sharing Is Not Observable). The interning is a pure
representation optimization behind the opaque value-heap-runtime handle (component-abi.md #A Runtime
Value Crosses As An Opaque Handle); a generation that does not intern at all — comparing symbols by a
plain byte scan — satisfies the identical observable contract, just without the speedup.

## Equality-only in this version

A `Symbol` supports `=` (and use as a map key), **not** ordering (`<` `>` `<=` `>=`). Equality is the
compiler's hot path — a hash-map symbol table needs equality and hashing, not order — and matches Lisp
symbols and Erlang atoms. A content-**lexicographic** order (the lexicographic order of the underlying
strings, collections-and-text.md #String Comparison Is Defined On Scalar Values) MAY be added
**additively** by a later revision of this choice if a sorted symbol table or a Symbol-keyed ordered
map needs it; it is deliberately left out of v1. An **intern-id order** is never an option — it is the
same forbidden allocation-order observable as an intern-id equality.

## The nominal boundary

A `Symbol` is not comparable to the untagged `String` it wraps: `(= (Symbol.of "x") "x")` is a type
error, rejected `CDZ0202` (type-system.md #Nominal Types Are Not Comparable Across Their Boundary),
exactly as comparing a nominal record to the plain record of its shape is (spec/semantics/05-compound-
types.sexp). Crossing the boundary is explicit — `Symbol.to-string` on one side or `Symbol.of` on the
other — so to compare a symbol's content to a string you write `(= (Symbol.to-string s) "x")`. No new
diagnostic code: the Symbol boundary reuses the existing nominal-boundary rejection.

## The reader literal `#"<text>"`

`#"<text>"` is **reader sugar** for `(Symbol.of "<text>")`: a `#` immediately before a string literal
reads to a `Symbol` node, reusing `String`'s existing lexing — including its escape rules — so the
sugar adds no new escape or token grammar. The canonical tree carries only `(Symbol.of …)`, the way the
display sugar `a.b` carries only `(. a b)`.

```
#"map-insert"   ; reads to (Symbol.of "map-insert")
#"List.at"      ; a qualified name — the dot is ordinary string content
#""             ; the empty symbol
```

## Resolved forks

Three design forks were resolved when this choice was adopted:

- **String-backed** (not Bytes-backed, and not general value hash-consing). A `Symbol` wraps a
  `String`, so its identity is String's normalized-contents equality — two spellings of one source name
  intern to one symbol, which is what a symbol table wants. A Bytes-backed symbol would use raw-byte
  identity (wrong for names); interning **any** value by its canonical byte form (hash-consing whole AST
  subtrees for O(1) structural equality) is strictly more powerful but a far larger commitment that
  touches the entire value heap and the RC discipline — left for a separate decision to be taken only if
  a real need for O(1) subtree equality demands it.
- **Equality-only** (not ordered). Justified above: the compiler's need is a hash-keyed symbol table;
  ordering is an additive later step if a sorted table appears, and only ever content-lexicographic,
  never intern-id order.
- **`#"<text>"` string-form reader literal** (not the Lisp `'foo` quote-shorthand, and not a bare-token
  `#foo`). The `'` sigil is the natural shorthand for this homoiconic language's existing `quote`
  (spec/semantics/12-metaprogramming.sexp) — reserving it for symbols would collide with the meaning
  every reader in the family expects. A bare-token `#foo` cannot carry a qualified name with a dot
  (`#List.at` reads ambiguously against member-access sugar) or arbitrary content without falling back
  to a string form anyway. The string-form `#"…"` interns arbitrary content with String's lexing and
  collides with nothing. (`Symbol.of` remains available with no reader change, so a generation MAY
  realize the value form before the reader sugar.)

## What it replaces

The form replaces raw-`String` name comparison in a self-hosting compiler's symbol table. Name
resolution:

```
; before — O(N) byte scan on every symbol-table probe
(= name "map-insert")
; after — O(1) handle compare
(= sym #"map-insert")
```

and it makes a name a first-class table key whose lookup compares identities rather than contents,
which is the constant-time inner loop the whole feature exists for.
