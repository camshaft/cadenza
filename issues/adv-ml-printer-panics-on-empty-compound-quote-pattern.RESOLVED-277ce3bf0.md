# BREAKER FINDING 2026-07-21 (trunk 76ed1b0eb) — ML PRINTER PANIC (never-panic invariant broken)

**`cdz convert --to ml` PANICS** (`internal error: entered unreachable code`,
`cadenza-syntax/src/printer.rs:2440`) on an **empty-compound `quote` in PATTERN position**:

```
(do
  (def (main) (match (quote ()) ((quote ()) 1) (_ 0)))
  (export main))
```

## Isolation (all on trunk 76ed1b0eb)

| shape | result |
|---|---|
| `(quote ())` in EXPRESSION position | OK — prints `quote(#[])` |
| `(quote (a))` non-empty, PATTERN position | OK — prints `quote(a())` |
| `(quote ())` empty, PATTERN position | **PANIC** printer.rs:2440 `unreachable!` |

The panic site is the printer's pattern arm falling to a catch-all that assumes the node is an
`Atom` leaf; an empty-compound quote pattern reaches it as a non-Atom (empty list node) →
`unreachable!()`. The EXPRESSION-position printer has an empty-compound arm (`#[]`); the PATTERN
printer lacks it.

## Severity / urgency

- Violates the syntax lane's **never-panic** invariant (a reader/printer must total-error, not abort).
- **Imminent fleet-wide impact:** the queued corpus pin `8091554bf` ("pin the empty-compound quote
  pattern — the zero-arity end of fixed-arity", metaprogramming lane) adds exactly this shape to
  12-metaprogramming. The moment it lands, `cargo xtask roundtrip` + `corpus_roundtrip` go RED for
  every agent (this was ALREADY observed on a discarded test lineage that briefly carried the pin —
  2 cases errored). Fixing the printer BEFORE that pin lands avoids a trunk-red window.

## Suggested fix shape (v-syntax lane)

Give the ML pattern printer the empty-compound arm the expression printer has (print the pattern as
`quote(#[])` — matching the expression sugar — or whatever the reader accepts back; ensure
reader/printer agree so the round-trip closes).

## Repro

`target/release-debug/cdz convert <file above> --to ml` → panic. The compile path is unaffected
(the program compiles and runs to 1 on wasm).
