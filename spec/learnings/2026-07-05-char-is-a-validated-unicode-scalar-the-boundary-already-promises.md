# Char is a validated Unicode scalar — the boundary already promises a type the surface cannot produce

*2026-07-05*

**What happened.** Reviewing the primitive-type inventory surfaced a latent inconsistency: the
type-mapping table **already carries a `char` row** — `Character (Unicode scalar) → char` — but there
is **no `Char` type in the language**. There is no char literal, no operation that returns one scalar
of a string, and no scalar-classification predicate. `String.scalar-len` *counts* scalars, but nothing
can *return* one. The boundary promises a type the surface cannot construct. The resolution: add
`Char` properly as a first-class **validated Unicode scalar value**, so the surface can produce the
type the boundary already names.

**Why the language wants it.** A self-hosting **lexer works one scalar at a time**: `is-digit`,
`is-alpha`, `is-whitespace`, "peek the next scalar," "is this the `(` that opens a form." Today that
has to detour through `Bytes` and re-implement UTF-8 classification by hand — decoding continuation
bytes to recover a scalar the string already knows. A `String` is defined as "a sequence of Unicode
**scalar values**" (collections-and-text.md §"A String Is A Sequence Of Unicode Scalar Values"), so the
element type of that sequence is exactly `Char`; the language names the sequence and its length in
scalars but omitted the element itself. `Char` closes that: it is the element type of the value the
string spec is already written in terms of.

**What it is.** A `Char` is a **validated Unicode scalar value** — a code point in `0..=0x10FFFF`
**excluding the surrogate range `0xD800..=0xDFFF`** (surrogates are not scalar values; a string is a
sequence of *scalars*, so its element type must exclude them). It is representationally a `UInt21`
refined by that predicate, and it maps 1:1 to the component-model `char` the boundary row already
names — so `Char` is what makes that row reachable, not a new boundary type.

**The surface (each piece total, never trapping):**
- A **char literal** — the syntax is an isolated options-decision (like sum-declaration syntax and the
  symbol `#"…"` literal), candidates `?a` / `'a'` / `#\a`; the *type* is fixed here, the *spelling* is
  the choice.
- `String.scalar-at : (String, Int) → Option<Char>` — the scalar-indexed, fallible read, the `Char`
  analogue of `List.at` (collections-and-text.md §"Indexing And Lookup Are Fallible, Not Trapping"):
  in bounds ⇒ `Some` the scalar, out of bounds (including negative) ⇒ `None`. This is the operation
  that was missing — `scalar-len` had no companion that returned an element.
- `Char.to-int : Char → Int` (total — every scalar is an int) and `Char.from-int : Int → Option<Char>`
  (**fallible** — not every int is a scalar; surrogates and `> 0x10FFFF` yield `None`, never a trap,
  never an invalid `Char`). The fallibility is the whole point of `Char` being a *validated* value: the
  check lives at construction, so every `Char` that exists is a real scalar and downstream code needs
  no re-validation.
- Scalar-classification predicates (`Char.is-digit`, `Char.is-alpha`, `Char.is-whitespace`, …) — the
  lexer's inner loop, defined on the scalar rather than on decoded bytes.

**Equality, order, canonical form.** A `Char`'s value is its scalar; equality is scalar equality, and
its order is the scalar-value order — which is exactly the order string comparison is already defined
on (collections-and-text.md §"String Comparison Is Defined On Scalar Values"), so a `Char` order and a
`String` order agree by construction. Its canonical byte form is the scalar as a fixed-width code point
(the `char` boundary form already implies it) — **additive** under deterministic-value-form.md
§"Additive Evolution": a canonical form for a value that previously had none, **no version increment**.

**Either add it or delete the row.** The one state the language should *not* keep is the current one —
a boundary type with no producer. This learning chooses to add the producer. Encoding/decoding
consistency falls out for free: `String.scalar-at` composed across a whole string must reconstruct the
same scalar sequence `String.to-bytes` / UTF-8 decode yields, tying `Char` to the existing total UTF-8
decode (collections-and-text.md §"Decoding Bytes To A String Is Total, Not Trapping").

**Realization / gating.** `Char` is a later-generation value form; its corpus cases carry
`(needs collections)` (scalar access rides the string/collection machinery) and the seed skips them
until a generation realizes scalar indexing. `Char.from-int` returning `Option` and `String.scalar-at`
returning `Option` mean **no new trap** and **no new diagnostic** for the happy path; the only new
rejection is a char literal that names a non-scalar (a surrogate or out-of-range code point), a
reader-level literal error in the `CDZ00xx` band (the same band as the string-escape `CDZ0001`), the
`Char` analogue of the out-of-range `Bytes` byte a later generation checks.

**The requirements it drove.** [collections-and-text.md](../capabilities/collections-and-text.md)
§"Text" gains §"A Char Is A Single Unicode Scalar Value" (validated, surrogates excluded), §"A String's
Scalars Are Addressable" (`scalar-at` is fallible, returns `Option<Char>`), and §"A Char Converts To
And From An Integer Totally" (`to-int` total, `from-int` fallible). The type-mapping `char` row is
annotated as realized by the `Char` type (it was previously an orphan). A char-literal spelling becomes
an `options/` decision. Corpus witness: `Char` cases added to `13-strings.sexp` (scalar-at in/out of
bounds, `from-int` on a scalar / on a surrogate → `None`, `to-int` round-trip, a classification
predicate), alongside the existing `scalar-len` / `byte-len` cases.
