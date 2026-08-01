# PR#988 review comments — set-malformed case rejected at list-build (not set path) + map-key doc overlap (corpus-bugfix)

Mirrored from GitHub PR#988 review comments (Copilot), ids `3695211427` (:17259) + `3695211435` (:17263).
File: `spec/semantics/05-compound-types.sexp` — corpus → corpus-bugfix. Blame `a5cacdfa1` "corpus(compound):
3-row drain AR — … the uniform malformed-collection code".

## Comment 1 (verbatim) — :17259, coverage

- (id 3695211427) "This case is meant to add coverage for the *set* malformed-collection code, but the
  current input constructs a heterogeneous `(list 1 true)`, which will be rejected at the list
  construction step before `Set.of` can contribute any set-specific behavior. Use a homogeneous list to
  build the initial set, then introduce heterogeneity via a set operation (e.g., `Set.insert`) so the
  failure is attributable to the set construction path."

### Liaison verification (confirmed on trunk a2875840b)

Case "heterogeneous constructions take ONE malformed-collection code across list, map, and set". Input:
`(Set.of (list 1 true))`. The doc says "this adds the SET-element kind (CDZ0201)". But `(list 1 true)` is
itself a heterogeneous LIST — it faults CDZ0201 at LIST construction, BEFORE `Set.of` sees it. So the
CDZ0201 the case pins is the LIST-mix code (already covered by the list case), NOT a set-specific path —
the set-element malformed path is never reached. Copilot right: build a homogeneous `(Set.of (list 1 2))`
then `Set.insert` a mismatched element (`(Set.insert … true)`) so the fault is attributable to the SET op.
Coverage — the case as-is duplicates the list-mix pin under a "set" label.

## Comment 2 (verbatim) — :17263, doc clarity

- (id 3695211435) "The docstring says the existing/pinned map case only covers mixed VALUES, but this
  file already has key-homogeneity coverage for `Map.insert` (e.g., inserting a Bool key into an
  Int64-keyed map). If the intent here is to add *String* as an additional mixed-key variant, it would
  help to state that explicitly to avoid misleading future readers about what is already pinned."

### Liaison verification (confirmed on trunk a2875840b)

The map-key case (:17262) input `(Map.insert (Map.insert Map.empty 1 10) "k" 20)` — a String key into an
Int64-keyed map. Its doc "the pinned map case mixes VALUES; this adds the map-KEY face". Copilot's point:
if the file ALREADY has a Bool-key mixed-insert pin, this String-key one should say it's adding STRING as
another mixed-key variant (not imply key-homogeneity is newly covered). Owner (corpus-bugfix) confirms
whether a prior Bool-key pin exists + clarifies the doc. Doc-only.

Owner: **corpus-bugfix** (`spec/semantics/05-compound-types.sexp`; `a5cacdfa1`). Fix the set case to
exercise the set path (homogeneous + Set.insert-mismatch); clarify the map-key doc vs existing coverage.
