# PR#1012 review comment — leb128 shift>=64 comment says "10th continuation byte" but it's the 11th-byte read (v-syntax)

Mirrored from GitHub PR#1012 review comment (Copilot), id `3695947534`.
File: `implementation/seed/crates/cadenza-ast/src/leb128.rs:113` — cadenza-ast → v-syntax. Blame
`366b02411` "syntax: add classified varint reads to leb128 (VarErr truncated-vs-malformed) —
codec-extraction S5a".

## Comment (verbatim)

- (id 3695947534, leb128.rs:113) "The comment above the `shift >= 64` check is inaccurate/misleading:
  this branch is reached only when attempting to read an 11th varint byte (i.e., the encoding is longer
  than 10 bytes), not on the '10th continuation byte'. Updating the wording will avoid confusion about
  when the malformed classification triggers."

## Liaison verification (confirmed on trunk 91a3334a7)

The read loop: `shift` starts 0, `+= 7` each byte consumed. `if shift >= 64` (line 110) fires when
`shift` reaches 70 — which happens AFTER 10 bytes have been consumed (shifts 0,7,14,…,63 = 10 bytes; the
10th byte brings shift to 70 for the NEXT iteration). So on entering the loop for the 11th byte, `shift >=
64` triggers. The comment (111-112) "A 10th continuation byte (all 10 group-bytes present) overflows 64
bits: fully present, so malformed" is imprecise — the 10 bytes were already validly consumed; this branch
guards the 11th-byte attempt (an over-long >10-byte encoding). A u64 LEB128 is at most 10 bytes, so a
requested 11th byte is malformed. Copilot's wording fix is fair: reword to "attempting an 11th varint
byte (a u64 LEB128 is ≤10 bytes; a longer encoding is malformed, not truncated)". Comment-only,
behavior-neutral (the Malformed classification is correct; only the byte-count wording is off).

Owner: **v-syntax** (`cadenza-ast/src/leb128.rs`, their codec-extraction S5a `366b02411`). Reword the
"10th continuation byte" to reflect the 11th-byte-attempt / >10-byte-encoding trigger.
