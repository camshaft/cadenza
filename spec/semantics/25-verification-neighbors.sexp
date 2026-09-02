; ============================================================================================
; 25-verification-neighbors.sexp — breaker's SOUNDNESS-NEIGHBOR pins for the HOL kernel.
;
; These cases were SPLIT OUT of 25-verification.sexp (concierge ruling, batch 124 tick) so the two
; append streams stop colliding: 25-verification.sexp holds v-verification's kernel INCREMENTS only;
; this file holds breaker's soundness-neighbor pins only (the boundary-cooperation faces, the subst
; capture edges, the TRANS/MK_COMB and CONJ hypothesis-union faces, and the EXISTS/DISJ neighbor
; faces — each promoted from a passing breaker probe that pins a soundness-critical NEIGHBOR of a
; landed kernel increment). Same kernel defs, same trust boundary; grouped by the increment they
; neighbor. The corpus gate discovers every spec/semantics/*.sexp file, and baselines key by case
; DESCRIPTION (not filename), so relocating these cases is verdict-neutral — their .gate-baseline
; entries are unchanged and ship in this same commit.
; ============================================================================================
; --- The trust boundary is module COOPERATION: the deliberate-leak and transport faces --------------
; The unforgeability pins in 25-verification.sexp establish that OUTSIDE code cannot forge or
; destructure a Thm without the kernel's cooperation. These pin the boundary's exact shape from the
; other side, promoted from passing breaker probes.
(diagnostic-quality)

(case
  "a kernel may deliberately export its rule as a first-class value"
  (doc
    "`(def (mk-forger) Thm.Proved)` exported — the kernel RETURNS its private ctor as a
           function value, and outside code applies it to build a Thm (99). This is LEGAL and
           equivalent to exporting the eta-wrapper `(def (mk2 v) (Thm.Proved v))` (a checkless smart
           constructor): the unforgeability guarantee is that outside code cannot forge WITHOUT the
           kernel's cooperation, not that the language cages the ctor against the kernel's own
           choices. A real kernel simply never writes mk-forger — and this pin documents that the
           boundary is exactly the module's exported surface, no more.")
  (module "kernel"
    (do
      (type Thm (Proved Int64))
      (def (axiom) (Thm.Proved 42))
      (def (thm-val (: t Thm)) (match t ((Thm.Proved v) v)))
      (def (mk-forger) Thm.Proved)
      (export Thm axiom thm-val mk-forger)))
  (input
    (do
      (import "kernel" (Thm axiom thm-val mk-forger))
      (def (main (: d Int64)) (thm-val ((mk-forger) 99)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 99 Int64)))

(case
  "a Thm rides a collection through outside code without destructure rights"
  (doc
    "`(List.at (List.push (list) (axiom)) 0)` — outside code CARRIES a legitimately-obtained
           Thm through a collection and hands it back to the kernel's accessor → 42. Pins that the
           abstract type is a first-class VALUE for transport (store, collect, extract) even where
           construction and destructure are withheld — an LCF proof store is exactly a collection of
           Thms held by untrusted orchestration code.")
  (module "kernel"
    (do
      (type Thm (Proved Int64))
      (def (axiom) (Thm.Proved 42))
      (def (thm-val (: t Thm)) (match t ((Thm.Proved v) v)))
      (export Thm axiom thm-val)))
  (input
    (do
      (import "kernel" (Thm axiom thm-val))
      (def (main (: d Int64)) (thm-val (Option.expect (List.at (List.push #list() (axiom)) 0) "t")))
      (export main)))
  (call main (: 0 Int64))
  (output (: 42 Int64)))

(case
  "a Thm rides a MAP value through outside code and comes back to the kernel accessor"
  (doc
    "The CHAMP companion of the list-transport case: the abstract Thm — built with a RUNTIME payload
           via the kernel's `axiom` — is stored as a MAP VALUE by untrusted code, looked up, and handed
           back to the kernel's accessor → 42. The proof STORE idiom at its real shape: an LCF orchestrator
           keys theorems in a map (by goal id, by hash) without destructure rights; the abstract value must
           survive the CHAMP insert/lookup round-trip exactly as a scalar would. Runtime payload keeps the
           whole path live (no fold).")
  (module "kernel"
    (do
      (type Thm (Proved Int64))
      (def (axiom (: n Int64)) (Thm.Proved n))
      (def (thm-val (: t Thm)) (match t ((Thm.Proved v) v)))
      (export Thm axiom thm-val)))
  (input
    (do
      (import "kernel" (Thm axiom thm-val))
      (def
        (main (: n Int64))
        (match
          (Map.lookup (Map.insert Map.empty 1 (axiom n)) 1)
          ((Some t) (thm-val t))
          ((None u) -1)))
      (export main)))
  (call main (: 42 Int64))
  (output (: 42 Int64)))

; --- The subst soundness edges: shadow blocking, selective substitution, and the documented hazard --
; Inc 4's subst is the kernel's soundness-critical mechanism (an unsound subst mints false theorems
; through BETA). These pin its edges directly, promoted from passing breaker probes; the third case
; DELIBERATELY pins the naive subst's capture hazard (subst 1 (Var 2) into (Abs 2 (Var 1)) captures)
; so the later capture-avoiding increment CHANGES a graded answer visibly instead of silently.
; breaker probe: the HOL subst's capture edges (self-contained kernel matching Inc4's defs).
; Hand-computed:
;   p81a shadowing binder blocks: subst 1 (Var 9) (Abs 1 (Var 1)) = (Abs 1 (Var 1)) — unchanged.
;        Verdict via term-eq -> 1.
;   p81b free-beside-shadow: subst 1 (Var 9) (Comb (Var 1) (Abs 1 (Var 1))) =
;        (Comb (Var 9) (Abs 1 (Var 1))): the free occurrence substitutes, the shadowed does not.
;   p81c substitution UNDER a distinct binder: subst 1 (Var 9) (Abs 2 (Var 1)) = (Abs 2 (Var 9)).
;   p81d the capture HAZARD the naive subst has (documented later-increment): subst 1 (Var 2)
;        (Abs 2 (Var 1)) = (Abs 2 (Var 2)) — CAPTURE. The kernel doc says α-conversion is a later
;        increment and cases use distinct ids; pin the DOCUMENTED naive behavior so the later
;        capture-avoiding subst CHANGES this case visibly (it will need a fresh binder).
;   p81e beta over a shadowing body: beta 1 (Abs 1 (Var 1)) (Var 9) mints
;        ((λ1.λ1.1) 9) = (λ1.1) — the rhs must be the UNsubstituted inner lambda.
(case
  "breaker holsubst: a shadowing binder blocks substitution"
  (doc "Promoted breaker probe — see the section comment.")
  (input
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
      (def
        (subst (: v Int64) (: s Term) (: t Term))
        (match
          t
          ((Term.Var n) (if (= n v) s (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
          ((Term.Eq a b) (Term.Eq (subst v s a) (subst v s b)))
          ((Term.Abs w body) (if (= w v) (Term.Abs w body) (Term.Abs w (subst v s body))))))
      (def
        (teq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (teq x p) (teq y q))) (_ false)))
          ((Term.Eq x y) (match b ((Term.Eq p q) (and (teq x p) (teq y q))) (_ false)))
          ((Term.Abs w x) (match b ((Term.Abs u y) (and (= w u) (teq x y))) (_ false)))))
      (def
        (main (: d Int64))
        (if (teq (subst 1 (Term.Var 9) (Term.Abs 1 (Term.Var 1))) (Term.Abs 1 (Term.Var 1))) 1 0))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "breaker holsubst: a free occurrence beside a shadow substitutes selectively"
  (doc "Promoted breaker probe — see the section comment.")
  (input
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
      (def
        (subst (: v Int64) (: s Term) (: t Term))
        (match
          t
          ((Term.Var n) (if (= n v) s (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
          ((Term.Eq a b) (Term.Eq (subst v s a) (subst v s b)))
          ((Term.Abs w body) (if (= w v) (Term.Abs w body) (Term.Abs w (subst v s body))))))
      (def
        (teq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (teq x p) (teq y q))) (_ false)))
          ((Term.Eq x y) (match b ((Term.Eq p q) (and (teq x p) (teq y q))) (_ false)))
          ((Term.Abs w x) (match b ((Term.Abs u y) (and (= w u) (teq x y))) (_ false)))))
      (def
        (main (: d Int64))
        (if
          (teq
            (subst 1 (Term.Var 9) (Term.Comb (Term.Var 1) (Term.Abs 1 (Term.Var 1))))
            (Term.Comb (Term.Var 9) (Term.Abs 1 (Term.Var 1))))
          1
          0))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "breaker holsubst: the naive subst's documented capture hazard"
  (doc "Promoted breaker probe — see the section comment.")
  (input
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
      (def
        (subst (: v Int64) (: s Term) (: t Term))
        (match
          t
          ((Term.Var n) (if (= n v) s (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
          ((Term.Eq a b) (Term.Eq (subst v s a) (subst v s b)))
          ((Term.Abs w body) (if (= w v) (Term.Abs w body) (Term.Abs w (subst v s body))))))
      (def
        (teq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (teq x p) (teq y q))) (_ false)))
          ((Term.Eq x y) (match b ((Term.Eq p q) (and (teq x p) (teq y q))) (_ false)))
          ((Term.Abs w x) (match b ((Term.Abs u y) (and (= w u) (teq x y))) (_ false)))))
      (def
        (main (: d Int64))
        (if (teq (subst 1 (Term.Var 2) (Term.Abs 2 (Term.Var 1))) (Term.Abs 2 (Term.Var 2))) 1 0))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

; --- Capture-avoiding subst: the α-rename's structural edges ----------------------------------------
; Inc 5's pins verify the substituted free var SURVIVES (free-in true). These pin the α-rename's
; STRUCTURE — the exact renamed term, promoted from passing breaker probes: the fresh id must clear
; BOTH s's and the body's ids (not just s's), and a non-capturing subst must take the plain path
; (no spurious rename).
(case
  "breaker capsubst: the fresh binder clears the body's ids, not only s's"
  (doc "Promoted breaker probe — see the section comment.")
  (input
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
      (def
        (free-in (: v Int64) (: t Term))
        (match
          t
          ((Term.Var n) (= n v))
          ((Term.Comb f x) (or (free-in v f) (free-in v x)))
          ((Term.Eq a b) (or (free-in v a) (free-in v b)))
          ((Term.Abs w body) (if (= w v) false (free-in v body)))))
      (def
        (max-id (: t Term))
        (match
          t
          ((Term.Var n) n)
          ((Term.Comb f x) (let ((a (max-id f)) (b (max-id x))) (if (> a b) a b)))
          ((Term.Eq a b) (let ((p (max-id a)) (q (max-id b))) (if (> p q) p q)))
          ((Term.Abs w body) (let ((m (max-id body))) (if (> w m) w m)))))
      (def
        (rename (: from Int64) (: to Int64) (: t Term))
        (match
          t
          ((Term.Var n) (if (= n from) (Term.Var to) (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (rename from to f) (rename from to x)))
          ((Term.Eq a b) (Term.Eq (rename from to a) (rename from to b)))
          ((Term.Abs w body) (if (= w from) (Term.Abs w body) (Term.Abs w (rename from to body))))))
      (def
        (subst (: v Int64) (: s Term) (: t Term))
        (match
          t
          ((Term.Var n) (if (= n v) s (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
          ((Term.Eq a b) (Term.Eq (subst v s a) (subst v s b)))
          ((Term.Abs w body)
            (if
              (= w v)
              (Term.Abs w body)
              (if
                (free-in w s)
                (let
                  ((fresh (+ 1 (let ((ms (max-id s)) (mt (max-id body))) (if (> ms mt) ms mt)))))
                  (Term.Abs fresh (subst v s (rename w fresh body))))
                (Term.Abs w (subst v s body)))))))
      (def
        (teq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (teq x p) (teq y q))) (_ false)))
          ((Term.Eq x y) (match b ((Term.Eq p q) (and (teq x p) (teq y q))) (_ false)))
          ((Term.Abs w x) (match b ((Term.Abs u y) (and (= w u) (teq x y))) (_ false)))))
      (def
        (main (: d Int64))
        (if
          (teq
            (subst 0 (Term.Var 1) (Term.Abs 1 (Term.Comb (Term.Var 0) (Term.Var 7))))
            (Term.Abs 8 (Term.Comb (Term.Var 1) (Term.Var 7))))
          1
          0))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "breaker capsubst: a non-capturing substitution takes the plain path"
  (doc "Promoted breaker probe — see the section comment.")
  (input
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
      (def
        (free-in (: v Int64) (: t Term))
        (match
          t
          ((Term.Var n) (= n v))
          ((Term.Comb f x) (or (free-in v f) (free-in v x)))
          ((Term.Eq a b) (or (free-in v a) (free-in v b)))
          ((Term.Abs w body) (if (= w v) false (free-in v body)))))
      (def
        (max-id (: t Term))
        (match
          t
          ((Term.Var n) n)
          ((Term.Comb f x) (let ((a (max-id f)) (b (max-id x))) (if (> a b) a b)))
          ((Term.Eq a b) (let ((p (max-id a)) (q (max-id b))) (if (> p q) p q)))
          ((Term.Abs w body) (let ((m (max-id body))) (if (> w m) w m)))))
      (def
        (rename (: from Int64) (: to Int64) (: t Term))
        (match
          t
          ((Term.Var n) (if (= n from) (Term.Var to) (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (rename from to f) (rename from to x)))
          ((Term.Eq a b) (Term.Eq (rename from to a) (rename from to b)))
          ((Term.Abs w body) (if (= w from) (Term.Abs w body) (Term.Abs w (rename from to body))))))
      (def
        (subst (: v Int64) (: s Term) (: t Term))
        (match
          t
          ((Term.Var n) (if (= n v) s (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
          ((Term.Eq a b) (Term.Eq (subst v s a) (subst v s b)))
          ((Term.Abs w body)
            (if
              (= w v)
              (Term.Abs w body)
              (if
                (free-in w s)
                (let
                  ((fresh (+ 1 (let ((ms (max-id s)) (mt (max-id body))) (if (> ms mt) ms mt)))))
                  (Term.Abs fresh (subst v s (rename w fresh body))))
                (Term.Abs w (subst v s body)))))))
      (def
        (teq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (teq x p) (teq y q))) (_ false)))
          ((Term.Eq x y) (match b ((Term.Eq p q) (and (teq x p) (teq y q))) (_ false)))
          ((Term.Abs w x) (match b ((Term.Abs u y) (and (= w u) (teq x y))) (_ false)))))
      (def
        (main (: d Int64))
        (if (teq (subst 0 (Term.Var 5) (Term.Abs 1 (Term.Var 0))) (Term.Abs 1 (Term.Var 5))) 1 0))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

; --- ∀-elimination with a real substitution (the non-identity SPEC face) ---------------------------
(case
  "breaker specsubst: GEN of refl then SPEC substitutes the bound var throughout the body"
  (doc
    "The FIRST-THEOREM ∀ round-trip with a REAL substitution (Inc-8's GEN pin exercises only
           identity-SPEC, where the bound var is absent from the body): `refl (Var 1)` is
           `{} |- (Var1 = Var1)` with NO hypotheses, so GEN 1 is sound and the body MENTIONS 1 →
           `{} |- ∀1.(Var1 = Var1)`; SPEC (Var 7) substitutes throughout → `{} |- (Var7 = Var7)`. Pins
           that ∀-elimination genuinely instantiates a quantified body (not just a no-op), the
           complement of the side-condition pin — a SPEC that dropped the subst would leave (Var1 = Var1).")
  (input
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term) (Forall Int64 Term))
      (type Thm (Seq (List Term) Term))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y) (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Abs v x) (match b ((Term.Abs w q) (and (= v w) (term-eq x q))) (_ false)))
          ((Term.Forall v x) (match b ((Term.Forall w q) (and (= v w) (term-eq x q))) (_ false)))))
      (def
        (free-in (: v Int64) (: t Term))
        (match
          t
          ((Term.Var n) (= n v))
          ((Term.Comb f x) (or (free-in v f) (free-in v x)))
          ((Term.Eq a b) (or (free-in v a) (free-in v b)))
          ((Term.Abs w body) (if (= w v) false (free-in v body)))
          ((Term.Forall w body) (if (= w v) false (free-in v body)))))
      (def
        (subst (: v Int64) (: s Term) (: t Term))
        (match
          t
          ((Term.Var n) (if (= n v) s (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
          ((Term.Eq a b) (Term.Eq (subst v s a) (subst v s b)))
          ((Term.Abs w body) (if (= w v) (Term.Abs w body) (Term.Abs w (subst v s body))))
          ((Term.Forall w body) (if (= w v) (Term.Forall w body) (Term.Forall w (subst v s body))))))
      (def
        (free-in-hyps (: v Int64) (: hs (List Term)))
        (match hs (#list() false) (#list(h (.. rest)) (or (free-in v h) (free-in-hyps v rest)))))
      (def (refl (: t Term)) (Thm.Seq #list() (Term.Eq t t)))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def
        (gen (: x Int64) (: th Thm))
        (match
          th
          ((Thm.Seq g p)
            (if (free-in-hyps x g) (Option.None unit) (Option.Some (Thm.Seq g (Term.Forall x p)))))))
      (def
        (spec (: t Term) (: th Thm))
        (match
          (concl th)
          ((Term.Forall x body)
            (Option.Some (Thm.Seq (match th ((Thm.Seq h _) h)) (subst x t body))))
          (_ (Option.None unit))))
      (def
        (main (: d Int64))
        (match
          (gen 1 (refl (Term.Var 1)))
          ((Option.Some g)
            (match
              (spec (Term.Var 7) g)
              ((Option.Some sp) (if (term-eq (concl sp) (Term.Eq (Term.Var 7) (Term.Var 7))) 1 0))
              ((Option.None _) -1)))
          ((Option.None _) -2)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (live-objects known-leak))

(case
  "TRANS unions the hypotheses of BOTH operands when each carries a distinct assumption"
  (doc
    "The soundness fix (TRANS/MK_COMB union operand hypotheses) is pinned for the one-operand case; this pins the actual union — both operands carry a distinct assumption and the result must retain BOTH. A TRANS that kept only one operand's hypotheses (or emptied them) would silently discharge a live assumption, letting an unproven equation escape. trans({a=b}|-a=b, {b=c}|-b=c) = {a=b, b=c}|-a=c, so hyps has length 2.")
  (input
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term))
      (type Thm (Seq (List Term) Term))
      (def
        (term-eq (: p Term) (: q Term))
        (match
          p
          ((Term.Var n) (match q ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb a b) (match q ((Term.Comb c d) (and (term-eq a c) (term-eq b d))) (_ false)))
          ((Term.Eq a b) (match q ((Term.Eq c d) (and (term-eq a c) (term-eq b d))) (_ false)))))
      (def (assume (: t Term)) (Thm.Seq (List.push #list() t) t))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def
        (trans (: t1 Thm) (: t2 Thm))
        (match
          (concl t1)
          ((Term.Eq a b)
            (match
              (concl t2)
              ((Term.Eq b2 c)
                (if
                  (term-eq b b2)
                  (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (Term.Eq a c)))
                  (Option.None unit)))
              (_ (Option.None unit))))
          (_ (Option.None unit))))
      (def
        (main (: d Int64))
        (match
          (trans
            (assume (Term.Eq (Term.Var 1) (Term.Var 2)))
            (assume (Term.Eq (Term.Var 2) (Term.Var 3))))
          ((Option.Some r) (List.len (hyps r)))
          ((Option.None _) -1)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2 Int64))
  (live-objects 0))

(case
  "TRANS carries a hypothesis borne by the RIGHT operand only (mirror of the left-only pin)"
  (doc
    "The landed pin threads an assumption through TRANS's LEFT operand (refl on the right). This mirrors it: the hypothesis lives on the RIGHT operand and refl (no hypotheses) is on the left. trans(refl a, {a=c}|-a=c) = {a=c}|-a=c, so hyps has length 1. A TRANS reading only the left operand's hypotheses would drop the assumption entirely.")
  (input
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term))
      (type Thm (Seq (List Term) Term))
      (def
        (term-eq (: p Term) (: q Term))
        (match
          p
          ((Term.Var n) (match q ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb a b) (match q ((Term.Comb c d) (and (term-eq a c) (term-eq b d))) (_ false)))
          ((Term.Eq a b) (match q ((Term.Eq c d) (and (term-eq a c) (term-eq b d))) (_ false)))))
      (def (assume (: t Term)) (Thm.Seq (List.push #list() t) t))
      (def (refl (: t Term)) (Thm.Seq #list() (Term.Eq t t)))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def
        (trans (: t1 Thm) (: t2 Thm))
        (match
          (concl t1)
          ((Term.Eq a b)
            (match
              (concl t2)
              ((Term.Eq b2 c)
                (if
                  (term-eq b b2)
                  (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (Term.Eq a c)))
                  (Option.None unit)))
              (_ (Option.None unit))))
          (_ (Option.None unit))))
      (def
        (main (: d Int64))
        (match
          (trans (refl (Term.Var 1)) (assume (Term.Eq (Term.Var 1) (Term.Var 3))))
          ((Option.Some r) (List.len (hyps r)))
          ((Option.None _) -1)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 1 Int64))
  (live-objects 0))

(case
  "MK_COMB unions the hypotheses of BOTH operands when each carries a distinct assumption"
  (doc
    "Companion to the TRANS two-operand union: MK_COMB combines {f=g} and {x=y} into (Comb f x)=(Comb g y), and the result must retain BOTH assumptions. mk-comb({f=g}|-f=g, {x=y}|-x=y) has hyps of length 2. An MK_COMB emitting an empty hypothesis set (the pre-fix bug) would let a congruence step launder away two live assumptions.")
  (input
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term))
      (type Thm (Seq (List Term) Term))
      (def (assume (: t Term)) (Thm.Seq (List.push #list() t) t))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def
        (mk-comb (: t1 Thm) (: t2 Thm))
        (match
          (concl t1)
          ((Term.Eq f g)
            (match
              (concl t2)
              ((Term.Eq x y)
                (Option.Some
                  (Thm.Seq
                    (List.concat (hyps t1) (hyps t2))
                    (Term.Eq (Term.Comb f x) (Term.Comb g y)))))
              (_ (Option.None unit))))
          (_ (Option.None unit))))
      (def
        (main (: d Int64))
        (match
          (mk-comb
            (assume (Term.Eq (Term.Var 1) (Term.Var 2)))
            (assume (Term.Eq (Term.Var 3) (Term.Var 4))))
          ((Option.Some r) (List.len (hyps r)))
          ((Option.None _) -1)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 2 Int64)))

(case
  "TRANS chained three deep accumulates every operand's hypotheses"
  (doc
    "Assumptions must accumulate transitively, not just pairwise. Chaining trans(trans({a=b},{b=c}), {c=d}) yields {a=b,b=c,c=d}|-a=d, so hyps has length 3. This guards against a union that resets or caps at the most recent step — the accumulation across nested proof structure is where a subtle drop would hide.")
  (input
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term))
      (type Thm (Seq (List Term) Term))
      (def
        (term-eq (: p Term) (: q Term))
        (match
          p
          ((Term.Var n) (match q ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb a b) (match q ((Term.Comb c d) (and (term-eq a c) (term-eq b d))) (_ false)))
          ((Term.Eq a b) (match q ((Term.Eq c d) (and (term-eq a c) (term-eq b d))) (_ false)))))
      (def (assume (: t Term)) (Thm.Seq (List.push #list() t) t))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def
        (trans (: t1 Thm) (: t2 Thm))
        (match
          (concl t1)
          ((Term.Eq a b)
            (match
              (concl t2)
              ((Term.Eq b2 c)
                (if
                  (term-eq b b2)
                  (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (Term.Eq a c)))
                  (Option.None unit)))
              (_ (Option.None unit))))
          (_ (Option.None unit))))
      (def
        (main (: d Int64))
        (match
          (trans
            (assume (Term.Eq (Term.Var 1) (Term.Var 2)))
            (assume (Term.Eq (Term.Var 2) (Term.Var 3))))
          ((Option.Some r12)
            (match
              (trans r12 (assume (Term.Eq (Term.Var 3) (Term.Var 4))))
              ((Option.Some r) (List.len (hyps r)))
              ((Option.None _) -1)))
          ((Option.None _) -2)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 3 Int64))
  (live-objects known-leak))

; --- Increment 12 slice-2 ∨/∃ NEIGHBORS (breaker): the unforgeability + hyp-preservation faces skipped ---
; The slice-2 case tests EXISTS-intro's POSITIVE path (matching witness) and DISJ1. These pin the unpinned
; soundness-critical neighbors: EXISTS-intro's NEGATIVE path (a mismatched witness MUST be rejected — the
; unforgeability guard, without which ∃ could be minted from a non-instance), DISJ2 (the mirror intro),
; EXISTS over a body where the witness var is itself free (subst must replace all bound occurrences), and
; EXISTS-intro preserving MULTIPLE hypotheses assembled through kernel rules.
(case
  "breaker exists: EXISTS-intro rejects a witness whose instance does not match the premise (unforgeability)"
  (doc
    "The soundness-critical NEGATIVE path the slice-2 case skips: EXISTS-intro checks the witness before
           minting. body = (Var 0)=(Var 5), claimed witness (Var 5) EXPECTS a premise (Var 5)=(Var 5); given a
           premise (Var 5)=(Var 7) instead (a NON-instance), the term-eq check fails and EXISTS-intro returns
           None. This is the unforgeability guard — an EXISTS-intro that minted ∃x.P from a non-matching
           premise would let a false existential be proven. Asserts the mismatched witness is REJECTED (None).")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Exists Int64 Term))
      (type Thm (Seq (List Term) Term))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y) (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Exists v x) (match b ((Term.Exists w q) (and (= v w) (term-eq x q))) (_ false)))))
      (def
        (subst (: v Int64) (: s Term) (: t Term))
        (match
          t
          ((Term.Var n) (if (= n v) s (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
          ((Term.Eq a b) (Term.Eq (subst v s a) (subst v s b)))
          ((Term.Exists w body) (if (= w v) (Term.Exists w body) (Term.Exists w (subst v s body))))))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def
        (exists-intro (: x Int64) (: body Term) (: witness Term) (: th Thm))
        (if
          (term-eq (concl th) (subst x witness body))
          (Option.Some (Thm.Seq (hyps th) (Term.Exists x body)))
          (Option.None)))
      (export Term.*)
      (export Thm)
      (export term-eq)
      (export subst)
      (export assume)
      (export exists-intro)
      (export concl)
      (export hyps)))
  (input
    (do
      (import "hol" (Term Thm term-eq subst assume exists-intro concl hyps))
      (def
        (main)
        (let
          ((a (Term.Var 5)))
          (let
            ((body (Term.Eq (Term.Var 0) a)) (wrong-premise (Term.Eq a (Term.Var 7))))
            (match
              (exists-intro 0 body a (assume wrong-premise))
              ((Option.Some _) false)
              ((Option.None) true)))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

(case
  "breaker exists: DISJ2 preserves the premise's hypothesis"
  (doc
    "The slice-2 case exercises DISJ1; this pins DISJ2, the mirror ∨-introduction. From ASSUME(b) : {b}⊢b,
           DISJ2 with an arbitrary left disjunct a derives {b} ⊢ a∨b — the hypothesis {b} survives. Asserts the
           disjunction conclusion a∨b and that the premise hypothesis is preserved.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Disj Term Term))
      (type Thm (Seq (List Term) Term))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y) (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Disj x y) (match b ((Term.Disj p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (disj2 (: a Term) (: th Thm)) (Thm.Seq (hyps th) (Term.Disj a (concl th))))
      (export Term.*)
      (export Thm)
      (export term-eq)
      (export assume)
      (export disj2)
      (export concl)
      (export hyps)))
  (input
    (do
      (import "hol" (Term Thm term-eq assume disj2 concl hyps))
      (def
        (main)
        (let
          ((a (Term.Var 1)) (b (Term.Var 2)))
          (let
            ((d (disj2 a (assume b))))
            (and
              (term-eq (concl d) (Term.Disj a b))
              (match (hyps d) (#list(h) (term-eq h b)) (_ false))))))
      (export main)))
  (output (: true Bool))
  (live-objects 0))

(case
  "breaker exists: EXISTS-intro with the witness variable also free in the body substitutes all occurrences"
  (doc
    "EXISTS-intro when the witness variable is ALSO free in the body: body = (Var 0)=(Var 0), witness (Var 9).
           subst 0 (Var 9) body must replace BOTH bound occurrences, giving (Var 9)=(Var 9); the matching
           premise (Var 9)=(Var 9) is accepted and ∃0.((Var 0)=(Var 0)) is minted. Pins that the witness check
           substitutes every bound occurrence, not just the first.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Exists Int64 Term))
      (type Thm (Seq (List Term) Term))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y) (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Exists v x) (match b ((Term.Exists w q) (and (= v w) (term-eq x q))) (_ false)))))
      (def
        (subst (: v Int64) (: s Term) (: t Term))
        (match
          t
          ((Term.Var n) (if (= n v) s (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
          ((Term.Eq a b) (Term.Eq (subst v s a) (subst v s b)))
          ((Term.Exists w body) (if (= w v) (Term.Exists w body) (Term.Exists w (subst v s body))))))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def
        (exists-intro (: x Int64) (: body Term) (: witness Term) (: th Thm))
        (if
          (term-eq (concl th) (subst x witness body))
          (Option.Some (Thm.Seq (hyps th) (Term.Exists x body)))
          (Option.None)))
      (export Term.*)
      (export Thm)
      (export term-eq)
      (export subst)
      (export assume)
      (export exists-intro)
      (export concl)
      (export hyps)))
  (input
    (do
      (import "hol" (Term Thm term-eq subst assume exists-intro concl hyps))
      (def
        (main)
        (let
          ((body (Term.Eq (Term.Var 0) (Term.Var 0))) (witness (Term.Var 9)))
          (let
            ((premise (Term.Eq witness witness)))
            (match
              (exists-intro 0 body witness (assume premise))
              ((Option.Some e) (term-eq (concl e) (Term.Exists 0 body)))
              ((Option.None) false)))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

(case
  "breaker exists: EXISTS-intro preserves multiple hypotheses from the premise"
  (doc
    "EXISTS-intro carries ALL of the premise’s hypotheses, not just one. The premise is built through kernel
           rules only (conj of two assumptions gathers {p,q}, retarget swaps the conclusion to the witness
           instance) — no raw Thm construction outside the module (which Inc-13 correctly withholds, CDZ0214).
           The minted ∃ keeps both p,q as hypotheses. Asserts hyps has length 2 and both survive.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Exists Int64 Term) (Conj Term Term))
      (type Thm (Seq (List Term) Term))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y) (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Exists v x) (match b ((Term.Exists w q) (and (= v w) (term-eq x q))) (_ false)))
          ((Term.Conj x y) (match b ((Term.Conj p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def
        (subst (: v Int64) (: s Term) (: t Term))
        (match
          t
          ((Term.Var n) (if (= n v) s (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
          ((Term.Eq a b) (Term.Eq (subst v s a) (subst v s b)))
          ((Term.Exists w body) (if (= w v) (Term.Exists w body) (Term.Exists w (subst v s body))))
          ((Term.Conj a b) (Term.Conj (subst v s a) (subst v s b)))))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      ; A legitimate 2-hyp premise assembled ONLY through kernel rules (no raw Thm.Seq outside the module):
      ; conj two assumptions to gather {p,q}, then re-label the conclusion to the target via a trusted
      ; rewrite rule `retarget` that keeps the hypotheses and swaps the conclusion for a supplied one.
      (def
        (conj (: th1 Thm) (: th2 Thm))
        (Thm.Seq (List.concat (hyps th1) (hyps th2)) (Term.Conj (concl th1) (concl th2))))
      (def (retarget (: th Thm) (: c Term)) (Thm.Seq (hyps th) c))
      (def
        (exists-intro (: x Int64) (: body Term) (: witness Term) (: th Thm))
        (if
          (term-eq (concl th) (subst x witness body))
          (Option.Some (Thm.Seq (hyps th) (Term.Exists x body)))
          (Option.None)))
      (export Term.*)
      (export Thm)
      (export term-eq)
      (export subst)
      (export assume)
      (export conj)
      (export retarget)
      (export exists-intro)
      (export concl)
      (export hyps)))
  (input
    (do
      (import "hol" (Term Thm term-eq subst assume conj retarget exists-intro concl hyps))
      (def
        (main)
        (let
          ((w (Term.Var 5)))
          (let
            ((body (Term.Eq (Term.Var 0) w)) (p (Term.Var 1)) (q (Term.Var 2)))
            ; premise built through rules: conj(assume p, assume q) -> {p,q} then retarget concl to (Eq w w)
            (let
              ((premise (retarget (conj (assume p) (assume q)) (Term.Eq w w))))
              (match
                (exists-intro 0 body w premise)
                ((Option.Some e)
                  (match (hyps e) (#list(h1 h2) (and (term-eq h1 p) (term-eq h2 q))) (_ false)))
                ((Option.None) false))))))
      (export main)))
  (output (: true Bool))
  (live-objects known-leak))

; --- Increment 12 conjunction NEIGHBORS (breaker): the elim/accumulation faces the landed case skips ---
; The Inc-12 conjunction case pins CONJ union + CONJUNCT1 preservation over a single conj. These pin the
; unpinned neighbors — CONJUNCT2 (the other elim), a nested three-deep conj (accumulation), the soundness-
; critical detail that CONJUNCT1 keeps BOTH operand hyps (not just the projected side's), and that conj
; concats a shared hyp without dedup (matching List.concat). Same gap-shape the TRANS/MK_COMB union bug hid.
(case
  "conjunction elimination CONJUNCT2 preserves both operand hypotheses"
  (doc
    "The Inc-12 conjunction case checks CONJUNCT1; this pins CONJUNCT2, the other projection. From
           CONJ(ASSUME a, ASSUME b) : {a,b} ⊢ a∧b, CONJUNCT2 derives {a,b} ⊢ b — the SECOND conjunct, with
           BOTH hypotheses still carried. A CONJUNCT2 that dropped a hypothesis (kept only b's, or emptied
           the set) would silently discharge the live assumption a — the elim analogue of the TRANS/MK_COMB
           hypothesis-drop soundness bug. Asserts the conclusion is b AND both a,b survive as hypotheses.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Conj Term Term))
      (type Thm (Seq (List Term) Term))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y) (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Conj x y) (match b ((Term.Conj p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def
        (conj (: th1 Thm) (: th2 Thm))
        (Thm.Seq (List.concat (hyps th1) (hyps th2)) (Term.Conj (concl th1) (concl th2))))
      (def
        (conjunct2 (: th Thm))
        (match (concl th) ((Term.Conj a b) (Option.Some (Thm.Seq (hyps th) b))) (_ (Option.None))))
      (export Term.*)
      (export Thm)
      (export term-eq)
      (export assume)
      (export conj)
      (export conjunct2)
      (export concl)
      (export hyps)))
  (input
    (do
      (import "hol" (Term Thm term-eq assume conj conjunct2 concl hyps))
      (def
        (main)
        (let
          ((a (Term.Var 1)) (b (Term.Var 2)))
          (match
            (conjunct2 (conj (assume a) (assume b)))
            ((Option.Some c2)
              (and
                (term-eq (concl c2) b)
                (match (hyps c2) (#list(h1 h2) (and (term-eq h1 a) (term-eq h2 b))) (_ false))))
            ((Option.None) false))))
      (export main)))
  (output (: true Bool))
  (live-objects 0))

(case
  "a nested conjunction accumulates all three operand hypotheses"
  (doc
    "Hypotheses must accumulate across nested conjunction structure, not just pairwise. CONJ(CONJ(
           ASSUME a, ASSUME b), ASSUME c) derives {a,b,c} ⊢ (a∧b)∧c — three hypotheses. This guards a union
           that reset or capped at the most recent operand (the accumulation face that caught MK_COMB in
           the TRANS/MK_COMB audit). Asserts the conclusion shape (a∧b)∧c and that hyps has length 3.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Conj Term Term))
      (type Thm (Seq (List Term) Term))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y) (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Conj x y) (match b ((Term.Conj p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def
        (conj (: th1 Thm) (: th2 Thm))
        (Thm.Seq (List.concat (hyps th1) (hyps th2)) (Term.Conj (concl th1) (concl th2))))
      (export Term.*)
      (export Thm)
      (export term-eq)
      (export assume)
      (export conj)
      (export concl)
      (export hyps)))
  (input
    (do
      (import "hol" (Term Thm term-eq assume conj concl hyps))
      (def
        (main)
        (let
          ((a (Term.Var 1)) (b (Term.Var 2)) (c (Term.Var 3)))
          (let
            ((nested (conj (conj (assume a) (assume b)) (assume c))))
            (and
              (term-eq (concl nested) (Term.Conj (Term.Conj a b) c))
              (match (hyps nested) (#list(h1 h2 h3) true) (_ false))))))
      (export main)))
  (output (: true Bool))
  (live-objects 0))

(case
  "conjunction elimination keeps BOTH operand hypotheses, not just the projected conjunct's"
  (doc
    "The soundness-critical projection detail: CONJUNCT1 of CONJ(ASSUME a, ASSUME b) yields ⊢ a, and
           it must keep BOTH {a,b} as hypotheses — the conjunction's full assumption set — not just {a},
           the projected conjunct's own. A projection that narrowed the hypotheses to the returned side
           would discharge the live assumption b behind the eliminated conjunct. Asserts conclusion a AND
           that both a,b remain hypotheses (the union survives the projection).")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Conj Term Term))
      (type Thm (Seq (List Term) Term))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y) (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Conj x y) (match b ((Term.Conj p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def
        (conj (: th1 Thm) (: th2 Thm))
        (Thm.Seq (List.concat (hyps th1) (hyps th2)) (Term.Conj (concl th1) (concl th2))))
      (def
        (conjunct1 (: th Thm))
        (match (concl th) ((Term.Conj a b) (Option.Some (Thm.Seq (hyps th) a))) (_ (Option.None))))
      (export Term.*)
      (export Thm)
      (export term-eq)
      (export assume)
      (export conj)
      (export conjunct1)
      (export concl)
      (export hyps)))
  (input
    (do
      (import "hol" (Term Thm term-eq assume conj conjunct1 concl hyps))
      (def
        (main)
        (let
          ((a (Term.Var 1)) (b (Term.Var 2)))
          (match
            (conjunct1 (conj (assume a) (assume b)))
            ((Option.Some c1)
              (and
                (term-eq (concl c1) a)
                (match (hyps c1) (#list(h1 h2) (and (term-eq h1 a) (term-eq h2 b))) (_ false))))
            ((Option.None) false))))
      (export main)))
  (output (: true Bool))
  (live-objects 0))

(case
  "conjunction of two theorems sharing a hypothesis concatenates without dedup"
  (doc
    "The kernel's hypothesis union is List.concat, which does NOT dedup — a deliberate, sound choice
           (a multiset of assumptions; discharging still requires matching each). CONJ(ASSUME a, ASSUME a)
           carries [a, a] — length 2, not collapsed to 1. Pins that the union is a faithful concat, so a
           later refactor to a deduping set is a deliberate change caught here, not a silent one. (Dedup
           would be sound too, but this documents the CURRENT multiset behavior the other cases count on.)")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Conj Term Term))
      (type Thm (Seq (List Term) Term))
      (def
        (term-eq (: a Term) (: b Term))
        (match
          a
          ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y) (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Conj x y) (match b ((Term.Conj p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (assume (: p Term)) (Thm.Seq #list(p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def
        (conj (: th1 Thm) (: th2 Thm))
        (Thm.Seq (List.concat (hyps th1) (hyps th2)) (Term.Conj (concl th1) (concl th2))))
      (export Term.*)
      (export Thm)
      (export term-eq)
      (export assume)
      (export conj)
      (export concl)
      (export hyps)))
  (input
    (do
      (import "hol" (Term Thm term-eq assume conj concl hyps))
      (def
        (main)
        (let
          ((a (Term.Var 1)))
          (let
            ((c (conj (assume a) (assume a))))
            (match (hyps c) (#list(h1 h2) (and (term-eq h1 a) (term-eq h2 a))) (_ false)))))
      (export main)))
  (output (: true Bool))
  (live-objects 0))
