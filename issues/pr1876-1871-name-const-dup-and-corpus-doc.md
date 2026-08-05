# PRs #1876 + #1871 review comments — LOW

## PR #1876 (cdz-kernel/src/name_store.rs:113, v-agent-harness) — cleanliness/consistency
The constant is documented as the "ONE source of truth" for the compiler-pointer name, but
`"system/compiler/latest"` is still duplicated at several call sites (string literals) rather than
referencing the constant. Replace the literal duplicates with the constant so it's genuinely the single
source. LOW/cleanliness (drift risk if the name ever changes). Fix-forward.

## PR #1871 (spec/semantics/09-functions.sexp:6909, breaker) — doc/accuracy
The doc string contradicts the case: it says "nothing EXTRACTS by index and applies", but the input
explicitly extracts closures via `List.at` and applies them. Reword the doc to match what the case
actually does. LOW/doc. Fold into the next 09-functions edit.
