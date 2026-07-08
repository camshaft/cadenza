# Decision — Byte-String Literal Syntax

**The decision.** The reader spelling AND the canonical display of a `Bytes` value — the surface token
that reads to a byte sequence, and the text a byte sequence renders back to. The constitution and the
collections spec fix that `Bytes` is an immutable byte-sequence value with construction from a list of
integers (`Bytes.of`), equality, length, concatenation, and fallible indexing/slicing
(spec/semantics/10-bytes.sexp; collections-and-text.md); they do not fix how a byte sequence is *spelled*
in text or *displayed* back, because — like the `#"…"` symbol literal and the `#\` char literal — that
surface is a reader-and-printer concern outside the compiler's trusted path (ast-encoding.md §"Parsing
And Printing Are Not In The Compiler's Trusted Path").

**Why the language wants it.** `(Bytes.of (list 137 80 78 71))` is unreadable — a wall of decimal
integers hides that these four bytes are the PNG magic number `\x89PNG`. A self-hosting compiler builds
and pattern-matches magic numbers, opcode tables, and section tags constantly; a legible byte-string
literal makes those values readable at a glance, and a legible *display* makes a rendered byte sequence
(the compiler's own `list<u8>` output) inspectable. The value is fully reachable without any reader
change through `Bytes.of`; the literal is the ergonomic surface, so a generation MAY realize the value
form before the literal spelling.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- The literal reads to an ordinary `Bytes` value — the SAME value `(Bytes.of (list …))` denotes — so it
  adds a surface, not a new value form (10-bytes.sexp §"a byte-string literal equals the explicit byte
  sequence it desugars to"). The canonical tree carries only `Bytes.of`; the literal is display sugar the
  way `a.b` is sugar for `(. a b)`.
- The literal can denote ANY byte 0..=255, including non-printing bytes, so it carries an escape form as
  well as a direct form. A malformed literal (a truncated escape, an unknown escape, an unterminated
  literal) is a compile-time reader error, never a silently-wrong value.
- The spelling MUST NOT collide with an existing reader token — `"…"` already reads to a `String`, `#"…"`
  to a `Symbol` (options/symbol-interning/), `#\` to a `Char` (options/char-literal-syntax/), and the
  `(bin …)` binary form (options/binary-syntax/) is a parenthesized application, not a literal token.
- The display form and the reader form are INVERSES: rendering a byte sequence and reading the result
  back yields the same value (round-trips), consistent with the constitution's round-trip requirement
  over the canonical form (homoiconic-decoupled-display.md).
- It composes with the `(bin …)` binary form (16-binary-matching.sexp): a byte-string literal is a
  whole-value literal (matches by equality, splices into a `(bytes …)` segment), where `(bin …)` is a
  structured segment application — orthogonal surfaces, both denoting an ordinary `Bytes`.

**Why this is an isolated decision.** The form is reader-and-printer sugar over the existing `Bytes`
value form: it lowers to `(Bytes.of (list …))` in the reader and is produced by the type-directed
renderer. It touches no frozen contract, introduces no new value form or node kind, and no capability
requirement — changing the escape set is an edit to a choice file here plus the reader and the shared
byte-escape helper. It is realized by a later generation for the *literal*; the *display* form the seed
already produces (the byte sequence is on the seed's ignition path). Its round-trip corpus cases carry
`(needs bytes)`.

## Choices

- [`b-string`](./b-string.md) — `b"…"` (the `b` sigil immediately before a string literal), with a
  printable-ASCII passthrough and the escape set `\n \r \t \\ \" \0 \xNN`, exactly matching the widely
  used Rust `bytes` crate's `Debug` rendering. **The default.**

DEFAULT: b-string
