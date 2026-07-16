# PR review comment — mirrored from GitHub PR #444 (Copilot inline)

- **PR:** #444 "fleet: sixty-fourth batch (…, rust-backend CHAR, …)" (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/backend/rust/expr.rs:2027` (`rust_char_literal`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3592774829
- **Link:** https://github.com/camshaft/cadenza/pull/444#discussion_r3592774829

## Comment (verbatim)
> `rust_char_literal` claims to emit a valid escape for any control/non-printable scalar, but the current guard only covers C0 controls and DEL. Using `c.is_control()` here matches the existing `cadenza-syntax::literal::render_char` behavior and avoids embedding raw control chars in generated Rust source.

## Liaison triage — CONFIRMED against trunk
Confirmed: `rust_char_literal`'s escape guard is `c if (c as u32) < 0x20 || c == '\u{7f}'` — it escapes
only C0 controls (< 0x20) and DEL. It MISSES C1 controls (0x80–0x9F) and other non-printable/format
scalars (U+0085 NEL, zero-width, bidi controls), which get emitted RAW into the generated Rust `'…'`
char literal → confusing/possibly-malformed generated source. FIX (as reviewer): use `c.is_control()`
for the escape branch (matching `cadenza-syntax::literal::render_char`), so any control scalar is
`\u{..}`-escaped. v-rust-backend. Fix on `trunk`. Quote + link in queue file.
