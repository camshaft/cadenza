# pr650 — 3 doc/message nits: music trap-msg + arpeggiate "fills a bar" + corpus stray paren (3 Copilot)

Mirrored from GitHub PR #650 review comments (Copilot). All VERIFIED against `git show trunk`.
PR: https://github.com/camshaft/cadenza/pull/650 (3-MR batch)

## #1 — id 3611378102 (music/src/schedule.cdz:304) — misleading trap message [v-music]
> This trap message fires when the *second* event is still an "on"; the current string says "second is the
> off", which is misleading when debugging a failure.

VERIFIED: `if ev-on(second) then trap("second is the off") else unit` — the trap is in the `ev-on(second)`
TRUE branch (second IS an on), but says "second is the off". Should be e.g. "second is NOT the off (still an
on)". Real trap-message bug (misleads on failure).

## #2 — id 3611378105 (music/src/piece.cdz:26) — arpeggiate "fills a bar"/"four per bar" overstates [v-music]
> These doc comments imply the accompaniment "fills a bar" / "four per bar", but `arpeggiate` emits one note
> per chord tone (triad => 3 notes), leaving the last beat silent. Clarify the comments to match.

VERIFIED: `step = 960` doc says "four per bar"; `accompaniment-bar` doc says "filling a bar"; `bar = 3840`
(= 4 steps). But `accompaniment-bar = arpeggiate(nums, 0, 90, start, step, step)` emits one note per chord
tone — a triad (3 notes) = 3 quarters = 2880 ticks, leaving the 4th beat (960) silent. So "fills a bar"/"four
per bar" is wrong for a triad. Clarify docs to "one note per chord tone (a triad → 3 of the 4 beats)".

## #3 — id 3611378097 (spec/semantics/05-compound-types.sexp:8734) — stray paren in doc string [corpus, PM]
> Doc snippet has an extra closing parenthesis inside the backticked expression: `List.len xs)` should be
> `List.len xs` to avoid confusing readers.

VERIFIED: the `List.prepend` persistence case doc has "...the original `List.len xs)` = 2..." — a stray `)`.
Trivial doc typo in a corpus case (not the executable sexpr, just the `(doc ...)` prose). → PM to place
(compound-types corpus case; v-patterns or whoever owns 05-compound-types).

## Owner
#1, #2 → v-music. #3 → PM (corpus doc typo). All doc/message only, no behavior change.


---
## PM triage (corpus-bugfix, 2026-07-20)
- #3 (corpus stray paren, 05-compound-types.sexp:8807) FIXED by corpus-bugfix directly (doc-only); MR sent to pr-sync.
- #1/#2 (v-music) routed to v-music via note.
