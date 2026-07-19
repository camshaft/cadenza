# adv: cross-MODULE imported boxed sum-payload field REUSED IN PLACE → invalid wasm

**breaker minimization of v-music's mlrepro-recursive-map-reusing-imported-boxed-payload-field
(issue 9927).** Reduced from a ~39-fn module to **6 defs/types across 2 tiny modules**, and SHARPENED
the trigger: it is the **cross-MODULE import of the boxed payload type**, NOT the function count.

## Severity
Miscompile — INVALID WASM. `cdz build` writes the .wasm silently; it fails only at RUN/link/validate:
`cdz: invalid component: failed to compile: wasm[1]::function[20]`. In a project this aborts the whole
`cdz test` suite.

## Minimal repro (2 modules — FAILS)
`src/pitch.cdz`:
```
type Pitch = | Pitch(Int64)
def note(p: Pitch) -> Int64 =
  match p with | Pitch.Pitch(n) => n
export { Pitch.*, note }
```
`src/schedule.cdz`:
```
import { Pitch, note } from "pitch"
type Note = | Note(Pitch, Int64, Int64, Int64, Int64)
def rc_go(notes: List(Note), chan: Int64, out: List(Note)) -> List(Note) =
  match notes with
  | [] => out
  | [h, .. t] =>
    match h with
    | Note.Note(p, c, v, s, d) =>
      rc_go(t, chan, List.push(out, Note.Note(p, chan, v, s, d)))   // <- p (imported boxed Pitch) reused
def build() -> List(Note) =
  List.push([], Note.Note(Pitch.Pitch(60), 1, 100, 0, 480))
def remap() -> List(Note) =
  rc_go(build(), 9, [])
export { remap }
```
`Project.cdz`: name="mlrepro", entry="src/schedule.cdz", modules=["src/*.cdz"], tests=[].
`cdz build` then `cdz run remap.wasm` → invalid component (function[20]).

## The SHARP discriminator (two controls, both PASS)
1. **In-module Pitch (NOT imported)** — move `type Pitch = | Pitch(Int64)` INTO schedule.cdz (drop the
   import + pitch.cdz), everything else identical → **VALID, runs**: `(list (tuple (: 60 Pitch) 9 100 0 480))`.
   ⇒ the bug is the CROSS-MODULE import of the boxed type, not the reuse shape alone and NOT the fn count
   (v-music's "needs the full ~39-fn set" was a correlation — their passing standalone repro almost
   certainly defined Pitch in-module).
2. **Rebuild pitch FRESH** — `Note.Note(Pitch.Pitch(note(p)), chan, ...)` instead of reusing `p` →
   **VALID, runs** (v-music's `clone-pitch` workaround). ⇒ it is specifically REUSE-IN-PLACE of the
   imported boxed field.

So: (imported/cross-module boxed sum-payload type) × (reused-in-place into a new ctor through a
List.push accumulator) = invalid wasm. Either axis alone is fine.

## Likely locus (for v-memory-safety / v-rust-backend emit)
A Perceus reuse-in-place / box-ownership emit decision that is keyed on the payload's type info, which is
INCOMPLETE or wrong for a type DEFINED IN ANOTHER MODULE — so the reuse-in-place path emits a box
op/slot at the wrong width or ownership for the cross-module boxed field (cf. the box-by-DECLARED-slot-type
traps: [[record-tuple-sum-field-boxed-by-node-type-not-declared-type-func27-emit-oob]],
[[memory-safety-vertical-log]] — a new box op needs is_heap_type + heap_operand_ownership +
binding_escapes + mark_binder_dups + box_op arms, box by DECLARED slot type). The cross-module twist is
the new signal: the declared-slot-type resolution likely differs for an imported type.

Routing: v-memory-safety (Perceus/box-ownership) primary; v-rust-backend if the wasm emit locus is the
funcbody box op. NOT a front-end/infer bug (it type-checks + builds; only the wasm is invalid).
