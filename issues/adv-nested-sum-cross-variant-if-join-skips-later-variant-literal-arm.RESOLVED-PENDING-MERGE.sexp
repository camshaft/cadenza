; BREAKER FINDING 2026-07-17 (trunk 5d0ade368 base) — WASM-ONLY DIFFERENTIAL MISCOMPILE (wrong value):
; a match over a NESTED sum (sum-in-sum payload) whose scrutinee is an IF-JOIN of TWO DIFFERENT inner
; variants SKIPS a LITERAL-payload arm of the LATER variant — the value falls through to the binder
; arm. The rust backend computes it correctly (differential).
;
;   (type Inner2 (P Int64) (W Int64))
;   (type Outer2 (Wrap Inner2))
;   (match (if (= sel 0) (Outer2.Wrap (Inner2.P n)) (Outer2.Wrap (Inner2.W n)))
;     ((Outer2.Wrap (Inner2.P 3)) 1000)
;     ((Outer2.Wrap (Inner2.P x)) x)
;     ((Outer2.Wrap (Inner2.W 5)) 2000)     ; <- SKIPPED on wasm
;     ((Outer2.Wrap (Inner2.W y)) y))
;   (sel=1, n=5):  wasm -> 5 (binder arm)   rust -> 2000 (correct)   at O0 AND O2.
;   (sel=0, n=3):  1000 on both (the FIRST variant's literal arm works).
;
; The 2x4 isolation (all verified):
;   DIRECT construction (Outer2.Wrap (Inner2.W n)), same arms          -> 2000 ✓ (both backends)
;   FLAT sum (no Outer wrapper), same cross-variant if-join, same arms -> 2000 ✓
;   NESTED + if-join of the SAME variant on both sides                 -> 2000 ✓
;   NESTED + cross-variant if-join                                     -> 5 ✗ (wasm only)
; Also reproduces with a NARROW (Int8) sibling variant (original find), with the wide-payload arm
; skipped identically — width is NOT required; the minimal trigger is nested-wrapper x cross-variant
; join x literal-arm-on-the-LATER-variant.
;
; Smells like the if-joined-heap-value family (the materialize-once/read-twice regressions pinned at
; 05-compound:666): the join of two Outer2.Wrap values whose INNER variants differ produces a value
; whose inner-variant DISCRIMINANT read (for the nested literal test) is wrong/stale on the wasm path
; — the later variant's literal test never fires, while the binder arm (no discriminant needed beyond
; the outer) works. First-variant literal arms work, so the inner-tag read likely defaults to tag 0.
;
; SEVERITY: silent wrong value on an idiomatic shape (branchy constructors feeding a match).
;
; Expected: (1,5) -> 2000 on both backends (rust's answer).
(case "a cross-variant if-joined nested sum still matches the later variant's literal-payload arm"
  (doc    "`(if c (Wrap (P n)) (Wrap (W n)))` joins two nested-sum values whose INNER variants differ;
           matching the joined value must test the inner discriminant per arm — `(Wrap (W 5))` with
           sel=1,n=5 selects the W-literal arm -> 2000, exactly as the direct-construction and flat-sum
           twins do. Currently the wasm backend skips the LATER variant's literal arm on the joined
           value (falls to the binder arm -> 5) while rust computes 2000 — a silent differential.
           First-variant literal arms (sel=0,n=3 -> 1000) work on both.")
  (input  (do
            (type Inner2 (P Int64) (W Int64))
            (type Outer2 (Wrap Inner2))
            (def (main (: sel Int64) (: n Int64))
              (match (if (= sel 0) (Outer2.Wrap (Inner2.P n)) (Outer2.Wrap (Inner2.W n)))
                ((Outer2.Wrap (Inner2.P 3)) 1000)
                ((Outer2.Wrap (Inner2.P x)) x)
                ((Outer2.Wrap (Inner2.W 5)) 2000)
                ((Outer2.Wrap (Inner2.W y)) y)))
            (export main)))
  (call   main (: 1 Int64) (: 5 Int64))
  (output (: 2000 Int64))
  (call   main (: 0 Int64) (: 3 Int64))
  (output (: 1000 Int64)))

; ---
; ROUTED to v-patterns (corpus-bugfix 2026-07-17, VERIFIED trunk 5d0ade368: wasm -> 5, expected 2000;
; rust correct = wasm-only differential). Nested-sum cross-variant if-join skips the LATER variant's
; LITERAL arm. Trigger: nested-wrapper x cross-variant if-join x literal-on-later-variant (direct
; construction / flat sum / same-variant join all OK; first-variant literals OK). Likely the inner-variant
; discriminant read on the if-joined heap value reads tag 0/stale (05-compound:666 materialize-and-read
; nested-discriminant face). Width not required. v-patterns match-lowering territory. Promote when fixed.

; ROOT-CAUSED + OWNED (v-patterns, 2026-07-17, task #1): NOT the if-join (that just defeats const-fold).
; Minimal = ANY erased single-variant-newtype outer wrapper over a RUNTIME nested sum. ROOT: construction
; correctly erases the outer Wrap (no sum-new), but the match's SumCont::Switch is built with the RAW
; switch_path [Payload] NOT run through erase_nominal_steps (unlike Core::SumPayload which IS erased at
; lower.rs:340) — backend emits sum-payload for the erased-Wrap Payload step, unwraps one level too deep,
; sum-disc reads the box -> 0 -> always first variant. The literal-arm face (2000-not-5) is the same bug
; one level deeper ([Payload,Payload]). FIX: erase_nominal_steps on the switch + lit-test paths at
; construction. v-patterns building next tick + promoting this case to graded corpus (05-compound). Mine.

; SCOPE-WIDENED (breaker, 2026-07-17): MATCH-arm join AND FN-BOUNDARY join (helper returning either
; variant) fail identically (5 not 2000) — NOT if-specific, confirming v-patterns' erased-wrapper root
; (any join/merge producing the nested sum; the tag-0 default is in match-lowering's assumed static inner
; tag, not the join). Record-payload variant join is FINE (needs the nested VARIANT discriminant). Fix
; matrix must cover if/match/fn-return joins — one erase_nominal_steps fix closes all (same construction path).

; SEVERITY UPGRADE + DEPTH-3 (breaker, 2026-07-17): (1) BINDER-ONLY arms also mis-dispatch — inner tag
; is 0 for BOTH variants (W-value dispatches to P arm, 1005). NOT literal-specific; payload survives so
; it is purely the inner DISCRIMINANT. => any branchy nested-sum into a match is wrong on wasm (rust ok).
; (2) At 3 levels (L1 A|B / L2 X|Y L1 / L3 Box L2) tags CROSS levels: (B,X)->Y(A), (A,Y)->X(A) — inner
; tag leaks into middle read => join materialization reads/writes nested discriminants at WRONG OFFSETS,
; not just missing. Fix matrix: {if,match,fn-return} x {literal,binder-only} x {2-level,3-level}.

; PAYLOAD-WIDTH-DEPENDENT (breaker, 2026-07-18): Float32 payloads -> both dispatch to the 2nd (G) arm
; (2000/2000); Int64 -> 1st (P) arm (1005). The stale inner-tag read shifts with payload WIDTH => tag read
; at a payload-width-dependent WRONG OFFSET, not a constant default — clinches crossed-offsets/layout.
; Fix matrix: payload width (Int64/Float32/Int8) x depth (2/3).

; ---
; RESOLVED-PENDING-MERGE (v-patterns, 2026-07-18, MR 2ee3076f6): FIX = a Payload step through a
; Ty::Nominal erased single-variant newtype is now a NO-OP in push_discriminant (was emitting a spurious
; sum-payload -> read the inner disc one level too deep -> always first variant). Covers all faces
; (if/match/fn-return joins, literal + binder-only arms, all payload widths, multi-level) — the minimal
; trigger was the erased-newtype wrapper, not the if-join. Promoted to GRADED corpus case in
; 05-compound-types.sexp ("a match through an erased single-variant newtype dispatches on the inner sum's
; discriminant", both call checks 2000+1000, all 3 baselines) + rcdzc regression test. Retire on land.

; VALIDATED (v-patterns, 2026-07-18): the fix (MR 2ee3076f6) closes the ENTIRE widened matrix — binder-only
; fn-return join, match-arm join, depth-3 (no level-crossing), Float32 + Int8 payloads all PASS. All the
; escalating symptoms (crossed-offsets, width-dependence) were ONE bug: match dispatch walked the erased
; [Payload] as a real sum-payload -> every nested disc read one box too deep (deeper nesting = more levels
; mis-offset = the depth-3 level-crossing; wrongly-unwrapped box's disc-slot at a width-dependent spot =
; the payload-width-dependence). Corpus follow-up (binder-only + depth-3 faces) after the fix lands.
