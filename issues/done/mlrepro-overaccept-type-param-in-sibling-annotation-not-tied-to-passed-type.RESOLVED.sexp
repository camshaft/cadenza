;; ANALYSIS SHARPENED 2026-07-18 (v-inference, _w57): SPEC-MANDATED, not a judgment call.
;;   type-system.md line 60: "A position that binds a type-valued parameter MUST be a bidirectional-
;;   checking boundary, at which a type is ... CHECKED against an explicit annotation, RATHER THAN SOLVED
;;   BY UNIFICATION." So f(Bool, 41) for (def (f (: t Type) (: x t)) x) MUST REJECT (Bool binds the var,
;;   41 is checked against it) — the current accept (41's Int64 solves the var by unification) VIOLATES
;;   the spec. This is a CONFORMANCE fix; no operator design ack needed.
;;   EXACT BOUNDARY (re-verified): g(x: a, y: a) called g(1, true) REJECTS (2 errors) — two siblings share
;;   a bare var, unification catches the conflict. But f(t: Type, x: t) called f(Bool, 41) ACCEPTS (0
;;   errors) — the Type-param slot and the sibling var are DISCONNECTED in the scheme (-> Type (-> a a)):
;;   the first slot is the KIND Type (accepts any type value), 'a' is a separate var only the sibling uses,
;;   and the passed Bool never binds 'a'. pair(Int64,1,true) already REJECTS correctly (both x AND y use
;;   the SAME var, so x=1 pins it) — and its message now renders "Int64" not "_" (the cosmetic twin was
;;   fixed earlier by the subst.apply diagnostic work).
;;   FIX: at a call binding a (: t Type) param to a concrete type VALUE, UNIFY that reflected type value
;;   into the scheme var the sibling (: x t) uses — making the type-param slot a checking boundary. Locus:
;;   compute_def_scheme (infer.rs ~3989) must record the t-binder -> sibling-var linkage (today Type and
;;   'a' are disconnected slots), and apply_scheme_to_args (~4875) / the collect step-1 unify (~9461) must
;;   bind the reflected arg value to that var BEFORE unifying the sibling args. Deep but well-scoped +
;;   spec-mandated. READY TO IMPLEMENT (my next dedicated unit).
;;
;; UPDATE 2026-07-18 (v-inference, _w57): NOW USER-FACING via the landed forall-binder sugar (breaker).
;;   `def id(x: forall a. a) = x; id(Bool, 41)` checks rc=0 + runs 42 — the explicit Bool type arg is DEAD
;;   (same root as below; forall desugars to (: a Type), so the type arg is a first-class user surface).
;;   Scheme (cdz type): `id` = (-> Type (-> a a)); `len` = (-> Type (-> (Lst a) Int64)). The FIRST slot is
;;   the KIND `Type`; the passed type VALUE unifies with the kind then is DISCARDED; the sibling var `a`
;;   pins FRESH from the VALUE arg. Fix locus (re-pinned): compute_def_scheme (infer.rs ~3989 — the
;;   (: t Type) param types Ty::Type, a SEPARATE slot from the Ty::Var(a) the sibling `(Lst t)`/`(: x t)`
;;   produces; they must share a var) + apply_scheme_to_args (infer.rs ~4875) + the fault path — so the
;;   passed type VALUE binds that shared var before sibling args unify. Still LOW severity (over-accept,
;;   never a miscompile). NEXT DEDICATED v-inference unit (sequenced after forall, which it interacts with).
;;   Interacts w/ type-system.md §228 (bidirectional-checking boundary) — may want an operator design ack.
;;
;; OVER-ACCEPTANCE (low severity — a MISSED early rejection, NOT a miscompile), v-inference, 2026-07-17.
;;
;; When a `(: t Type)` type-valued parameter is MENTIONED in a SIBLING parameter's annotation as a type
;; constructor argument — `(: l (Lst t))`, `(: g (-> t Int64))`, etc. — the annotation does NOT tie that
;; position to the CONCRETE type VALUE passed for `t` at the call. So a call that passes `t = Int64` but a
;; `Lst Bool` argument is ACCEPTED with no error, even though a DIRECT `(: l (Lst Int64))` annotation
;; correctly rejects the same `Lst Bool` arg ("its payload should be Int64, but this one is Bool").
;;
;; ROOT (via `cdz type f`): `(def (f (: t Type) (: l (Lst t))) l)` gets the scheme
;;     (-> Type (-> (Lst a) (Lst a)))
;; The `(Lst t)` annotation reduces `t` to a QUANTIFIED var `a` (correct for a generic signature), but the
;; scheme does not RELATE that `a` to the VALUE of the first `Type` argument. At a call, `a` instantiates
;; FRESH and unifies with the arg's element (`Lst Bool` → a=Bool), independently of the `Int64` passed for
;; `t`. The `Type`-valued first argument and the `a` in `(Lst a)` are DISCONNECTED. A concrete `(Lst Int64)`
;; annotation has no such var, so it constrains directly — hence the asymmetry.
;;
;; SEVERITY = LOW (over-accept, not unsound). The mismatch is caught the moment an element is USED at a
;; conflicting concrete type: `(def (firstOr (: t Type) (: d t) (: l (Lst t))) (match l ((Lst.Nil) d)
;; ((Lst.Cons h _) h)))` called `(firstOr Int64 0 (Lst.Cons true Lst.Nil))` DOES reject ("match arms differ:
;; Int64 vs Bool") and does not emit wasm. Only a program that NEVER uses the mismatched element at a
;; conflicting type (a pure `len`/`identity` over the list) slips through — it then monomorphizes at the
;; arg's real element type, so no wrong value is produced. So: a missed EARLY diagnostic, not a miscompile.
;;
;; SAME-ROOT COSMETIC SYMPTOM: `(def (pair (: t Type) (: x t) (: y t)) x)` called `(pair Int64 1 true)`
;; DOES reject `y` (Bool), but the message reads "a value of type `_` is expected here" — the `_` is the
;; still-unresolved var for `t`'s position, NOT the `Int64` that `t`+`x` already pinned. Same disconnect:
;; the sibling `t`-position isn't linked to the solved value, so it renders as an unresolved hole. A proper
;; fix (tying the passed/solved type into every `t`-mentioning annotation) resolves BOTH the over-accept
;; above AND this `_` render at once — so do NOT band-aid the render alone.
;;
;; FIX SKETCH (deferred — deep + careful): when a def has a `(: t Type)` param AND a sibling annotation
;; mentioning `t`, the call-site monomorphization should SUBSTITUTE the passed type VALUE for `t` into the
;; sibling annotation BEFORE unifying the argument (bind the quantified `a` to the passed `Int64`), so
;; `(Lst t)` at `t=Int64` checks the arg exactly as `(Lst Int64)` does. Likely in the type-valued-param
;; arg-check / `type_specialize` seam (infer.rs / lower.rs). Interacts with the greenlit forall-binder work
;; (both are about a type-var's value flowing into an annotation) — sequence after that lands.
;;
;; The two repro defs below both PASS `cdz check` today (rc=0) but SHOULD reject the arg (Lst Bool vs t=Int64):

(module m
  (type Lst Nil (Cons a (Lst a)))
  ;; `len` only COUNTS — never uses an element at a concrete type — so the Lst Bool arg slips past t=Int64.
  (def (len (: t Type) (: l (Lst t)))
    (match l ((Lst.Nil) 0) ((Lst.Cons h tl) (+ 1 (len t tl)))))
  (def (main) (len Int64 (Lst.Cons true (Lst.Cons false Lst.Nil))))
  (export main))

; ---
; RESOLVED-PENDING-MERGE (v-inference, 2026-07-18, MR 119db7522): root exactly as the queue analysis — the
; (: t Type) param slot and the sibling var were disconnected, so the passed type value never bound the var
; a sibling (: x t)/(Lst t) used. FIX ties them at the call's arg-check: unify Ty::Var(param_occ.0) :=
; reflected type-value BEFORE sibling args unify (spec-mandated type-system.md L60). Both repro defs (len
; over (Lst Bool) at t=Int64) now REJECT; correct-type calls + monomorphization unaffected. The '_'-render
; cosmetic twin was already fixed (subst.apply). 2104/2104 pass. Retire on land.

; LANDED + CONTENT-VERIFIED (corpus-bugfix 2026-07-18, trunk 57ac76c53): 119db7522 on trunk. (f Int64 (list true false)) with (: l (List t)) now correctly REJECTS CDZ0203 ("argument for l is a (List Bool), but (List Int64) is expected — elements should be Int64, but these are Bool"). The (: t Type) param now ties the sibling (List t) annotation to the passed type value. Fully resolved.
