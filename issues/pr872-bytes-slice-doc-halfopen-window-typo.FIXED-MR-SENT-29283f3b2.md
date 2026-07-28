# PR#872 review comment — 10-bytes.sexp mode-3 window doc typo (corpus-bugfix)

Mirrored from GitHub PR#872 review comment (Copilot), id `3662068483`.
File: `spec/semantics/10-bytes.sexp:1631` — corpus data/doc → corpus-bugfix's lane.

## Comment (verbatim)

- (id 3662068483, 10-bytes.sexp:1631) "The doc string describes `Bytes.slice` windows in half-open
  `[start,end)` form (with `end = start + length`), but mode 3 says `window [3,4)` even though the code
  uses `lo=3` and `ln=4` (so the window is `[3,7)`). This is misleading when reading the pin."

## Liaison verification (confirmed on trunk 2f6928a10)

Case "String.from-bytes over a rope-backed slice accepts aligned windows and rejects a mid-scalar cut".
Doc uses half-open `[start,end)` notation consistently: mode 1 "window [0,3)", mode 2 "window [0,4)".
The code is `(def lo (if (= mode 3) 3 0))` and `(def ln (if (= mode 1) 3 4))`, so mode 3 = `lo=3, ln=4`
→ `Bytes.slice b 3 4` = the four emoji bytes = half-open `[3,7)`. The doc's "mode 3: window [3,4)" is the
odd one out — it wrote `[start, length)` instead of `[start, start+length)`. Should read "window [3,7)".
Behavior-neutral (output pin `3 → 7` i.e. Some, 1 scalar, unchanged); doc-only fix.

Owner: **corpus-bugfix** (`spec/semantics/*.sexp` case doc — corpus lane, per liaison-routing rule; not
v-compiler-ml even if they authored the pin). One-line doc fix.
