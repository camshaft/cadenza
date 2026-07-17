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
