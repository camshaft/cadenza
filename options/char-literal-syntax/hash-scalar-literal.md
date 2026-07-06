# Char Literal Syntax — Choice: hash-scalar-literal

> **The default choice for the `char-literal-syntax` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It pins `#\<scalar>` for a `Char` literal,
> reusing the `#`-sigil reader family the `#"…"` symbol literal already established.

## The choice

A `Char` literal is written `#\` followed by either a single scalar or a numeric scalar escape:

```
#\a          ; the scalar U+0061 'a'
#\           followed by a space is #\space via the named form below, not a bare space
#\newline    ; a named non-printing scalar
#\u+1F600    ; a numeric scalar by hex code point — 😀, a supplementary-plane scalar
```

| Form | Reads to | Meaning |
|---|---|---|
| `#\<one-scalar>` | `Char` node | the literal scalar, for any single printing scalar |
| `#\<name>` | `Char` node | a named non-printing scalar: `#\space`, `#\newline`, `#\tab`, `#\return` (the named counterparts of the closed string-escape set) |
| `#\u+<hex>` | `Char` node | the scalar whose code point is `<hex>`, for non-printing or supplementary-plane scalars a direct form cannot spell |

The `#\` prefix follows the Lisp/Scheme character-literal tradition, and the `#` sigil is the same
reader family `#"…"` (symbol) uses, so the char literal adds a token to an established family rather
than a new lexical convention. The canonical tree carries a `Char` node; `#\a` is display sugar for it
the way `a.b` is sugar for `(. a b)`.

## Validation happens in the reader

A `#\u+<hex>` (or any form) that names a value outside `U+0000..=U+10FFFF` or inside the surrogate range
`U+D800..=U+DFFF` is **not a valid scalar**, so it is a compile-time reader error `CDZ0002`
(collections-and-text.md §"A Char Is A Single Unicode Scalar Value"). This is the `Char` analogue of the
out-of-range `Bytes` byte and the string-escape `CDZ0001`: the literal can never denote a value that is
not a real scalar, so every `Char` that reaches the type system is already validated. The runtime path
`Char.from-int` handles the *dynamic* out-of-range case by returning `None` (total, never a trap); the
literal handles the *static* case by rejecting at read time.

## Why not the alternatives

- **`'a'` (single-quote char, C/Rust style).** The `'` sigil is the homoiconic language's `quote`
  shorthand (spec/semantics/12-metaprogramming.sexp); reserving it for chars would collide with the
  meaning every reader in the family expects — the same reason the symbol literal rejected `'foo`.
- **`?a` (Ruby/Emacs-Lisp style).** `?` carries no established meaning in the reader today, but it does
  not compose with a numeric/named escape form as naturally as the `#\name` / `#\u+hex` family, and it
  does not reuse the existing `#`-sigil family the symbol literal already established.
- **A one-scalar `String`.** A `Char` is a *distinct type* (the element of a string's scalar sequence),
  not a length-1 `String`; conflating them would lose the type distinction the whole `Char` addition
  exists to provide and force every scalar comparison through string machinery.

`Char.from-int` remains available with no reader change, so a generation MAY realize the `Char` value
form and its fallible conversion before this literal sugar — the literal is ergonomics over a value
form that already stands on its own.
