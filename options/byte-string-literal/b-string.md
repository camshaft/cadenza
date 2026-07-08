# Byte-String Literal Syntax — Choice: b-string

> **The default choice for the `byte-string-literal` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It pins `b"…"` for a `Bytes` literal and its
> display, matching the Rust `bytes` crate's `Debug` rendering, and reusing the string-literal token
> the reader already has.

## The choice

A `Bytes` literal is written `b"…"` — the sigil `b` immediately followed by a double-quoted body. Each
body byte contributes one byte to the sequence; the body is a raw byte sequence, NOT NFC-normalized text
(unlike a `String` literal). The escape set is:

```
b"ABC"            ; the three printable bytes 65 66 67  = (Bytes.of (list 65 66 67))
b""               ; the empty byte sequence             = (Bytes.of (list))
b"\x89PNG"        ; the PNG magic prefix 137 80 78 71   = (Bytes.of (list 137 80 78 71))
b"line\n"         ; 'line' then a newline (byte 10)
b"a\"b\\c\0"      ; a quote, a backslash, and a NUL, escaped
```

| In the body | Byte |
|---|---|
| a printable ASCII char `0x20..=0x7e` (except `"` and `\`) | itself |
| `\n` `\r` `\t` | 10, 13, 9 |
| `\\` `\"` | 92, 34 |
| `\0` | 0 |
| `\xNN` (two hex digits) | the byte `0xNN`, any value 0..=255 |

The `b` sigil is a byte-string introducer ONLY when a `"` follows it immediately; a bare `b`, or a name
that merely starts with `b` (like the `bin` binary form's head, or `bytes`), stays an ordinary name — so
the literal does not collide with the `(bin …)` / `(bytes …)` grammar (16-binary-matching.sexp). The
canonical tree carries `(Bytes.of (list …))`; `b"…"` is display sugar for it the way `a.b` is sugar for
`(. a b)`, so a byte-string literal and the explicit form are byte-identical trees and denote one value.

## The display form is the same shape, and it round-trips

A byte sequence *renders* back to `b"…"` by the inverse rule: a printable ASCII byte prints as itself,
the named bytes print as `\n \r \t \\ \" \0`, and every other byte prints as `\xNN` (two **lowercase**
hex digits). This is exactly the Rust `bytes` crate's `Debug` implementation. Because the reader escape
set is the inverse of the display escape set, rendering a byte sequence and reading the result back
yields the same value — the round-trip the constitution requires over the canonical form
(homoiconic-decoupled-display.md). The escape *order* is load-bearing: `\` and `"` fall inside the
printable range, so a renderer must test them before the printable-passthrough arm, and a reader/renderer
pair that disagree on order would break the round-trip.

Three renderers must agree byte-for-byte — the compile-time constant fold, the emitted-wasm
type-directed renderer, and the runtime crate's reference renderer — plus the corpus oracle; they share
one `escape_byte` helper so they cannot drift (the differential gate compares oracle-vs-compiled text,
so a drift is a gate failure, not a silent divergence).

## Why not the alternatives

- **Keep `(Bytes.of (list 137 80 78 71))` as the only form.** Legible for `1 2 3`, unreadable for a real
  magic number or a UTF-8 string's bytes; it hides that `72 101 108 108 111` is `"Hello"`. The explicit
  form stays valid (it is the desugaring), but a wall of decimals is a poor *default display* for the
  compiler's own byte output.
- **A hex-only form `x"89504E47"`.** Compact for binary blobs but unreadable for the common case where a
  byte sequence is mostly ASCII (HTTP headers, wasm section names, source fragments) — the whole reason
  the `bytes` crate prints ASCII when it can. `b"…"` degrades gracefully: printable runs stay legible,
  non-printable bytes fall back to `\xNN`.
- **`0x`-prefixed byte arrays or a `#bytes[…]` form.** Introduces a new bracket token and does not reuse
  the string-literal lexer the reader already has; `b"…"` adds one sigil to the existing string token,
  the minimal lexical change, and matches a convention (`b"…"` byte strings) programmers already know
  from Rust and Python.
- **Reusing `#"…"` (the symbol sigil) for bytes.** `#"…"` already reads to a `Symbol`
  (options/symbol-interning/); overloading it would collide. The `b` sigil is unclaimed before a `"` and
  reads mnemonically as "**b**ytes".

`Bytes.of` remains available with no reader change, so a generation MAY realize the `Bytes` value form
and its display before this literal sugar — the literal is ergonomics over a value form that already
stands on its own.
