# PR#1026 — compiler-ml: sread.cdz record-field-type-unbound doc says "checks only the HEAD" but the code keeps the field NAME as `last` (v-compiler-ml)

One Copilot review comment, `implementation/compiler-ml/src/sread.cdz` → v-compiler-ml.
(compiler-ml is DE-PRIORITIZED per operator, but routed for the owner's queue.) Gate = compiler-ml
self-host suite (per the rcdzc-emit-verify rule) + `cargo test -p rcdzc`.

## Comment (verbatim) — sread.cdz record-field-type-unbound doc (id 3696415944)

- "The doc comment for `record-field-type-unbound` says that when the field type is a nested applied form
  like `(Box X)` the scan 'checks only the HEAD', but the implementation actually skips the entire
  parenthesized group (`skip-to-close`) without reading `Box`. In that case the 'last' atom remains
  whatever preceded the `(` (typically the field name), so the comment is misleading about what gets
  validated."

## Liaison verification (confirmed on trunk 0565a93e4)

`record-field-type-unbound` (sread.cdz:697): on `char-at(s,q) == "("` it does
`record-field-type-unbound(s, skip-to-close(s, q + 1, 1), last, tree)` — it SKIPS the whole `(…)` group
and RECURSES WITH THE SAME `last` (the inline comment there even says "keep prior `last`"). So for a field
`(f (Box X))`, when the scan reaches `(Box X)` the prior `last` is `f` (the field NAME), and `Box` is
never read as an atom. The doc (:692-694) says: "if the field type is itself a nested applied `(Box X)`,
this reads the atom BEFORE the `(` as the last scanned atom and checks only that HEAD". Copilot is right:
"checks only that HEAD" misleadingly implies it checks `Box` (the applied-form head), but the atom before
`(` is the field NAME `f`, not `Box` — and `type-atom-unbound(last, …)` then checks `f`, not the type at
all. (Whether checking the field name is even correct is a separate question, but the finding is scoped to
the DOC being misleading about what's validated.) Fix: reword to say a `(`-led field type is SKIPPED
(skip-to-close) and the atom kept as `last` is whatever preceded the `(` — typically the field name — NOT
the applied-form's head; i.e. a nested applied field type is not descended into (matching C's
no-recurse-into-applied-args), so its head `Box` is not the checked atom. v-compiler-ml should also sanity
-check whether keeping the field name as `last` (vs. the field type) is the intended validation semantics
here, or just tighten the doc if the behavior is deliberate.

Owner: **v-compiler-ml** (`implementation/compiler-ml/src/sread.cdz`). Doc-accuracy: the "checks only that
HEAD" wording implies `Box` is validated when the code actually keeps the atom before the `(` (the field
name) as `last`. Reword to match (skip-to-close, keep prior `last`); confirm the field-name-as-`last`
behavior is intended. Low priority (compiler-ml de-prioritized; doc-only unless the semantics are wrong).
