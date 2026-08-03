# PR#1017 review comment — BadText doc: Copilot's multi-scalar premise is INVERTED (read_scalar doesn't reject it) — real latent gap (v-syntax)

Mirrored from GitHub PR#1017 review comment (Copilot), id `3696144991` (codec.rs:136, also :519).
File: `cadenza-ast/src/codec.rs` — v-syntax. Blame `604312a8e` "syntax: add decode_detailed + DecodeError
to codec (codec-extraction S5b)".

## Comment (verbatim)

- (id 3696144991, codec.rs:136) "`DecodeError::BadText`'s doc comment currently only mentions the
  empty-string case for `char`/`bad-escape`. If `read_scalar` enforces canonical single-scalar strings,
  the docs should also call out multi-scalar UTF-8 as `BadText` to match the decoder's behavior. This
  issue also appears on line 519 of the same file."

## Liaison verification (confirmed on trunk ad50154bd — Copilot's PREMISE is INVERTED)

`read_scalar` (codec.rs:521-523): `read_string(r)?.chars().next().ok_or(DecodeError::BadText)`. It takes
the FIRST scalar and returns `BadText` only on EMPTY — it does NOT reject a MULTI-scalar body. A `"ab"`
body decodes to `'a'` (tail silently dropped), NOT `BadText`. So Copilot's conditional ("IF read_scalar
enforces canonical single-scalar…") is FALSE: it does not enforce single-scalar, so there's no
multi-scalar-→BadText behavior to document. A straight doc-add would DESCRIBE BEHAVIOR THAT DOESN'T EXIST.

BUT Copilot half-surfaced a REAL latent gap: the doc (:519-520) says the field "must hold exactly at least
one scalar" (awkward wording) and "the encoder always writes one" — i.e. single-scalar is the INTENT, but
`read_scalar` only checks NON-EMPTY, silently accepting + truncating a multi-scalar body. So a corrupt/
malformed 2-scalar `Char`/`BadEscape` body decodes to the first scalar WITHOUT error — a should-reject gap
for a canonical-form decoder (`decode_detailed` is the classify-malformed path). v-syntax's call:
- (a) if single-scalar canonicity SHOULD be enforced → make `read_scalar` reject a >1-scalar body as
  `BadText` (`chars()` yields exactly one), THEN Copilot's doc-add is correct; OR
- (b) if accepting-first-scalar is intentional → the doc should say so (and the "must hold exactly …one"
  wording, which reads as enforcement, should be softened to "the encoder writes exactly one; decode
  takes the first").
Either way the doc + code should agree. Do NOT just add the multi-scalar-BadText doc line without also
making `read_scalar` reject it (else doc≠code).

Owner: **v-syntax** (`cadenza-ast/src/codec.rs`; `604312a8e`). Decide enforce-vs-accept for a multi-scalar
`Char`/`BadEscape` body, then make doc + code agree (the doc currently implies enforcement the code
doesn't do). Also fix the "exactly at least one" wording at :519.
