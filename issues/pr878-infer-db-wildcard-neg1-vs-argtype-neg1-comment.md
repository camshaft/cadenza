# PR#878 review comment — infer-db.cdz -1 wildcard sentinel vs argType -1 comment ambiguity (v-compiler-ml)

Mirrored from GitHub PR#878 review comment (Copilot), id `3667348887`.
File: `implementation/compiler-ml/src/infer-db.cdz:786` — compiler-ml PORT source, code-shape/comment →
v-compiler-ml (the port owner). Blame `1913a15cb` "compiler-ml: _-wildcard ctor-pattern binders
(ignore-field via -1 sentinel)".

## Comment (verbatim)

- (id 3667348887, infer-db.cdz:786) "The new wildcard comment introduces a second '-1 sentinel' meaning
  (binderId == -1), but it doesn't distinguish this from the existing '-1' used in argType encodings
  (which decode to TErr). As written, readers can easily confuse 'binder -1' with 'argType -1',
  especially given nearby comments about '-1 field → TErr'. Clarify in the comment that the -1 here is
  specifically the binderId wildcard sentinel, and is distinct from the argType encoding sentinel."

## Liaison verification (confirmed on trunk 64ee9058c)

`seed-ctor-binders-go` line 785-786: `(if (bid == (0 - 1)) then ...skip...)` with the comment "a -1
sentinel binder types NOTHING". Immediately above (line 778) is "A -1 field → TErr binder → any use
declines" — that -1 is the argType ENCODING sentinel decoded by `decode-argtype-enc`, a DIFFERENT -1
from the binderId wildcard. Both live within ~8 lines, so the ambiguity is real: two distinct "-1
sentinel" meanings (binderId wildcard vs argType-encoding TErr) with no disambiguating wording. Reword
the wildcard comment to name it "the binderId WILDCARD sentinel (distinct from the argType-encoding -1
that decodes to TErr)". Comment-only, behavior-neutral.

Owner: **v-compiler-ml** (compiler-ml port source, code-shape/comment of their own `1913a15cb`). Comment
disambiguation.
