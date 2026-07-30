# PR#923 + PR#926 review comments — 25-verification "never STORE" doc contradiction + 10-bytes grammar (corpus-bugfix)

Two Copilot review comments, both corpus doc → corpus-bugfix. (A THIRD comment on PR#923,
08-self-hosting-surface.sexp:106, was a paren-miscount FALSE POSITIVE — DISMISSED, not routed; see the
ledger. It's the PR#884 class.)

## Comment A (verbatim) — PR#923, 25-verification:2389

- (id 3684637275, 25-verification.sexp:2389) "This docstring says the Thm pins should 'never STORE in a
  collection', but the case is explicitly validating that storing Thms in a Map is safe and preserves
  opacity. Updating the wording will avoid readers interpreting this as a prohibition that contradicts
  the test."

### Liaison verification (confirmed on trunk 97791a6cb; blame `11030cfa9`)

Case "abstract theorems stored in a MAP stay unforgeable and usable — lookup returns a real Thm". Doc:
"(the Thm pins bind directly or via import chains — never STORE in a collection): kernel-minted Thms as
CHAMP values, looked up and consumed…". The parenthetical "never STORE in a collection" DIRECTLY
CONTRADICTS the case, whose whole point is that storing Thms in a Map IS safe with opacity intact
("kernel-minted Thms as CHAMP values, looked up and consumed by kernel accessors… survives the collection
round-trip with opacity intact"). The "never STORE" clause is stale (likely a pre-this-case invariant
describing OTHER Thm pins) and reads as a prohibition the case violates. Fix: reword — this case
DEMONSTRATES Thms-in-a-Map is safe; drop/qualify the "never STORE" clause (e.g. "prior Thm pins bind
directly; THIS case adds the collection-value idiom"). Doc-only, pin correct.

## Comment B (verbatim) — PR#926, 10-bytes:1759

- (id 3684862087, 10-bytes.sexp:1759) "The doc string ends with an ungrammatical clause: 'rust rows todo
  with the Bytes-CHAMP-key family.' Consider rephrasing to a clear sentence so the note reads correctly."

### Liaison verification (confirmed on trunk 97791a6cb; blame `6e3f8afbc`)

Case "a Bytes-compact key and a flat rebuild both hit a rope-keyed map entry…". Doc ends: "…a 2-byte
compact-slice PREFIX misses (0). rust rows todo with the Bytes-CHAMP-key family." The trailing "rust rows
todo with the Bytes-CHAMP-key family" is a telegraphic fragment (missing verb/structure) — presumably
means "the rust-target rows are TODO, tracked with the Bytes-CHAMP-key family". Reword to a clear
sentence. Doc-only.

Owner: **corpus-bugfix** (both `spec/semantics/*.sexp` case docs; `11030cfa9` + `6e3f8afbc`). Two doc
rewords (Thm-store contradiction + bytes grammar fragment).
