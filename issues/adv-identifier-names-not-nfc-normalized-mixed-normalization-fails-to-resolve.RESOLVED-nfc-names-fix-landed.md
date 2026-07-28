# Identifier NAMES are not NFC-normalized → mixed-normalization names fail to resolve (v-syntax finding)

Found 2026-07-21 (v-syntax, trunk 82423de9c) probing unicode identifier round-trip.

## The gap
The reader NFC-normalizes STRING literals (literal.rs:401 `out.nfc()`) and SYMBOLS (`#"…"`/`#name`,
unescape_sym_token line 430) — but NOT identifier NAMES. A bare identifier is interned verbatim
(lexer `ident()` is span-based; parser interns the raw `&str` via `ast::Builder::leaf_name`, the hot
dedup path). So a name written in two Unicode-canonically-equal forms becomes two DISTINCT `Leaf::Name`s.

## Repro (real harm — silent unbound-name)
    def café() = 5          # precomposed é  (U+00E9)
    def main() = café()     # decomposed é   (U+0065 U+0301)
→ `error [CDZ0101]: unbound name `café`` — the call does NOT resolve against the def, though they are
visually identical + Unicode-canonically equal. Control (both precomposed) resolves.

Verified: precomposed `café` interns bytes `63 61 66 c3a9`; decomposed interns `63 61 66 65 cc81` — two
different `Leaf::Name` values. Each round-trips STABLY on its own (the reader preserves what was written,
which is v-syntax's contract), so it is NOT a round-trip bug — it is a NAME-IDENTITY / resolve bug.

## Why NOT fixed unilaterally (needs a ruling + cross-lane)
1. SPEC GAP: the spec mandates NFC only for STRING text ("String stored in NFC", "equality post-NFC");
   identifier names have NO stated normalization rule. So "names are NFC-normalized" is a semantic
   decision, not clearly the current contract.
2. PERF: names intern through the hot `leaf_name(&str)` path that v-compiler-perf deliberately keeps
   allocation-free on dedup hits (classify_word_nonname returns None for a name precisely so the slice
   is interned without a String). NFC-normalizing correctly means a per-name `.nfc()` scan at that hot
   path (or at every intern site) — a hot-path change v-compiler-perf should weigh.

## Proposed fix (once ruled)
NFC-normalize the identifier body at the name-interning funnel (mirror unescape_sym_token's `body.nfc()`),
so `café` precomposed/decomposed intern to ONE `Leaf::Name`. Symbols + strings already do this; extending
to names is consistent + closes the resolve trap. v-syntax owns the reader change; coordinate the hot-path
cost with v-compiler-perf; ruling on whether names ARE NFC-normalized is operator/spec (defaults §naming?).
