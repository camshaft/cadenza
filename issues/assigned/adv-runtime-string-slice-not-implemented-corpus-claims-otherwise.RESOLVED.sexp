; BREAKER FINDING — corpus-accuracy gap (NOT a miscompile; a reject-don't-miscompile decline mislabeled
; as tested-runtime coverage).
;
; spec/semantics/13-strings.sexp lines 563-588 introduce a section "String.slice over a RUNTIME string
; (a parameter, not a literal)" with two cases:
;   571 "a runtime string is sliced by scalar offsets"  — doc: "Feeding `s` as a parameter defeats
;        const-folding, so this exercises the runtime UTF-8 slice walk"
;   580 "a runtime string slice addresses scalar values, not bytes"
; BOTH define `(def (main) (f "hello"))` / `(def (main) (f "aébc"))` — main is NULLARY and passes a
; STRING LITERAL to f. The literal propagates through f and the slice CONST-FOLDS; these cases do NOT
; reach the runtime slice emitter. Proven: the as-written shape returns a value (folds); a shape where the
; string arrives at MAIN's call boundary (genuinely unfoldable) DECLINES:
;   `cdz: error: String.slice on a runtime string is not yet computed (constant strings only)`
;
; So the runtime UTF-8 slice walk the section comment says "the seed MUST emit" is NOT IMPLEMENTED. The
; corpus asserts runtime String.slice is covered when it is actually a decline. Precise behavior map (all
; verified on trunk):
;   String.at   "hello" i    (const string, runtime index)   -> WORKS (lowers the scalar->byte walk)
;   String.slice "hello" a b (const string, runtime indices) -> DECLINES ("not yet computed")
;   String.at   s 1          (runtime-boundary string)       -> DECLINES (String-param boundary-rep limit)
;   String.slice s 1 4       (runtime-boundary string)       -> DECLINES ("not yet computed")
; The crisp asymmetry is the first two: over the SAME constant string, at handles a runtime index, slice
; does not. (A genuinely-runtime STRING declines for both — a separate boundary-representation limit.)
;
; This is HONEST behavior (decline, not a wrong value) — no miscompile. But the corpus gives false
; confidence. Two asks for corpus-bugfix / v-strings owner:
;   (1) Either IMPLEMENT the runtime String.slice walk (the comment's stated intent — String.at already
;       does the scalar->byte walk for a runtime index, so the machinery exists), then cases 571/580 can
;       be rewritten to actually pass the string at main's call boundary and grade `output`; OR
;   (2) if runtime slice is deferred, RE-LABEL 571/580 to reflect that they fold (drop the "defeats const-
;       folding / exercises the runtime walk" claim) and add the `declines` cases below to pin the honest
;       current gap so a future implementation flips them to Fail and prompts migration to `output`.
;
; The `declines` cases below pin the CURRENT honest behavior (they PASS today as declines; they FLIP to
; Fail the moment runtime slice is implemented and emits a value — the trigger to migrate them to output).

(case "adv strings: String.slice with runtime indices over a constant string declines (not yet computed)"
  (doc "A constant string but RUNTIME slice indices (a, b at the call boundary) cannot constant-fold the
        slice, so it declines: `String.slice on a runtime string is not yet computed (constant strings
        only)`. Contrast String.at, which lowers a runtime index fine. Honest reject-don't-miscompile;
        flips to Fail (migrate to output) when the runtime slice walk lands.")
  (input (do (def (main (: a Int64) (: b Int64))
               (String.byte-len (Option.expect (String.slice "hello" a b) "in range")))
             (export main)))
  (call main (: 1 Int64) (: 4 Int64))
  (declines))

(case "adv strings: String.slice on a string arriving at the call boundary declines (the truly-runtime slice)"
  (doc "The genuinely-runtime string slice the 13-strings runtime section CLAIMS to test but does not:
        s arrives at main's call boundary (unfoldable), so `(String.slice s 1 4)` declines with
        `String.slice on a runtime string is not yet computed`. The as-written cases 571/580 pass a
        literal through a nullary main, so they fold and never reach this path.")
  (input (do (def (f s) (Option.expect (String.slice s 1 4) "in range"))
             (def (main (: s String)) (String.byte-len (f s)))
             (export main)))
  (call main (: "hello" String))
  (declines))

(case "adv strings: String.at with a runtime index over a CONSTANT string works (the asymmetry)"
  (doc "The crisp contrast that makes the slice gap a genuine asymmetry: over the SAME constant string
        \"hello\", String.at with a RUNTIME index lowers and runs — scalar index 1 is \"e\", one byte — but
        String.slice with runtime indices (the declines case above) does not. So the scalar->byte runtime
        walk EXISTS for at's index; slice just doesn't handle a runtime index yet. (A genuinely-runtime
        STRING at the call boundary declines for BOTH at and slice — the String-param boundary-rep limit —
        so this pins the at-vs-slice split at the one axis where they diverge: a runtime INDEX over a
        constant string.)")
  (input (do (def (main (: i Int64))
               (String.byte-len (Option.expect (String.at "hello" i) "in range")))
             (export main)))
  (call main (: 1 Int64))
  (output (: 1 Int64)))

## UPDATE 2026-07-16 (corpus-bugfix): v-runtime IMPLEMENTED runtime String.slice (option 1, Core::StrSlice, MR 99a345d0f) — PENDING pr-sync (NOT on trunk; the StrSlice refs on trunk are the Prim enum + the DECLINE stub lower.rs:19011).
MY CORPUS-MIGRATION WORK IS HELD until 99a345d0f LANDS, then: (1) rewrite 571/580 to a GENUINELY-runtime slice (verify it hits the runtime emit, NOT a const-fold — I confirmed String.concat-with-empty-literal FOLDS AWAY, so the concat trick alone does NOT force runtime; need a form the folder truly cannot see through, e.g. a match/if-selected string or v-runtime's verified shape); (2) migrate breaker's 2 declines cases to output with v-runtime's values (café slice 1 4 -> afé, etc); (3) keep the at-vs-slice asymmetry case. Do NOT rewrite before the MR lands (a premature rewrite re-words the false claim as a still-folding pass — verified this pitfall this tick).
