; DIAGNOSTIC GAP (v-verification, 2026-07-16) — a withheld-constructor MATCH outside its module gives the
; WRONG diagnostic code for a SINGLE-VARIANT sum: CDZ0203 instead of CDZ0214.
;
; SOUND either way (the match IS rejected, so abstract-type opacity/unforgeability HOLDS — you cannot
; destructure an abstract value outside its module). This is a DIAGNOSTIC-CODE gap, not a soundness hole:
; the spec (modules-and-namespaces.md §A Type's Handle And Its Constructors Are Independently Visible;
; 11-modules.sexp:861) says a construction OR a match through a withheld constructor MUST carry the
; withheld-constructor code CDZ0214. Construction always does. A MATCH does for a MULTI-variant sum, but a
; SINGLE-VARIANT sum's match falls into CDZ0203 (a type/annotation-mismatch path) instead.
;
; ISOLATION (all four verified on trunk 88f15533d via `cargo xtask gate`):
;   CONSTRUCT single-variant withheld  (C.A 9) outside          → CDZ0214  ✅ correct
;   MATCH     multi-variant  withheld  (match .. (C.A n)(C.B m)) → CDZ0214  ✅ correct
;   MATCH     single-variant withheld  (match .. (C.A n))        → CDZ0203  ❌ should be CDZ0214
;   MATCH     single-variant NULLARY   (match .. (C.A))          → CDZ0203  ❌ should be CDZ0214
; So the discriminator is SINGLE-VARIANT (arity of the payload does not matter; a two-variant sum matched
; over both arms is correctly CDZ0214). A single-variant match is likely routed as an irrefutable /
; exhaustive destructuring and hits a type/binding check (CDZ0203) BEFORE the per-arm withheld-ctor
; visibility gate that multi-variant matches reach.
;
; WHY IT MATTERS TO v-verification: the HOL kernel's `Thm`/`Term`/`Hty` are newtype-style SINGLE-VARIANT
; sums (e.g. `(type Thm (MkThm <sequent>))`). An importer matching `Thm.MkThm` to read the sequent out of
; a theorem is correctly REJECTED (unforgeability holds), but with the wrong code — so a machine-actionable
; "this constructor is withheld on purpose, use the exported accessor" message is not emitted for exactly
; the type shape the kernel uses. Once fixed, v-verification pins the match-outside case as CDZ0214.
;
; SEAM: the match-arm withheld-ctor visibility check (resolve.rs `withheld_ctor_reject` /
; `is_abstract_type_at`) vs the single-variant match lowering (v-patterns' irrefutable/single-arm path).
; Likely v-patterns + resolve co-owned. Decline-don't-miscompile is intact; this is purely the code.

(case "a single-variant abstract type's constructor match outside its module is rejected (currently CDZ0203, spec wants CDZ0214)"
  (doc    "SOUNDNESS holds (the match is rejected — an abstract single-variant type cannot be destructured
           outside its module), but the DIAGNOSTIC CODE is wrong: a withheld-constructor MATCH should be
           CDZ0214 (the withheld-constructor code) exactly as construction is and as a multi-variant match
           is, per modules-and-namespaces.md. A single-variant sum's match currently rejects CDZ0203. `lib`
           exports the abstract handle `C` + smart ctor `mk` but not `C`'s variant ctor; the entry matches
           `C.A` outside → rejected. This is the exact shape a HOL-kernel `Thm` newtype uses. Fix routes
           the single-variant match through the same withheld-ctor gate the multi-variant path already hits.")
  (module "lib"
    (do (type C (A Int64)) (def (mk) (C.A 5)) (export C) (export mk)))
  (input  (do (import "lib" (C mk)) (def (main) (match (mk) ((C.A n) n))) (export main)))
  (error  CDZ0214))
