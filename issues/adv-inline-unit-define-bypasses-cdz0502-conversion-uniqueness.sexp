; BREAKER FINDING 2026-07-17 (trunk d1d09dfcc) — UNITS-LAYER SOUNDNESS HOLE: `Unit.define` used
; INLINE (in a `Qty.of` unit position) bypasses the CDZ0502 name→conversion-uniqueness check that
; the TOP-LEVEL declaration form enforces. A program can silently redefine a BUILT-IN unit's ratio,
; or use the SAME user name at TWO different ratios in one expression — each occurrence just uses
; its own ratio, no diagnostic.
;
; The pinned protections (both still work at TOP LEVEL):
;   (Unit.define #"foot" (Unit.of #"meter") 2 1) …          -> CDZ0502 (18-units:1518 pin) ✓
;   two top-level (Unit.define #"fur" … 201 1 / 999 1)      -> CDZ0502 "declared twice" ✓
; The BYPASS (all verified, each compiles + runs, no diagnostic):
;   (+ (Qty.of a (Unit.define #"foot" (Unit.base #"meter") 2 1))   ; builtin foot REDEFINED 2m
;      (Qty.of a (Unit.base #"meter")))                     a=1 -> 3.0  (fake 2m ratio USED;
;                                                            builtin 381/1250 would give 1.3048)
;   (+ (Qty.of a (Unit.define #"fur" … 201 1))
;      (Qty.of a (Unit.define #"fur" … 999 1)))             a=1 -> 1200.0 (one name, two ratios,
;                                                            both used — 201 + 999)
;   top-level (Unit.define #"fur" … 201 1) + INLINE #"fur" … 999 1 in the same program -> 1200.0
;     (the inline conflicting ratio coexists with the declared one — the uniqueness the top-level
;      check just established is undone by any inline use)
;
; Spec: units-of-measure.md #A Named Unit's Conversion Is Unique — "a unit's name→conversion must be
; a well-defined function" (the 18-units:1505 comment). The inline expression position routes through
; a different evaluation path (unit_of/check_unit_composition consume it as a unit value) that never
; consults or updates the declaration table, so the name→conversion map is only enforced for the
; top-level statement form. This defeats the point of CDZ0502: dimensional-analysis soundness rests
; on one name = one conversion, and `foot`-as-2m is exactly the silent-wrong-physics the layer exists
; to prevent (a 201.168 furlong-mile mix-up is the classic).
;
; FIX direction: the inline `Unit.define` (however it is consumed) must register/check against the
; SAME name→conversion table as the top-level form — agreeing redeclaration admissible, conflicting
; -> CDZ0502, including conflicts with built-ins.
;
; Expected under the fix: this program is REJECTED CDZ0502 (conflicting redeclaration of `foot`).
(case "an inline Unit.define conflicting with a built-in conversion is rejected"
  (doc    "`(Qty.of a (Unit.define #\"foot\" (Unit.base #\"meter\") 2 1))` re-declares the built-in
           `foot` (381/1250 m) as 2 m from an INLINE unit position. The name→conversion-uniqueness
           rule (#A Named Unit's Conversion Is Unique, the CDZ0502 pin at the top-level form) must
           hold wherever the declaration occurs: an inline conflicting define is the same
           ill-formedness, rejected CDZ0502 — not silently evaluated at the fake ratio (currently
           `(+ (Qty.of 1.0 fake-foot) (Qty.of 1.0 meter))` runs to 3.0, using foot=2m).")
  (input  (do
            (def (main (: a Float64))
              (Qty.value (+ (Qty.of a (Unit.define #"foot" (Unit.base #"meter") 2 1))
                            (Qty.of a (Unit.base #"meter")))))
            (export main)))
  (error  CDZ0502))

; ---
; ROUTED to v-inference (corpus-bugfix 2026-07-17, VERIFIED trunk d1d09dfcc: check rc=0, should be
; CDZ0502-rejected). Silent wrong-physics: inline Unit.define in a Qty.of unit position bypasses the
; name->conversion uniqueness table (top-level path rejects; inline unit_of/check_unit_composition
; never consults it). Fix: route inline defines through the SAME table (agreeing redecl OK, conflict
; incl builtins -> CDZ0502). Check-layer, v-inference prelude-unit_families territory. Not spawning
; (fixer cap). Promote when fixed.
