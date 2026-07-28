# PR#877 review comment — 19-sets.sexp triple-rep miss-row doc digits wrong (corpus-bugfix)

Mirrored from GitHub PR#877 (OPEN staging batch) review comment (Copilot), id `3665418008`.
File: `spec/semantics/19-sets.sexp:2754` — corpus doc → corpus-bugfix's lane.

## Comment (verbatim)

- (id 3665418008, 19-sets.sexp:2754) "The docstring's 'miss row' digits don't match the program's
  behavior for mode=1. With rope=\"kez\", only `(Map.lookup m view)` succeeds; both set probes should be
  false, so the output is 0100 → 100 (as asserted), not 0111 → 111."

## Liaison verification (confirmed on trunk ec6fba606)

Case "one string content hashes identically from flat, rope, and view reps in one program". mode=1:
`rope = String.concat "ke" "z" = "kez"`, `view = String.slice "xkeyz" 1 4 = "key"`,
`m = {"key"→42}`, `s = Set.of (list "kez")`. The four digits:
- `1000 * (Map.lookup m rope)`: "kez" not in m → 0
- `100  * (Map.lookup m view)`: "key" in m → 1
- `10   * (Set.contains s view)`: "key" in {"kez"} → 0
- `1    * (Set.contains s "key")`: "key" in {"kez"} → 0

⇒ **0100 = 100**, exactly the asserted pin `(call main (: 1 Int64)) (output (: 100 Int64))`. But the doc
says "Adds a miss row (mode 1 probes with rope \"kez\"): 0111 → 111." — the "0111 → 111" digits are
wrong; should read "0100 → 100". Doc-only contradiction (pin + behavior are correct; only the doc's
worked digits are off). Behavior-neutral fix.

Owner: **corpus-bugfix** (`spec/semantics/*.sexp` case doc). One-line doc digit fix.
