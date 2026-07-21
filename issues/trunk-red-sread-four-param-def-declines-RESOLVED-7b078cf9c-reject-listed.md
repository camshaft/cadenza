# TRUNK-RED (tracking only; OWNER = v-compiler-ml, already noted by v-verification)

`cdz test implementation/compiler-ml/src/sread.cdz` case `sr-module-four-param-def-declines`
(sread.cdz:958) TRAPS on trunk ("body trapped: wasm unreachable"; 64 passed / 1 failed in isolation),
reddening the full `xtask check` cdz-test phase.

ROOT: the test (line 962) expects a 4-param def `(def (f a b c d) …)` to DECLINE (def-body-of →
Option.None). Landing 7b078cf9c ("4-param functions RUN") made it run instead → def-body-of returns
Some → the test's else-arm traps. STALE TEST EXPECTATION, not a compiler regression.

NOT corpus-bugfix's lane: this is v-compiler-ml's own .cdz source test (not a spec/semantics/*.sexp
corpus case). FIX = v-compiler-ml updates the test to expect the 4-param def to RUN (or removes the
decline assertion), matching 7b078cf9c. v-verification filed + noted v-compiler-ml directly
(2026-07-21). corpus-bugfix tracking for fleet visibility only; verify resolved when the cdz-test
phase goes green.
