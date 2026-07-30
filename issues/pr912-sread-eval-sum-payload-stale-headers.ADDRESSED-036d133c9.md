# PR#912 review comment — sread-eval-sum-payload stale "multi-field DECLINEs" section header contradicts the pinned tests (v-compiler-ml)

Mirrored from GitHub PR#912 review comment (Copilot), id `3680692181` (:136, also :273).
File: `implementation/compiler-ml/src/sread-eval-sum-payload.cdz` — compiler-ml PORT source, comment →
v-compiler-ml. Blame `1befac27f` "compiler-ml: split sread-eval-sum.cdz — extract the
payload/deconstruction cohort to sread-eval-sum-payload.cdz".

## Comment (verbatim)

- (id 3680692181, sread-eval-sum-payload.cdz:136) "The section header/comments claim multi-field payload
  constructors are 'OUT of the wired subset' and 'must DECLINE', but the tests immediately below assert
  the opposite (that `(P Int64 Int64)` now constructs and deconstructs successfully). This heading is
  misleading and should be updated to match the actual pinned behavior. This issue also appears on line
  273 of the same file."

## Liaison verification (confirmed on trunk 086915ef0)

Line 132 header: "---- MULTI-FIELD payload ctors are OUT of the wired subset → clean DECLINE (boundary
guard) ----" and its body: "A multi-field ctor `(P Int64 Int64)` must DECLINE cleanly (None), never
fabricate a value…". But the immediately-following `@test ss-multifield-payload-ctor-bare-constructs`
(:138) asserts `(P 3 4)` "now CONSTRUCTS + runs (returns the store HANDLE)", and its OWN comment states:
"the arg-N infer fix (type ALL payload args, not just arg1) makes multi-field construction TYPE+LOWER
consistently…; the earlier 'decline' was an ACCIDENT of the infer gap…, NOT a designed result-boundary
check. So (P 3 4) is in-subset and runs." So the section header is STALE — it describes the pre-fix
DECLINE behavior while the pinned tests now assert construction+deconstruction succeed. The `:273` region
is flagged same-class (a header/comment that no longer matches its tests — verify + reword). Fix: rewrite
the :132 header (and :273) to state the CURRENT pinned behavior (multi-field payloads now construct +
deconstruct via the arg-N infer fix; the old decline was an infer-gap accident). Comment/heading-only,
behavior-neutral (the tests + pins are correct; only the stale header misleads).

Owner: **v-compiler-ml** (compiler-ml port source, their `1befac27f` split). Update the stale
multi-field-DECLINE section headers to match the now-pinned construct+deconstruct behavior.
