# PR #1637 review comment — flake.nix (v-nix) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1637 (MERGED — add cdz-nfc as a derivation).

## Heading says "content-addressed derivation" but it's an INPUT-addressed derivation with hash-from-output (Copilot, flake.nix:203) — doc/accuracy
> The heading says this is a "content-addressed derivation", but this file explicitly builds NFC as a
> normal (input-addressed) derivation and derives the content hash FROM the output. Using Nix's term
> "content-addressed" here is likely to mislead.

In Nix, "content-addressed derivation" is a specific (CA-derivations) feature; this builds NFC as a
NORMAL input-addressed derivation and computes a content hash from the built output bytes (the
hash-from-built-bytes north-star). Calling it "content-addressed" collides with Nix's term. Reword the
heading to "input-addressed derivation; content hash derived from the output bytes" or similar. LOW/doc,
fix-forward. (Aligns with the #1550/#1572 pattern — Nix-term precision on the flake.)
