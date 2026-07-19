# mlrepro: recursive list-map reusing an imported boxed sum-payload field emits INVALID wasm

**Reporter:** v-music (found building implementation/music/src/schedule.cdz — the MIDI scheduler).
**Severity:** miscompile — INVALID wasm, `cdz test` aborts the whole suite (not a single case fail):
`could not inspect the test component: invalid component: failed to compile: wasm[0]::function[N]`.
**Workaround exists** (see below), so v-music is UNBLOCKED — but this is a real emit bug to fix.

## Symptom
In a LARGE multi-def module (implementation/music/src/schedule.cdz, ~39 fns, imports `Pitch` from
sibling module `pitch`), a recursive list-map that:
  1. destructures each element of a `List(Note)` where `Note = | Note(Pitch, Int64, Int64, Int64, Int64)`
     (first field is the imported boxed type `Pitch`), and
  2. rebuilds a NEW `Note` REUSING that boxed `Pitch` field DIRECTLY (not through a fn returning a fresh
     Pitch), and
  3. threads the result through a `List.push` accumulator,
emits invalid wasm — the whole test component fails to compile.

## Precise passing/failing delta (bisected inside the module)
- `transpose-piece` (rebuilds `Note.Note(transpose(note-pitch(h), ...), ...)`) → PASSES. `transpose`
  returns a FRESH `Pitch`, so the boxed field is not reused in place.
- `remap-channel` / `shift-piece` (rebuild `Note.Note(note-pitch(h), chan, ...)`, pitch UNCHANGED) →
  FAIL. The matched boxed `Pitch` field is reused directly into the new constructor.
- The SAME recursive-map shape in a small STANDALONE module (no imports, ~12 fns) PASSES — the bug only
  manifests with the module's full function set + the imported `Pitch` type. Minimization past that is
  left to the breaker / v-memory-safety (likely a Perceus reuse-in-place / box-ownership emit bug on the
  `List.push`-accumulator reuse path — cf. the box-by-declared-slot-type traps).

## Workaround (used in schedule.cdz so the slice can land)
Rebuild the pitch FRESH from its note number instead of reusing the boxed field:
  `def clone-pitch(p) = Pitch.Pitch(note(p))`  then  `Note.Note(clone-pitch(note-pitch(h)), ...)`.
Round-tripping through the plain Int64 gives an unshared Pitch and sidesteps the bad emit.

## Repro
The minimal in-module trigger, INSIDE the full schedule.cdz module (which imports `Pitch`):
  def rc-go(notes: List(Note), chan: Int64, out: List(Note)) =
    match notes with
      | [] => out
      | [h, .. t] =>
        match h with | Note.Note(p, _, v, s, d) =>
          rc-go(t, chan, List.push(out, Note.Note(p, chan, v, s, d)))
  called from a `@test`. Swapping `p` -> `Pitch.Pitch(note(p))` makes it pass. schedule.cdz on
  fleet/v-music (branch) carries the workaround + a `clone-pitch` comment pointing here.
