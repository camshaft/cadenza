# PR#917 review comment — 05-compound empty-string-key doc "(len 2)" ambiguous (corpus-bugfix)

Mirrored from GitHub PR#917 review comment (Copilot), id `3683596914`.
File: `spec/semantics/05-compound-types.sexp:16910` — corpus doc → corpus-bugfix. Blame `bb02a2f7c`
"corpus(compound): 6-pin drain P — CHAMP/compound misc…".

## Comment (verbatim)

- (id 3683596914, 05-compound-types.sexp:16910) "The doc string's parenthetical '(len 2)' is ambiguous
  here: it can be read as the 1-char string's length (which would be 1), but the test is actually about
  the map keeping two distinct keys (Map.len == 2). Reword to make it explicit that the length refers to
  the map, not the string key."

## Liaison verification (confirmed on trunk 57a632476)

Case "the EMPTY string keys a map — hit by an empty-concat rope, distinct from a 1-char sibling". The
input inserts `"" → 10` and `(String.concat "a" "") → 20` (i.e. `"a"`), then computes `(* 100 (Map.len
m)) + …`. The output pin `212` decodes as `2`·100 + `1`·10 + `2` — the leading `2` IS `Map.len m` = 2
distinct keys (`""` and `"a"`). The doc ends "The 1-char sibling stays distinct **(len 2)**" — "(len 2)"
means the MAP has 2 entries (`Map.len == 2`), but sitting right after "the 1-char sibling" it reads as
that string's length (which would be 1, not 2) — genuinely ambiguous. Fix: reword to "(the map keeps
both keys → `Map.len` 2)" or "(map len 2)". Doc-only, pin correct.

Owner: **corpus-bugfix** (`spec/semantics/05-compound-types.sexp`; `bb02a2f7c`). Disambiguate "(len 2)"
→ Map.len.
