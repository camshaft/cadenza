; BREAKER FINDING 2026-07-20 (trunk 5ef3d9ea4) — FRONTEND (both backends identically): resolution of a
; bare constructor whose name EQUALS its type's name is POSITION-DEPENDENT. The same spelling compiles
; in main's immediate body but is rejected CDZ0203 ("`Meters` is a type that takes no type parameters")
; when it sits inside a CALLED helper def, inside a let-bound lambda in main, or when the sum has more
; than one variant. One of the two behaviors must be wrong:
;
;   (type Meters (Meters Int64))
;   (Meters a) direct in main                       -> compiles, runs           ✓
;   (def (mk a) (Meters a)) + main calls mk         -> CDZ0203 at the ctor site ✗
;   same helper but NEVER CALLED                    -> compiles                 ✓ (resolution is lazy —
;                                                        the hijack fires at instantiation)
;   same helper, annotated (: a Int64)              -> CDZ0203                  ✗
;   let-bound (fn (b) (Meters b)) inside main       -> CDZ0203                  ✗
;   qualified Meters.Meters in the helper           -> compiles                 ✓
;   multi-variant (type N (N Int64) (J Int64)),
;     bare (N a) even DIRECT in main                -> CDZ0203                  ✗
;
; The bare same-name spelling is SANCTIONED: the landed corpus pin 05-compound-types.sexp:11708
; ("a transitive chain of erasable newtypes in reversed declaration order composes") constructs
; (A (B (C 60) 2) 3) with bare same-name ctors A/B/C in main and passes. So the helper-position
; CDZ0203 is a spurious reject of a valid program — the TYPE binding wins over the CONSTRUCTOR
; binding when the def body is resolved at its call-site instantiation, but the ctor wins in main's
; body. (Or, if the type is SUPPOSED to win, the main-body form and the landed pin are the bug.)
; Diagnostic is also misleading: it tells the author to write an annotation when they wrote a ctor.
;
; Both backends reject identically (shared frontend); rust gate agrees case-for-case with wasm.
; SEVERITY: not a miscompile — a spurious compile-time reject + resolution inconsistency on an
; idiomatic shape (a smart-constructor helper for a newtype is the FIRST thing a user writes).
;
; Expected: both cases below compile and run on both backends (main-body behavior is the reference).
(case "a same-name newtype constructor in a called helper resolves to the constructor, not the type"
  (doc    "`(type Meters (Meters Int64))` gives the constructor the type's name. `(def (mk a) (Meters
           a))` uses the bare constructor in a helper main calls; it must resolve to the CONSTRUCTOR
           (as it does written directly in main's body, and as the transitive-erasure corpus pin's
           bare A/B/C constructions do) — not be hijacked by the type binding into a CDZ0203
           'takes no type parameters' over-application reject at the instantiation of mk.")
  (input  (do
            (type Meters (Meters Int64))
            (def (mk a) (Meters a))
            (def (main (: a Int64))
              (match (mk a) ((Meters v) (+ v 1))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 5 Int64)))

(case "a same-name variant of a multi-variant sum constructs bare in main like a single-variant one does"
  (doc    "`(type N (N Int64) (J Int64))` — the first variant shares the type's name. Bare `(N a)`
           DIRECT in main is rejected CDZ0203, while the single-variant twin `(type Meters (Meters
           Int64))` bare `(Meters a)` in the same position compiles. Variant-count must not change
           whether a constructor name is visible; the qualified `N.N` works either way.")
  (input  (do
            (type N (N Int64) (J Int64))
            (def (main (: a Int64))
              (match (N a) ((N v) (+ v 1)) ((J w) w)))
            (export main)))
  (call   main (: 4 Int64)) (output (: 5 Int64)))

; ===== OWNER UPDATE (v-inference, 2026-07-20) — TWO DISTINCT FACES =====
; FACE A (multi-variant DIRECT construct, case 2 here): FIXED in commit e9d085454 — same_name_newtype_ctor_index
;   in db.rs was wrongly restricted to variants.len()==1; generalized to any variant whose name==type name.
;   rcdzc 2207/0, gate 4037/0, +unit test, all 17 same-name tests green. HELD behind v-inference's queued MR.
; FACE B (same-name ctor in a CALLED HELPER, case 1 here): DISTINCT root — at instantiation the def body is
;   β-copied to a SYNTH node (id >= user_node_count), so resolve's head-position rule (child_ix==0 &&
;   is_user_node) fails -> type wins -> CDZ0203. Explains 'never-called compiles / called rejects'. Can't just
;   drop the is_user_node gate (protects the generic-sum (Box a) synth type-expr). Needs a discriminator that
;   survives the β-copy — a dedicated resolve slice. REMAINS OPEN.
; TRACKING (corpus-bugfix): when e9d085454 lands -> PIN case 2 (FACE A, multi-variant direct) as PASSING to
;   corpus; keep case 1 (FACE B, helper) as a (declines) pin until v-inference lands B. Await v-inference ping.

; ===== OWNER UPDATE 2 (v-inference, 2026-07-20) — FACE B FIXED + MR'd =====
; FACE B (same-name ctor in a CALLED helper) now FIXED + MR sent b42821408 (awaiting merge). Root: β-copied
; helper body (Meters a) is a synth node in value-head position skipped by is_user_node → CDZ0203; fix fires
; the head-position ctor rule on a synth node too WHEN the same-name sum is MONOMORPHIC (generic stays gated).
; Fixes single- AND multi-variant monomorphic helpers.
; CORPUS BATTERY (corpus-bugfix to pin once b42821408 lands): {mono direct A, mono helper B (single+multi)} =
;   PASS; {GENERIC same-name via helper: type Box (Box a) + def mk(x)=Box(x)} = (declines) CDZ0203 pin
;   (harder discriminator, NOT a miscompile — the DIRECT generic construct works).
; VERIFIED on trunk 7a065bbf7 (corpus-bugfix): FACE A -> PASS (value 5); FACE B -> STILL CDZ0203 (b42821408
;   not yet landed); generic-via-helper -> declines CDZ0203 (as expected). => WAIT for b42821408 to land, THEN
;   pin the full battery in ONE corpus commit (all 3 baselines). Do NOT pin FACE B now (would gate-fail).
