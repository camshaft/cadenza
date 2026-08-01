# PR#970 review comment — cad relative-path cursor-bug comment asserts one cap value for two distinct bugs (v-cad)

Mirrored from GitHub PR#970 review comment (Copilot), id `3694473079` (:699, also :707).
File: `implementation/cad/src/exact.cdz` — v-cad. Blame `c121dc720` "cad: pin relative-path cursor
advance + v2abs negative-reach (mutation-found gaps)".

## Comment (verbatim)

- (id 3694473079, exact.cdz:699) "The explanation here says either a LineToRel or CubicToRel raw-delta
  cursor-advance bug would cap the max-x at 25, but with this fold (`path-seg-cursor` feeds later
  segments while `path-seg-reach` uses the current cursor), a LineToRel cursor bug would leave the max-x
  at 20, while a CubicToRel cursor bug would leave it at 25. Consider rewording so it doesn't assert a
  single incorrect capped value. This issue also appears on line 707 of the same file."

## Liaison verification (confirmed on trunk 2447e5529)

The comment (exact.cdz:697-699): "a LineToRel-raw bug makes the 2nd advance 10 (not 20) and a
CubicToRel-raw bug makes the cubic advance 5 (not 25) — **either caps the max-x at 25, not 35**." The
chain is start→line(10)→cur=10→line(10)→cur=20→cubic end(5)→cur=25→line(10)→cur=35. Copilot's point: the
two bugs cap at DIFFERENT values — a LineToRel raw-delta bug (2nd advance = raw 10 not cursor 20) shifts
the whole downstream so the final reach caps at ~20, while a CubicToRel raw-delta bug (cubic advance 5 not
25) caps at ~25. So "either caps at 25" collapses two distinct failure values into one incorrect number.
The TEST (hx==35) is correct and pins both; only the EXPLANATORY comment's single "25" is imprecise. Fix:
reword to state the two distinct caps (LineToRel bug → 20, CubicToRel bug → 25), or just "< 35" without
asserting a specific value. Comment-only, behavior-neutral, pin correct. (:707 sibling same-class.)

Owner: **v-cad** (`implementation/cad/src/exact.cdz`; `c121dc720`). Reword the cursor-bug cap explanation
(+ :707) to not assert a single value for two distinct bugs.
