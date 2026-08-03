# PR #1066 review comment — guide/src/content/chapters/SizedIntegers.tsx (v-guide)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1066
(PR: "cand: v-guide — SizedIntegers chapter").

## Guide overclaims what the CDZ0203 diagnostic says (Copilot, SizedIntegers.tsx:76) — doc/correctness
> The Note claims the diagnostic "names the concrete width you almost certainly meant", but the
> compiler's non-type-annotation message for a bare `Int` doesn't infer a width; it uses a generic
> example (hard-coded `Int64`) and explains that `Int` is a value, not a type. This wording could
> mislead readers who intended a different width.

Real doc-vs-code point: the CDZ0203 message (from rcdzc `non_type_annotation_message`) suggests the
DEFAULT sized type `Int64`, it does NOT infer the width you meant. Reword the chapter Note so it
doesn't imply width inference. (Note: this is adjacent to PR #1058's CDZ0203 wording, now on trunk —
worth aligning the guide's description to that message's actual text.)
