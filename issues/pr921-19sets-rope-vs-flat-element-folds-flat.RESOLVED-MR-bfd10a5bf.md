# PR#921 review comment — 19-sets rope-vs-flat element case folds the "rope" operand flat (corpus-bugfix)

Mirrored from GitHub PR#921 review comment (Copilot), id `3684464079`.
File: `spec/semantics/19-sets.sexp:2846` — corpus doc/coverage → corpus-bugfix. Blame `61b7bab50`
"corpus(sets): 3-pin drain S — … rope-vs-flat algebra elements …".

⚠ Same class as PR#874 (constant `String.concat` folds to flat `ConstStr`, defeating rope coverage) —
which corpus-bugfix already fixed there; this is another instance.

## Comment (verbatim)

- (id 3684464079, 19-sets.sexp:2846) "This case claims to cover rope-vs-flat string element unification,
  but `(String.concat \"ap\" \"ple\")` will be constant-folded to a `ConstStr` because both operands are
  constant ASCII. That means both operands likely contain a flat \"apple\", so the pin won't actually
  exercise cross-operand canonicalization between a runtime rope and a flat string. Make the \"apple\"
  construction runtime-dependent (while still evaluating to \"apple\" for k=1) so it lowers through the
  runtime `bytes-concat` rope path."

## Liaison verification (confirmed on trunk 5dfc74b9e)

Case "set algebra unifies a rope String element with its flat twin across operands". `a` is built with
`(String.concat "ap" "ple")` — BOTH operands constant ASCII → constant-folds to a flat `ConstStr "apple"`
(per `resolved.rs` Prim::StrConcat fold, the PR#874 mechanism). So the intended ROPE element (`a`'s
apple) is actually FLAT, and the case's claimed "rope-apple vs flat-apple cross-operand canonicalize" is
not exercised — both apples are flat. (Note `b`'s cherry DOES use a runtime `(if (= k 1) "rry" "z")` so
IT ropes, but the pin's headline is the apple rope-vs-flat, which folds.) The output pin 311 still holds
(folds to the right content), so the test passes but under-covers. Fix (Copilot's, sound + matches the
PR#874 remedy): make `a`'s "apple" construction runtime-dependent (e.g. concat a mode-derived-but-content-
fixed piece, or gate an operand behind `k`) so it lowers through the runtime bytes-concat rope path,
while still evaluating to "apple" at k=1.

Owner: **corpus-bugfix** (`spec/semantics/19-sets.sexp`; `61b7bab50`). Force `a`'s apple to a runtime
rope. (Cross-ref the PR#874 fix — same constant-concat-folds-flat class corpus-bugfix already handled for
19-sets/17-symbols.)
