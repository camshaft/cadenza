; ============================================================================================
; 25-verification.sexp — the trust-boundary soundness pins for machine-checked verification
; (an LCF-style HOL kernel baked into Cadenza as a library). See implementation/design/
; DESIGN-verification-hol-kernel.md. Vertical: v-verification.
;
; An LCF kernel's entire value is that its theorem type `Thm` is UNFORGEABLE: everything above the
; trusted kernel is ordinary untrusted code that can only obtain a `Thm` by calling one of the
; kernel's exported inference rules. Cadenza realizes this with an ABSTRACT (opaque) type — the
; kernel module exports the type HANDLE `Thm` but keeps its constructor private, so no importer can
; construct, match, strip, or structurally compare a `Thm` outside the kernel (opaque-types feature;
; modules-and-namespaces.md §A Type's Handle And Its Constructors Are Independently Visible;
; type-system.md §An Abstract Type's Representation Is Not Observable Across Its Boundary).
;
; These cases pin THAT boundary for a `Thm`-shaped abstract type — the soundness invariants the kernel
; depends on — so a future language change cannot silently reopen a forge vector. They are the shape a
; real kernel uses (a `Thm`/`Proof` sum whose constructor is a private inference-rule entry point), not
; the abstract `Color` the 11-modules.sexp opaque-type cases use.
;
; The `eval`-forge vector — an importer trying `(eval (quote (Thm.MkThm …)))` to fabricate a theorem
; through reflection — is CLOSED (trunk `e1506bd7c`, "close the eval-forges-abstract-type-private-ctor
; SOUNDNESS HOLE"): an eval-reconstructed constructor reference re-resolves under the SAME cross-file
; visibility gate as hand-written code, so it is CDZ0214 exactly as a direct reference is. Cases 5–6 pin
; it (construct + match), so this trust-critical fix can never silently regress.
;
; NOTE (tracked, not pinned here): a SINGLE-VARIANT abstract sum's MATCH outside its module currently
; rejects CDZ0203 rather than the withheld-constructor CDZ0214 (construction is always CDZ0214, and a
; MULTI-variant match is too — cases 4 + 6). Sound either way (the match IS rejected, opacity holds),
; but the diagnostic code is wrong per spec. Filed as queue/adv-single-variant-abstract-match-wrong-
; diag-cdz0203-not-cdz0214.sexp; this file pins the multi-variant match (correct CDZ0214) and will gain
; the single-variant match pin once that fix lands.
; ============================================================================================

(case "an abstract theorem type cannot be forged by constructing its rule constructor outside the kernel"
  (doc    "The core LCF unforgeability pin. `hol` exports the abstract handle `Thm` and the inference rule
           `refl`, but NOT `Thm`'s constructor `MkThm`. An importer trying to fabricate a theorem directly
           with `(Thm.MkThm 999)` — skipping the kernel's rules — is rejected CDZ0214: the constructor is
           withheld on purpose. This is the whole trust story: a `Thm` value can be built only by calling an
           exported inference rule (modules-and-namespaces.md §A Type's Handle And Its Constructors Are
           Independently Visible).")
  (module "hol"
    (do
      (type Thm (MkThm Int64))
      (def (refl (: x Int64)) (Thm.MkThm x))
      (export Thm)
      (export refl)))
  (input  (do
            (import "hol" (Thm refl))
            (def (main) (Thm.MkThm 999))
            (export main)))
  (error  CDZ0214))

(case "an abstract theorem is obtained only through the kernel's exported inference rule and accessor"
  (doc    "The companion of the reject above: the SAME abstract kernel used CORRECTLY. The importer never
           names `Thm`'s constructor — it obtains a theorem through the exported rule `refl` and reads it
           through the exported accessor `concl`, the only doors the kernel opened. `(concl (refl 42))` = 42.
           Pins that an abstract theorem is fully usable through the kernel's exported surface while its
           representation stays private — the LCF discipline is ergonomic, not merely safe.")
  (module "hol"
    (do
      (type Thm (MkThm Int64))
      (def (refl (: x Int64)) (Thm.MkThm x))
      (def (concl (: t Thm)) (match t ((Thm.MkThm c) c)))
      (export Thm)
      (export refl)
      (export concl)))
  (input  (do
            (import "hol" (Thm refl concl))
            (def (main) (concl (refl 42)))
            (export main)))
  (output (: 42 Int64)))

(case "a built-in comparison on an abstract theorem value outside the kernel is rejected"
  (doc    "Representation hiding for a theorem: an importer holds `Thm` values (via `refl`) but a built-in
           `=` on two of them is rejected CDZ0202 (type-system.md §An Abstract Type's Representation Is Not
           Observable Across Its Boundary). This matters for a kernel: theorem equality is a KERNEL
           operation (α/structural equality of sequents is the kernel's business), not a structural compare
           of the private representation — a kernel that wants theorems compared exports a function. Pins
           that the private sequent representation cannot be observed through built-in equality.")
  (module "hol"
    (do
      (type Thm (MkThm Int64))
      (def (refl (: x Int64)) (Thm.MkThm x))
      (export Thm)
      (export refl)))
  (input  (do
            (import "hol" (Thm refl))
            (def (main) (= (refl 1) (refl 1)))
            (export main)))
  (error  CDZ0202))

(case "a multi-variant abstract proof type's constructor match outside the kernel is a withheld-constructor rejection"
  (doc    "An importer cannot DESTRUCTURE an abstract proof value outside the kernel: matching `Proof`'s
           withheld constructors is rejected CDZ0214, exactly as constructing them is. `hol` exports the
           handle `Proof` + the rule `ax` but not `Proof`'s constructors, so `(match (ax 3) ((Proof.Axiom n)
           …) ((Proof.Step m) …))` in the importer is a withheld-constructor rejection — the importer can
           neither build nor take apart a proof, only pass it and feed it to exported functions. Pins the
           match half of the boundary for a MULTI-variant abstract type. (A single-variant match currently
           rejects CDZ0203 rather than CDZ0214 — a tracked diagnostic gap, see the header note; that pin is
           added when the fix lands.)")
  (module "hol"
    (do
      (type Proof (Axiom Int64) (Step Int64))
      (def (ax (: n Int64)) (Proof.Axiom n))
      (export Proof)
      (export ax)))
  (input  (do
            (import "hol" (Proof ax))
            (def (main) (match (ax 3) ((Proof.Axiom n) n) ((Proof.Step m) m)))
            (export main)))
  (error  CDZ0214))

(case "eval of a quoted theorem constructor cannot forge an abstract theorem outside the kernel"
  (doc    "The reflection forge vector, CLOSED. An LCF kernel is worthless if an importer can reach the
           private `Thm` constructor through `eval`/`quote` — `(eval (quote (Thm.MkThm 999)))` would
           reconstruct the constructor reference and, if eval got privileged visibility, forge a theorem
           without calling a rule. It does NOT: an eval-reconstructed constructor re-resolves under the
           SAME cross-file visibility as hand-written code, so it is rejected CDZ0214 exactly as the direct
           `(Thm.MkThm 999)` is (trunk `e1506bd7c` closed this; it had forged before). Pins that reflection
           is not a second door onto a `Thm` — the trust boundary holds through `eval`.")
  (module "hol"
    (do
      (type Thm (MkThm Int64))
      (def (refl (: x Int64)) (Thm.MkThm x))
      (export Thm)
      (export refl)))
  (input  (do
            (import "hol" (Thm refl))
            (def (main) (eval (quote (Thm.MkThm 999))))
            (export main)))
  (error  CDZ0214))

(case "eval of a quoted proof-variant match cannot destructure an abstract proof outside the kernel"
  (doc    "The dual of the eval-forge above: `eval` must not let an importer DESTRUCTURE an abstract value
           to read its representation either. `(eval (quote (match (ax 3) ((Proof.Axiom n) …) …)))` would
           reconstruct a match on `Proof`'s withheld constructors; it is rejected CDZ0214, exactly as a
           hand-written match is (case 4). Pins that eval cannot read the private payload out of a proof —
           for a kernel, that a theorem's sequent cannot be extracted through reflection. (Multi-variant
           proof type — the single-variant match diagnostic gap is tracked separately, see the header.)")
  (module "hol"
    (do
      (type Proof (Axiom Int64) (Step Int64))
      (def (ax (: n Int64)) (Proof.Axiom n))
      (export Proof)
      (export ax)))
  (input  (do
            (import "hol" (Proof ax))
            (def (main) (eval (quote (match (ax 3) ((Proof.Axiom n) n) ((Proof.Step m) m)))))
            (export main)))
  (error  CDZ0214))

(case "a re-declared same-name theorem type in another module is a distinct type a kernel value does not satisfy"
  (doc    "The forge-by-re-declaration defense: an attacker cannot fabricate a theorem by declaring their
           OWN structurally-identical `(type Thm (MkThm Int64))` and hoping the kernel's `Thm` values pass
           for it (or vice versa). A user type's identity is its DECLARATION, not its shape (type-system.md
           §Nominal Is An Orthogonal Modifier — identity is the fully-qualified name, file-scoped per the
           opaque-types work). So the importer's own `Thm` is a DISTINCT type: feeding a kernel-built
           `(refl 5)` (type `hol.Thm`) to a function expecting the importer's local `Thm` is a type
           mismatch CDZ0203, not silent acceptance. Pins that nominal identity is unforgeable across the
           module boundary — the composing form is to IMPORT the kernel's one `Thm`, never re-declare it.")
  (module "hol"
    (do
      (type Thm (MkThm Int64))
      (def (refl (: x Int64)) (Thm.MkThm x))
      (export Thm)
      (export refl)))
  (input  (do
            (import "hol" (refl))
            (type Thm (MkThm Int64))
            (def (fake (: t Thm)) (match t ((Thm.MkThm c) c)))
            (def (main) (fake (refl 5)))
            (export main)))
  (error  CDZ0203))

; ============================================================================================
; Increment 2 — the kernel SKELETON exercised as a real HOL fragment (not the toy Thm(MkThm Int64)
; above). A `hol` module declares Term (a HOL term: variable / application / equality) CONCRETELY —
; users build terms — and Thm ABSTRACTLY as a sequent (Seq hyps concl) — only the kernel's inference
; rules mint one. The equational-core leaf rules refl (⊢ t = t) and assume (p ⊢ p) are exported;
; structural term equality (term-eq, HOL's aconv modulo α which a later increment adds) and the
; concl/hyps accessors let a caller CHECK a theorem without being able to FORGE one. These cases run
; the kernel end-to-end through the real pipeline — proving the LCF mechanism works in Cadenza — and
; re-assert the unforgeability boundary for the realistic sequent-shaped Thm.
; ============================================================================================

(case "the kernel proves reflexivity end-to-end: refl t yields a theorem whose conclusion is (t = t)"
  (doc    "The first real theorem. `hol` exports Term concretely (a HOL term: Var / Comb / Eq), Thm
           abstractly (a sequent), the primitive rule `refl`, and the `concl` accessor + `term-eq`
           checker. The entry proves `⊢ x = x` for x = (Var 0) by calling `refl`, then CHECKS the
           conclusion really is an equality of x with itself via the exported term-eq — it never
           fabricates the theorem, it derives it through the rule and inspects it through the accessor.
           Runs to `true`. Pins that the LCF equational core works end-to-end in Cadenza: a primitive
           rule mints a theorem, an accessor reads it, and structural term equality (a recursive walk
           over the Term sum) folds over the derived value.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) ((Term.Comb _ _) false) ((Term.Eq _ _) false)))
          ((Term.Comb x y) (match b ((Term.Var _) false) ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) ((Term.Eq _ _) false)))
          ((Term.Eq x y)   (match b ((Term.Var _) false) ((Term.Comb _ _) false) ((Term.Eq p q) (and (term-eq x p) (term-eq y q)))))))
      (def (refl (: t Term)) (Thm.Seq (list) (Term.Eq t t)))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export refl)
      (export concl)))
  (input  (do
            (import "hol" (Term Thm term-eq refl concl))
            (def (main)
              (match (concl (refl (Term.Var 0)))
                ((Term.Eq a b) (term-eq a b))
                (_ false)))
            (export main)))
  (output (: true Bool)))

(case "the kernel's ASSUME rule yields the sequent {p} |- p"
  (doc    "The second primitive leaf rule: `ASSUME p` produces the theorem `{p} ⊢ p` — p as both the
           sole hypothesis and the conclusion. The sequent carries its hypotheses as a `List Term`. The
           entry assumes (Var 7), then verifies through the exported `concl`/`hyps` accessors that the
           conclusion is p AND the single hypothesis is p (both checked with term-eq). Runs to `true`.
           Pins that a theorem with HYPOTHESES threads its hyp list through the abstract boundary and is
           inspectable via accessors — the shape the discharging rules (DEDUCT_ANTISYM, EQ_MP) consume.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) ((Term.Comb _ _) false) ((Term.Eq _ _) false)))
          ((Term.Comb x y) (match b ((Term.Var _) false) ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) ((Term.Eq _ _) false)))
          ((Term.Eq x y)   (match b ((Term.Var _) false) ((Term.Comb _ _) false) ((Term.Eq p q) (and (term-eq x p) (term-eq y q)))))))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export assume)
      (export concl)
      (export hyps)))
  (input  (do
            (import "hol" (Term Thm term-eq assume concl hyps))
            (def (main)
              (let ((p (Term.Var 7))
                    (th (assume (Term.Var 7))))
                (and (term-eq (concl th) p)
                     (match (hyps th)
                       ((list h) (term-eq h p))
                       (_ false)))))
            (export main)))
  (output (: true Bool)))

(case "the sequent-shaped kernel Thm is unforgeable — building Thm.Seq outside the kernel is CDZ0214"
  (doc    "The soundness boundary re-asserted for the REALISTIC sequent Thm (not the toy Thm(MkThm Int64)
           of the earlier cases). Even though Term is exported CONCRETELY (an importer can build any term
           it likes), Thm's constructor `Seq` is withheld — so an attacker cannot fabricate a bogus
           theorem `{} ⊢ (Var 1 = Var 2)` by calling `Thm.Seq` directly; that is CDZ0214. This is the
           crux: the ability to build TERMS freely does not grant the ability to assert them as THEOREMS.
           A theorem is minted only by a kernel rule; term construction is not a trust surface.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term))
      (type Thm (Seq (List Term) Term))
      (def (refl (: t Term)) (Thm.Seq (list) (Term.Eq t t)))
      (export (. Term *))
      (export Thm)
      (export refl)))
  (input  (do
            (import "hol" (Term Thm refl))
            (def (main) (Thm.Seq (list) (Term.Eq (Term.Var 1) (Term.Var 2))))
            (export main)))
  (error  CDZ0214))

; ============================================================================================
; Increment 3 — the rest of the equational-core primitive inference rules, and a multi-step proof.
; Building on the Inc-2 skeleton (Term concrete / Thm abstract sequent / refl / assume / term-eq), this
; adds the derived-from-primitive rules that make the kernel usable: TRANS (chaining equalities),
; MK_COMB (congruence: equals applied to equals are equal), EQ_MP (from ⊢p=q and G⊢p derive G⊢q,
; unioning hypotheses), and DEDUCT_ANTISYM (from A⊢p and B⊢q derive (A-q)∪(B-p)⊢p=q — the rule that
; builds an equality from bidirectional entailment, discharging the matched hypotheses). Each rule mints
; a Thm ONLY through the private constructor and checks its premises with the recursive term-eq, so a
; malformed application yields Option.None (a non-theorem) rather than an unsound Thm. The final case
; composes several rules into a single derivation, exercising the kernel as a real proof engine.
; ============================================================================================

(case "the kernel's MK_COMB rule: equals applied to equals are equal — from f=g and x=y derive (f x)=(g y)"
  (doc    "Congruence. MK_COMB takes ⊢ f = g and ⊢ x = y and derives ⊢ (f x) = (g y): the theorem that
           applying equal functions to equal arguments yields equal results. The rule reads the two
           premise conclusions (each an Eq), and mints the application-equality only through the private
           Thm constructor. Here from refl(f) : ⊢ f=f and refl(x) : ⊢ x=x it derives ⊢ (f x) = (f x),
           whose two sides term-eq. Pins that a rule CONSUMING two theorems and PRODUCING a structurally
           larger one composes correctly through the abstract boundary.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) ((Term.Comb _ _) false) ((Term.Eq _ _) false)))
          ((Term.Comb x y) (match b ((Term.Var _) false) ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) ((Term.Eq _ _) false)))
          ((Term.Eq x y)   (match b ((Term.Var _) false) ((Term.Comb _ _) false) ((Term.Eq p q) (and (term-eq x p) (term-eq y q)))))))
      (def (refl (: t Term)) (Thm.Seq (list) (Term.Eq t t)))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (mk-comb (: th1 Thm) (: th2 Thm))
        (match (concl th1)
          ((Term.Eq f g)
            (match (concl th2)
              ((Term.Eq x y) (Option.Some (Thm.Seq (List.concat (hyps th1) (hyps th2)) (Term.Eq (Term.Comb f x) (Term.Comb g y)))))
              (_ (Option.None))))
          (_ (Option.None))))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export refl)
      (export mk-comb)
      (export concl)))
  (input  (do
            (import "hol" (Term Thm term-eq refl mk-comb concl))
            (def (main)
              (let ((f (Term.Var 0)) (x (Term.Var 1)))
                (match (mk-comb (refl f) (refl x))
                  ((Option.Some th)
                    (match (concl th) ((Term.Eq l r) (term-eq l r)) (_ false)))
                  ((Option.None) false))))
            (export main)))
  (output (: true Bool)))

(case "the kernel's EQ_MP rule: from |- p=q and G |- p derive G |- q (hypotheses unioned)"
  (doc    "Modus ponens for equality — the rule that lets a proof MOVE across a proven equality. EQ_MP
           takes ⊢ p = q and G ⊢ p and derives G ⊢ q, checking (via term-eq) that the second theorem's
           conclusion really is the left side p, and UNIONING the two hypothesis sets (List.concat). Here
           from refl(p) : ⊢ p=p and assume(p) : {p} ⊢ p it derives {p} ⊢ p. Pins hypothesis-threading: a
           theorem's hyp list survives the rule and the derived theorem carries the union. A mismatch
           (concl ≠ p) yields Option.None, never a forged theorem.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) ((Term.Comb _ _) false) ((Term.Eq _ _) false)))
          ((Term.Comb x y) (match b ((Term.Var _) false) ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) ((Term.Eq _ _) false)))
          ((Term.Eq x y)   (match b ((Term.Var _) false) ((Term.Comb _ _) false) ((Term.Eq p q) (and (term-eq x p) (term-eq y q)))))))
      (def (refl (: t Term)) (Thm.Seq (list) (Term.Eq t t)))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (eq-mp (: eq Thm) (: thm Thm))
        (match (concl eq)
          ((Term.Eq p q)
            (if (term-eq (concl thm) p)
                (Option.Some (Thm.Seq (List.concat (hyps eq) (hyps thm)) q))
                (Option.None)))
          (_ (Option.None))))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export refl)
      (export assume)
      (export eq-mp)
      (export concl)
      (export hyps)))
  (input  (do
            (import "hol" (Term Thm term-eq refl assume eq-mp concl hyps))
            (def (main)
              (let ((p (Term.Var 5)))
                (match (eq-mp (refl p) (assume p))
                  ((Option.Some th)
                    (and (term-eq (concl th) p)
                         (match (hyps th) ((list h) (term-eq h p)) (_ false))))
                  ((Option.None) false))))
            (export main)))
  (output (: true Bool)))

(case "the kernel's DEDUCT_ANTISYM rule: from A |- p and B |- q derive (A-q)++(B-p) |- p=q"
  (doc    "The rule that BUILDS an equality from bidirectional entailment (HOL's DEDUCT_ANTISYM_RULE):
           from A ⊢ p and B ⊢ q derive (A − q) ∪ (B − p) ⊢ p = q, discharging q from A's hypotheses and p
           from B's. It exercises a recursive `remove` over a hypothesis list (a leading-rest list pattern
           `(list h .. rest)` + term-eq + List.push) — the kernel's most structurally involved hypothesis
           manipulation. From assume(p) : {p}⊢p and assume(q) : {q}⊢q it derives ({p}−q) ∪ ({q}−p) ⊢ p=q =
           {p,q} ⊢ p=q, whose conclusion is the equality p=q. Pins that a rule reshaping hypothesis sets
           (not just unioning them) composes correctly and mints only through the private constructor.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) ((Term.Comb _ _) false) ((Term.Eq _ _) false)))
          ((Term.Comb x y) (match b ((Term.Var _) false) ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) ((Term.Eq _ _) false)))
          ((Term.Eq x y)   (match b ((Term.Var _) false) ((Term.Comb _ _) false) ((Term.Eq p q) (and (term-eq x p) (term-eq y q)))))))
      (def (remove (: t Term) (: hs (List Term)))
        (match hs
          ((list) (list))
          ((list h .. rest) (if (term-eq h t) (remove t rest) (List.push (remove t rest) h)))))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (deduct (: th1 Thm) (: th2 Thm))
        (match th1 ((Thm.Seq h1 p)
          (match th2 ((Thm.Seq h2 q)
            (Thm.Seq (List.concat (remove q h1) (remove p h2)) (Term.Eq p q)))))))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export assume)
      (export deduct)
      (export concl)))
  (input  (do
            (import "hol" (Term Thm term-eq assume deduct concl))
            (def (main)
              (let ((p (Term.Var 1)) (q (Term.Var 2)))
                (match (concl (deduct (assume p) (assume q)))
                  ((Term.Eq l r) (and (term-eq l p) (term-eq r q)))
                  (_ false))))
            (export main)))
  (output (: true Bool)))

(case "a multi-step kernel derivation composes several primitive rules into one theorem"
  (doc    "The kernel as a real proof engine: a derivation chaining TRANS and MK_COMB. From refl we get
           ⊢ a=a and ⊢ b=b; MK_COMB gives ⊢ (a b)=(a b); TRANS of ⊢(a b)=(a b) with itself again gives
           ⊢ (a b)=(a b). The point is not the (trivial) theorem but that MULTIPLE rules compose — each
           consuming theorems only obtainable from prior rules, each minting through the private
           constructor — and the final conclusion is the expected equality. This is the shape every real
           HOL proof takes: primitive rules threaded into a derivation, with the kernel the sole minter.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) ((Term.Comb _ _) false) ((Term.Eq _ _) false)))
          ((Term.Comb x y) (match b ((Term.Var _) false) ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) ((Term.Eq _ _) false)))
          ((Term.Eq x y)   (match b ((Term.Var _) false) ((Term.Comb _ _) false) ((Term.Eq p q) (and (term-eq x p) (term-eq y q)))))))
      (def (refl (: t Term)) (Thm.Seq (list) (Term.Eq t t)))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (mk-comb (: th1 Thm) (: th2 Thm))
        (match (concl th1)
          ((Term.Eq f g)
            (match (concl th2)
              ((Term.Eq x y) (Option.Some (Thm.Seq (List.concat (hyps th1) (hyps th2)) (Term.Eq (Term.Comb f x) (Term.Comb g y)))))
              (_ (Option.None))))
          (_ (Option.None))))
      (def (trans (: th1 Thm) (: th2 Thm))
        (match (concl th1)
          ((Term.Eq a b)
            (match (concl th2)
              ((Term.Eq b2 c) (if (term-eq b b2) (Option.Some (Thm.Seq (List.concat (hyps th1) (hyps th2)) (Term.Eq a c))) (Option.None)))
              (_ (Option.None))))
          (_ (Option.None))))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export refl)
      (export mk-comb)
      (export trans)
      (export concl)))
  (input  (do
            (import "hol" (Term Thm term-eq refl mk-comb trans concl))
            (def (main)
              (let ((a (Term.Var 0)) (b (Term.Var 1)))
                (match (mk-comb (refl a) (refl b))
                  ((Option.Some th-ab)
                    (match (trans th-ab th-ab)
                      ((Option.Some th)
                        (match (concl th)
                          ((Term.Eq l r) (term-eq l r))
                          (_ false)))
                      ((Option.None) false)))
                  ((Option.None) false))))
            (export main)))
  (output (: true Bool)))

; ============================================================================================
; Increment 4 — the λ-calculus layer: Abs (lambda), capture-naive substitution, and the BETA and ABS
; primitive rules, culminating in the FIRST-THEOREM milestone ⊢ (λx.x) y = y. This extends the Term sum
; with an Abs binder (Abs varid body) and gives the kernel `subst` (a recursive replacement of a free
; variable through the whole Term, including under binders — the mechanism a real HOL kernel needs and
; the part most likely to strain the language: recursion over a binding sum). BETA_CONV mints
; ⊢ (λv.body) arg = body[arg/v]; ABS lifts an equational theorem under a binder (⊢ t=u ⟹ ⊢ (λx.t)=(λx.u)).
; The identity theorem ⊢ (λx.x) y = y is the design doc's §2 first-theorem milestone — a genuine result
; about the λ-calculus, derived through a kernel rule, not asserted. (α-conversion / a fresh-variable
; capture-avoiding substitution is a later increment; these cases use distinct ids so naive subst is
; sound for them.)
; ============================================================================================

(case "the kernel's BETA rule reduces an application of a lambda: (λv.body) arg = body[arg/v]"
  (doc    "BETA_CONV, the β-reduction rule. The kernel gains an Abs binder on Term and a recursive `subst`
           (replace free (Var v) by s through Comb/Eq/Abs, not descending into a shadowing binder). BETA
           mints ⊢ ((λv.body) arg) = body[arg/v] through the private Thm constructor. Here (λx0.x0) applied
           to (Var 9) reduces: the conclusion's right side is (Var 9) = the substituted body. Pins that the
           substitution machinery — recursion over a BINDING sum, the part most likely to strain the
           language — compiles and folds correctly, and that β-reduction is a kernel-minted theorem.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Abs v x)  (match b ((Term.Abs w q) (and (= v w) (term-eq x q))) (_ false)))))
      (def (subst (: v Int64) (: s Term) (: t Term))
        (match t
          ((Term.Var n)   (if (= n v) s (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
          ((Term.Eq a b)  (Term.Eq (subst v s a) (subst v s b)))
          ((Term.Abs w body) (if (= w v) (Term.Abs w body) (Term.Abs w (subst v s body))))))
      (def (beta (: v Int64) (: body Term) (: arg Term))
        (Thm.Seq (list) (Term.Eq (Term.Comb (Term.Abs v body) arg) (subst v arg body))))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export subst)
      (export beta)
      (export concl)))
  (input  (do
            (import "hol" (Term Thm term-eq subst beta concl))
            (def (main)
              (match (concl (beta 0 (Term.Var 0) (Term.Var 9)))
                ((Term.Eq lhs rhs) (term-eq rhs (Term.Var 9)))
                (_ false)))
            (export main)))
  (output (: true Bool)))

(case "the kernel proves the identity theorem ⊢ (λx.x) y = y via BETA — the first-theorem milestone"
  (doc    "The design doc's §2 first-theorem milestone: a genuine result about the λ-calculus, DERIVED
           through the kernel rather than asserted. BETA on the identity combinator (λx0.x0) applied to y
           (= Var 42) yields ⊢ ((λx0.x0) y) = y. The case checks the left side is the identity applied to
           y and the right side is exactly y — so the kernel has PROVED the identity function returns its
           argument. This is the payoff of the equational + λ core: real theorems, minted only by rules.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Abs v x)  (match b ((Term.Abs w q) (and (= v w) (term-eq x q))) (_ false)))))
      (def (subst (: v Int64) (: s Term) (: t Term))
        (match t
          ((Term.Var n)   (if (= n v) s (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
          ((Term.Eq a b)  (Term.Eq (subst v s a) (subst v s b)))
          ((Term.Abs w body) (if (= w v) (Term.Abs w body) (Term.Abs w (subst v s body))))))
      (def (beta (: v Int64) (: body Term) (: arg Term))
        (Thm.Seq (list) (Term.Eq (Term.Comb (Term.Abs v body) arg) (subst v arg body))))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (id-fn) (Term.Abs 0 (Term.Var 0)))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export beta)
      (export concl)
      (export id-fn)))
  (input  (do
            (import "hol" (Term Thm term-eq beta concl id-fn))
            (def (main)
              (let ((y (Term.Var 42)))
                (match (concl (beta 0 (Term.Var 0) y))
                  ((Term.Eq lhs rhs)
                    (and (term-eq lhs (Term.Comb (id-fn) y))
                         (term-eq rhs y)))
                  (_ false))))
            (export main)))
  (output (: true Bool)))

(case "the kernel's ABS rule lifts an equation under a binder: from |- t=u derive |- (λx.t)=(λx.u)"
  (doc    "ABS, the rule that abstracts an equational theorem under a lambda: from G ⊢ t = u derive
           G ⊢ (λx.t) = (λx.u) (the free-variable side-condition on x is a later increment). From
           refl(Var 5) : ⊢ (Var 5)=(Var 5), ABS 0 yields ⊢ (λx0.Var5) = (λx0.Var5); the case verifies the
           left side is the expected abstraction. Pins that a rule PRODUCING a binder from an equational
           premise composes through the abstract boundary — the congruence rule for lambdas that, with
           BETA and MK_COMB, makes the λ-fragment usable.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Abs v x)  (match b ((Term.Abs w q) (and (= v w) (term-eq x q))) (_ false)))))
      (def (refl (: t Term)) (Thm.Seq (list) (Term.Eq t t)))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (abs-rule (: x Int64) (: th Thm))
        (match th ((Thm.Seq g c)
          (match c
            ((Term.Eq t u) (Option.Some (Thm.Seq g (Term.Eq (Term.Abs x t) (Term.Abs x u)))))
            (_ (Option.None))))))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export refl)
      (export abs-rule)
      (export concl)))
  (input  (do
            (import "hol" (Term Thm term-eq refl abs-rule concl))
            (def (main)
              (match (abs-rule 0 (refl (Term.Var 5)))
                ((Option.Some th)
                  (match (concl th)
                    ((Term.Eq l r) (term-eq l (Term.Abs 0 (Term.Var 5))))
                    (_ false)))
                ((Option.None) false)))
            (export main)))
  (output (: true Bool)))

(case "the λ-extended kernel Thm stays unforgeable — Thm.Seq with an Abs term outside is CDZ0214"
  (doc    "Re-asserts the soundness boundary after extending Term with the Abs binder: adding a term form
           does not open a forge path. An importer cannot fabricate a theorem about lambdas — building
           Thm.Seq directly (here a bogus ⊢ (λx0.x0) = (Var 1)) outside the kernel is CDZ0214. The λ-layer
           is exercised through the exported rules (beta/abs) only; term construction, including Abs, is
           free but confers no power to assert theorems.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
      (type Thm (Seq (List Term) Term))
      (def (refl (: t Term)) (Thm.Seq (list) (Term.Eq t t)))
      (export (. Term *))
      (export Thm)
      (export refl)))
  (input  (do
            (import "hol" (Term Thm refl))
            (def (main) (Thm.Seq (list) (Term.Eq (Term.Abs 0 (Term.Var 0)) (Term.Var 1))))
            (export main)))
  (error  CDZ0214))

; ============================================================================================
; Increment 5 — CAPTURE-AVOIDING substitution: the soundness fix for the λ-layer. The Inc-4 `subst` is
; NAIVE — substituting a term with a free variable `x` under a binder `λx.…` would CAPTURE it (bind the
; free x), which in a real kernel lets you prove FALSE theorems. This increment gives the kernel a
; capture-avoiding subst: when the substituted term's free variables would be captured by a binder, the
; binder is α-renamed to a FRESH variable first. It needs three supporting functions the kernel walks
; over the recursive Term: `free-in` (is v free in t?), `max-id` (largest id, for fresh generation), and
; `rename` (α-rename a binder). These cases pin BOTH faces — the naive version DOES capture (the bug this
; prevents) and the capture-avoiding version does NOT — and that BETA over the safe subst still proves the
; identity theorem (no regression). This makes the λ-fragment SOUND, not just mechanical.
; ============================================================================================

(case "capture-avoiding substitution renames a binder so the substituted term's free variable is not captured"
  (doc    "The soundness fix. Substituting s = (Var 0) for y (=x1) into (λx0. x1) must NOT capture: the
           binder x0 is a free variable of s, so a correct subst α-renames x0 to a fresh id before
           descending, leaving the substituted (Var 0) FREE in the result. The case checks `free-in 0
           result` is TRUE — the (Var 0) survived uncaptured. A naive (capturing) subst would bind it and
           this would be FALSE (see the control below). Pins that the kernel's substitution is
           capture-avoiding — the property that makes β-reduction and INST sound rather than able to
           derive false equalities. `free-in`, `max-id`, `rename`, and the capture-aware `subst` all fold
           over the recursive Term (with binders) correctly on trunk.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
      (def (free-in (: v Int64) (: t Term))
        (match t
          ((Term.Var n)   (= n v))
          ((Term.Comb f x) (or (free-in v f) (free-in v x)))
          ((Term.Eq a b)  (or (free-in v a) (free-in v b)))
          ((Term.Abs w body) (if (= w v) false (free-in v body)))))
      (def (max-id (: t Term))
        (match t
          ((Term.Var n)   n)
          ((Term.Comb f x) (let ((a (max-id f)) (b (max-id x))) (if (> a b) a b)))
          ((Term.Eq a b)  (let ((p (max-id a)) (q (max-id b))) (if (> p q) p q)))
          ((Term.Abs w body) (let ((m (max-id body))) (if (> w m) w m)))))
      (def (rename (: from Int64) (: to Int64) (: t Term))
        (match t
          ((Term.Var n)   (if (= n from) (Term.Var to) (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (rename from to f) (rename from to x)))
          ((Term.Eq a b)  (Term.Eq (rename from to a) (rename from to b)))
          ((Term.Abs w body) (if (= w from) (Term.Abs w body) (Term.Abs w (rename from to body))))))
      (def (subst (: v Int64) (: s Term) (: t Term))
        (match t
          ((Term.Var n)   (if (= n v) s (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
          ((Term.Eq a b)  (Term.Eq (subst v s a) (subst v s b)))
          ((Term.Abs w body)
            (if (= w v)
                (Term.Abs w body)
                (if (free-in w s)
                    (let ((fresh (+ 1 (let ((ms (max-id s)) (mt (max-id body))) (if (> ms mt) ms mt)))))
                      (Term.Abs fresh (subst v s (rename w fresh body))))
                    (Term.Abs w (subst v s body)))))))
      (export (. Term *))
      (export free-in)
      (export subst)))
  (input  (do
            (import "hol" (Term free-in subst))
            (def (main)
              (let ((s (Term.Var 0))
                    (body (Term.Abs 0 (Term.Var 1))))
                (free-in 0 (subst 1 s body))))
            (export main)))
  (output (: true Bool)))

(case "a naive (non-renaming) substitution captures a free variable — the bug capture-avoidance prevents"
  (doc    "The control that gives the pin above its teeth: a NAIVE subst that does not α-rename binders
           substitutes s = (Var 0) for y into (λx0. y) yielding (λx0. x0) — the free (Var 0) is now BOUND
           by the binder (captured). `free-in 0 result` is FALSE. This is the exact unsoundness the Inc-5
           capture-avoiding subst prevents; pinning it ensures a future 'simplification' that drops the
           renaming would flip this case and be caught. (A kernel with capturing substitution can derive
           false theorems, so this is a soundness — not merely a hygiene — property.)")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
      (def (free-in (: v Int64) (: t Term))
        (match t
          ((Term.Var n)   (= n v))
          ((Term.Comb f x) (or (free-in v f) (free-in v x)))
          ((Term.Eq a b)  (or (free-in v a) (free-in v b)))
          ((Term.Abs w body) (if (= w v) false (free-in v body)))))
      (def (naive-subst (: v Int64) (: s Term) (: t Term))
        (match t
          ((Term.Var n)   (if (= n v) s (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (naive-subst v s f) (naive-subst v s x)))
          ((Term.Eq a b)  (Term.Eq (naive-subst v s a) (naive-subst v s b)))
          ((Term.Abs w body) (if (= w v) (Term.Abs w body) (Term.Abs w (naive-subst v s body))))))
      (export (. Term *))
      (export free-in)
      (export naive-subst)))
  (input  (do
            (import "hol" (Term free-in naive-subst))
            (def (main)
              (free-in 0 (naive-subst 1 (Term.Var 0) (Term.Abs 0 (Term.Var 1)))))
            (export main)))
  (output (: false Bool)))

(case "BETA over the capture-avoiding substitution still proves the identity theorem (no regression)"
  (doc    "Guards that hardening subst to be capture-avoiding did not break β-reduction: BETA on the
           identity combinator (λx0.x0) applied to y (=Var 42), using the capture-avoiding subst, still
           yields ⊢ ((λx0.x0) y) = y — the conclusion's right side is y. Pins that the soundness fix
           composes with the primitive rules from Inc-4 (the identity theorem still holds).")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Abs v x)  (match b ((Term.Abs w q) (and (= v w) (term-eq x q))) (_ false)))))
      (def (free-in (: v Int64) (: t Term))
        (match t
          ((Term.Var n)   (= n v))
          ((Term.Comb f x) (or (free-in v f) (free-in v x)))
          ((Term.Eq a b)  (or (free-in v a) (free-in v b)))
          ((Term.Abs w body) (if (= w v) false (free-in v body)))))
      (def (max-id (: t Term))
        (match t
          ((Term.Var n)   n)
          ((Term.Comb f x) (let ((a (max-id f)) (b (max-id x))) (if (> a b) a b)))
          ((Term.Eq a b)  (let ((p (max-id a)) (q (max-id b))) (if (> p q) p q)))
          ((Term.Abs w body) (let ((m (max-id body))) (if (> w m) w m)))))
      (def (rename (: from Int64) (: to Int64) (: t Term))
        (match t
          ((Term.Var n)   (if (= n from) (Term.Var to) (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (rename from to f) (rename from to x)))
          ((Term.Eq a b)  (Term.Eq (rename from to a) (rename from to b)))
          ((Term.Abs w body) (if (= w from) (Term.Abs w body) (Term.Abs w (rename from to body))))))
      (def (subst (: v Int64) (: s Term) (: t Term))
        (match t
          ((Term.Var n)   (if (= n v) s (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
          ((Term.Eq a b)  (Term.Eq (subst v s a) (subst v s b)))
          ((Term.Abs w body)
            (if (= w v)
                (Term.Abs w body)
                (if (free-in w s)
                    (let ((fresh (+ 1 (let ((ms (max-id s)) (mt (max-id body))) (if (> ms mt) ms mt)))))
                      (Term.Abs fresh (subst v s (rename w fresh body))))
                    (Term.Abs w (subst v s body)))))))
      (def (beta (: v Int64) (: body Term) (: arg Term))
        (Thm.Seq (list) (Term.Eq (Term.Comb (Term.Abs v body) arg) (subst v arg body))))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export beta)
      (export concl)))
  (input  (do
            (import "hol" (Term Thm term-eq beta concl))
            (def (main)
              (let ((y (Term.Var 42)))
                (match (concl (beta 0 (Term.Var 0) y))
                  ((Term.Eq lhs rhs) (term-eq rhs y))
                  (_ false))))
            (export main)))
  (output (: true Bool)))

; ============================================================================================
; Increment 6 — α-EQUIVALENCE (aconv): the term equality a real HOL kernel actually uses. The structural
; term-eq of Inc-2/3 says (λx0.x0) and (λx1.x1) DIFFER (0 ≠ 1) — but they are the SAME function, differing
; only in the bound variable's name. aconv is α-equivalence: two terms are equal up to consistent renaming
; of BOUND variables (free variables must match exactly). It is implemented by a parallel walk carrying two
; binder stacks; a variable is α-equal iff it is bound at the same depth on both sides, or both free and
; numerically equal. This is the correct equality for a rule like EQ_MP (a premise may be an α-variant of
; the expected term) — using structural equality there would spuriously reject sound proofs. These cases
; pin aconv's positive/negative behavior, its subtle bound-vs-free edges, and its use inside a kernel rule.
; ============================================================================================

(case "α-equivalence (aconv) recognizes (λx.x) and (λy.y) as the same term up to bound-variable renaming"
  (doc    "The core of α-equivalence. (λx0.x0) and (λx1.x1) are both the identity function — the same
           term modulo the bound variable's name — so aconv is TRUE, even though structural term-eq (0≠1)
           would say false. Conversely (λx0.x0) and (λx0.(Var 9)) are NOT α-equivalent (one binds its
           variable, the other returns a free x9). Implemented by a parallel walk over two binder stacks
           (List.push on Abs entry; depth-of lookup): a Var is α-equal iff bound at the same depth on both
           sides, or both free and equal. Pins the term equality a real HOL kernel uses.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
      (def (depth-of (: v Int64) (: stack (List Int64)))
        (match stack
          ((list) (- 0 1))
          ((list top .. rest) (if (= top v) 0 (let ((d (depth-of v rest))) (if (< d 0) d (+ d 1)))))))
      (def (aconv-env (: sa (List Int64)) (: sb (List Int64)) (: a Term) (: b Term))
        (match a
          ((Term.Var n)
            (match b ((Term.Var m)
              (let ((da (depth-of n sa)) (db (depth-of m sb)))
                (if (< da 0) (and (< db 0) (= n m)) (= da db)))) (_ false)))
          ((Term.Comb f x) (match b ((Term.Comb g y) (and (aconv-env sa sb f g) (aconv-env sa sb x y))) (_ false)))
          ((Term.Eq p q)   (match b ((Term.Eq r s) (and (aconv-env sa sb p r) (aconv-env sa sb q s))) (_ false)))
          ((Term.Abs v body) (match b ((Term.Abs w body2) (aconv-env (List.push sa v) (List.push sb w) body body2)) (_ false)))))
      (def (aconv (: a Term) (: b Term)) (aconv-env (list) (list) a b))
      (export (. Term *))
      (export aconv)))
  (input  (do
            (import "hol" (Term aconv))
            (def (main)
              (and (aconv (Term.Abs 0 (Term.Var 0)) (Term.Abs 1 (Term.Var 1)))
                   (not (aconv (Term.Abs 0 (Term.Var 0)) (Term.Abs 0 (Term.Var 9))))))
            (export main)))
  (output (: true Bool)))

(case "α-equivalence handles nesting and distinguishes a bound variable from a same-numbered free variable"
  (doc    "The subtle correctness edges that make aconv a REAL α-equivalence, not a toy: (a) nested
           binders rename consistently — (λx0.λx1. x0 x1) is α-equal to (λx7.λx3. x7 x3); (b) a FREE
           variable must match EXACTLY — (λx0. x5) ≡ (λx9. x5) but not ≡ (λx9. x6); (c) crucially, a BOUND
           variable is NOT α-equal to a same-numbered FREE variable — (λx0.x0) is not (λx0.(Var 5)) (the
           second's body is a free x5, not the bound x0). Pins that aconv tracks binding depth, not raw
           ids, so it neither conflates free vars nor treats a coincidental id match as α-equality.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
      (def (depth-of (: v Int64) (: stack (List Int64)))
        (match stack
          ((list) (- 0 1))
          ((list top .. rest) (if (= top v) 0 (let ((d (depth-of v rest))) (if (< d 0) d (+ d 1)))))))
      (def (aconv-env (: sa (List Int64)) (: sb (List Int64)) (: a Term) (: b Term))
        (match a
          ((Term.Var n)
            (match b ((Term.Var m)
              (let ((da (depth-of n sa)) (db (depth-of m sb)))
                (if (< da 0) (and (< db 0) (= n m)) (= da db)))) (_ false)))
          ((Term.Comb f x) (match b ((Term.Comb g y) (and (aconv-env sa sb f g) (aconv-env sa sb x y))) (_ false)))
          ((Term.Eq p q)   (match b ((Term.Eq r s) (and (aconv-env sa sb p r) (aconv-env sa sb q s))) (_ false)))
          ((Term.Abs v body) (match b ((Term.Abs w body2) (aconv-env (List.push sa v) (List.push sb w) body body2)) (_ false)))))
      (def (aconv (: a Term) (: b Term)) (aconv-env (list) (list) a b))
      (export (. Term *))
      (export aconv)))
  (input  (do
            (import "hol" (Term aconv))
            (def (main)
              (and
                (aconv (Term.Abs 0 (Term.Abs 1 (Term.Comb (Term.Var 0) (Term.Var 1))))
                       (Term.Abs 7 (Term.Abs 3 (Term.Comb (Term.Var 7) (Term.Var 3)))))
                (and (aconv (Term.Abs 0 (Term.Var 5)) (Term.Abs 9 (Term.Var 5)))
                     (and (not (aconv (Term.Abs 0 (Term.Var 5)) (Term.Abs 9 (Term.Var 6))))
                          (not (aconv (Term.Abs 0 (Term.Var 0)) (Term.Abs 0 (Term.Var 5))))))))
            (export main)))
  (output (: true Bool)))

(case "a kernel rule using α-equivalence accepts an α-variant premise a structural equality would reject"
  (doc    "Why the kernel NEEDS aconv, not structural term-eq: EQ_MP takes ⊢ p=q and a theorem whose
           conclusion is p, deriving ⊢ q. If the second theorem's conclusion is an α-VARIANT of p — the
           same term with a renamed bound variable — a sound kernel must accept it. Here p = (λx0.x0),
           the premise theorem's conclusion is (λx5.x5) (α-equivalent to p), and EQ_MP checks the match
           with aconv → succeeds, deriving ⊢ q = (Var 100). A structural-equality EQ_MP would spuriously
           REJECT (0 ≠ 5), blocking sound proofs. Pins that α-equivalence is the correct premise-matching
           equality for the inference rules. (eq/thm are built here via `assume` for a self-contained
           witness; the α-matching in eq-mp is the point.)")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
      (type Thm (Seq (List Term) Term))
      (def (depth-of (: v Int64) (: stack (List Int64)))
        (match stack
          ((list) (- 0 1))
          ((list top .. rest) (if (= top v) 0 (let ((d (depth-of v rest))) (if (< d 0) d (+ d 1)))))))
      (def (aconv-env (: sa (List Int64)) (: sb (List Int64)) (: a Term) (: b Term))
        (match a
          ((Term.Var n)
            (match b ((Term.Var m)
              (let ((da (depth-of n sa)) (db (depth-of m sb)))
                (if (< da 0) (and (< db 0) (= n m)) (= da db)))) (_ false)))
          ((Term.Comb f x) (match b ((Term.Comb g y) (and (aconv-env sa sb f g) (aconv-env sa sb x y))) (_ false)))
          ((Term.Eq p q)   (match b ((Term.Eq r s) (and (aconv-env sa sb p r) (aconv-env sa sb q s))) (_ false)))
          ((Term.Abs v body) (match b ((Term.Abs w body2) (aconv-env (List.push sa v) (List.push sb w) body body2)) (_ false)))))
      (def (aconv (: a Term) (: b Term)) (aconv-env (list) (list) a b))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (eq-mp (: eq Thm) (: thm Thm))
        (match (concl eq)
          ((Term.Eq p q) (if (aconv (concl thm) p) (Option.Some (Thm.Seq (list) q)) (Option.None)))
          (_ (Option.None))))
      (export (. Term *))
      (export Thm)
      (export aconv)
      (export assume)
      (export eq-mp)
      (export concl)))
  (input  (do
            (import "hol" (Term Thm aconv assume eq-mp concl))
            (def (main)
              (let ((p (Term.Abs 0 (Term.Var 0)))
                    (q (Term.Var 100))
                    (p-variant (Term.Abs 5 (Term.Var 5))))
                (let ((eq (assume (Term.Eq p q)))
                      (thm (assume p-variant)))
                  (match (eq-mp eq thm)
                    ((Option.Some r) (match (concl r) ((Term.Var n) (= n 100)) (_ false)))
                    ((Option.None) false)))))
            (export main)))
  (output (: true Bool)))

; ============================================================================================
; Increment 7 — the IMPLICATION fragment and the FLAGSHIP LOGICAL THEOREM ⊢ p ⇒ p. This lifts the kernel
; from equational/λ reasoning to genuine LOGIC. Term gains an `Imp` form (p ⇒ q); the kernel gains two
; natural-deduction rules for it: DISCH (⇒-introduction — from G ⊢ q derive (G − p) ⊢ (p ⇒ q), discharging
; the assumption p) and MP (⇒-elimination / modus ponens — from ⊢ p ⇒ q and ⊢ p derive ⊢ q). The theorem
; ⊢ p ⇒ p — proved by DISCH over ASSUME — is the first LOGICAL (not merely equational) theorem the kernel
; derives: a real tautology, established through the rules, with its assumption correctly discharged.
; (This models ⇒ as a primitive term form with introduction/elimination rules — the natural-deduction
; presentation — rather than HOL-Light's ⇒-as-a-defined-constant, which would first need T/∧ and
; new_basic_definition. Both are sound; the primitive presentation is the cleaner slice and still yields
; the genuine ⊢ p ⇒ p. A defined-constant logical layer with the three HOL axioms can follow.)
; ============================================================================================

(case "the kernel proves the tautology ⊢ p ⇒ p via DISCH (implication introduction) over ASSUME"
  (doc    "The FLAGSHIP logical theorem — the kernel's first genuine tautology. ASSUME p gives {p} ⊢ p;
           DISCH p discharges the assumption, yielding ⊢ (p ⇒ p) with an EMPTY hypothesis set — a theorem
           that holds unconditionally. The case verifies both the conclusion is (Imp p p) AND the
           hypotheses are empty (p was discharged). This is the payoff of the whole kernel: a real logical
           truth, DERIVED through the inference rules (not asserted), minted only through the private Thm
           constructor. DISCH reuses the recursive `remove` over the hypothesis list.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term) (Imp Term Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Abs v x)  (match b ((Term.Abs w q) (and (= v w) (term-eq x q))) (_ false)))
          ((Term.Imp x y)  (match b ((Term.Imp p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (remove (: t Term) (: hs (List Term)))
        (match hs
          ((list) (list))
          ((list h .. rest) (if (term-eq h t) (remove t rest) (List.push (remove t rest) h)))))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (disch (: p Term) (: th Thm))
        (match th ((Thm.Seq g q) (Thm.Seq (remove p g) (Term.Imp p q)))))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export assume)
      (export disch)
      (export concl)
      (export hyps)))
  (input  (do
            (import "hol" (Term Thm term-eq assume disch concl hyps))
            (def (main)
              (let ((p (Term.Var 0)))
                (let ((th (disch p (assume p))))
                  (and (term-eq (concl th) (Term.Imp p p))
                       (match (hyps th) ((list) true) (_ false))))))
            (export main)))
  (output (: true Bool)))

(case "the kernel's modus ponens (⇒-elimination): from ⊢ p⇒q and ⊢ p derive ⊢ q"
  (doc    "The elimination rule dual to DISCH: MP takes ⊢ p ⇒ q and ⊢ p and derives ⊢ q, checking the
           antecedent matches (term-eq) and unioning hypotheses. Here from ⊢ (p ⇒ p) (built by DISCH over
           ASSUME) and {p} ⊢ p (ASSUME p), MP derives a theorem whose conclusion is p. Pins that the
           implication fragment is complete (introduction + elimination) and that MP mints only through the
           private constructor, returning Option.None on an antecedent mismatch rather than a forged Thm.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term) (Imp Term Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Abs v x)  (match b ((Term.Abs w q) (and (= v w) (term-eq x q))) (_ false)))
          ((Term.Imp x y)  (match b ((Term.Imp p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (remove (: t Term) (: hs (List Term)))
        (match hs
          ((list) (list))
          ((list h .. rest) (if (term-eq h t) (remove t rest) (List.push (remove t rest) h)))))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (disch (: p Term) (: th Thm))
        (match th ((Thm.Seq g q) (Thm.Seq (remove p g) (Term.Imp p q)))))
      (def (mp (: imp Thm) (: th Thm))
        (match (concl imp)
          ((Term.Imp p q) (if (term-eq (concl th) p) (Option.Some (Thm.Seq (List.concat (hyps imp) (hyps th)) q)) (Option.None)))
          (_ (Option.None))))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export assume)
      (export disch)
      (export mp)
      (export concl)))
  (input  (do
            (import "hol" (Term Thm term-eq assume disch mp concl))
            (def (main)
              (let ((p (Term.Var 0)))
                (let ((imp (disch p (assume p))))
                  (match (mp imp (assume p))
                    ((Option.Some r) (term-eq (concl r) p))
                    ((Option.None) false)))))
            (export main)))
  (output (: true Bool)))

(case "the implication-extended kernel Thm stays unforgeable — Thm.Seq of a bogus implication outside is CDZ0214"
  (doc    "Re-asserts the soundness boundary after adding the Imp term form and the DISCH/MP rules: the
           logical layer opens no forge path. An importer cannot fabricate a false implication theorem —
           building Thm.Seq directly (a bogus ⊢ (Var 1) ⇒ (Var 2), which does NOT hold) outside the kernel
           is CDZ0214. Logical connectives are terms an importer may build freely; asserting an implication
           as a THEOREM remains the exclusive province of the kernel's rules (DISCH).")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term) (Imp Term Term))
      (type Thm (Seq (List Term) Term))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (export (. Term *))
      (export Thm)
      (export assume)))
  (input  (do
            (import "hol" (Term Thm assume))
            (def (main) (Thm.Seq (list) (Term.Imp (Term.Var 1) (Term.Var 2))))
            (export main)))
  (error  CDZ0214))

; ============================================================================================
; Increment 8 — the UNIVERSAL QUANTIFIER (∀). Term gains a `Forall v body` form; the kernel gains GEN
; (∀-introduction — from G ⊢ P, if the variable is NOT free in any hypothesis, derive G ⊢ ∀x.P) and SPEC
; (∀-elimination — from ⊢ ∀x.P derive ⊢ P[t/x], instantiating with the witness t via the Inc-5
; capture-avoiding substitution). This takes the kernel from propositional to FIRST-ORDER logic. GEN's
; free-variable side-condition is the soundness guard (generalizing a variable that a hypothesis constrains
; would be unsound); the case pins that GEN DECLINES when it is violated. SPEC reuses the capture-avoiding
; subst extended to descend under Forall.
; ============================================================================================

(case "the kernel's GEN (∀-introduction) enforces its free-variable side-condition"
  (doc    "∀-introduction with its soundness guard. GEN x (G ⊢ P) derives G ⊢ (∀x.P) — BUT only if x is
           not free in any hypothesis G (generalizing a variable a hypothesis constrains would be unsound).
           Two faces: (a) GEN over a variable FREE in the hypotheses DECLINES (Option.None) — here x0 is
           free in the hyp {x0=x0} of ASSUME(x0=x0); (b) GEN over a variable NOT free in the hyps SUCCEEDS
           — x0 is not free in the hyp {x5} of ASSUME(x5), so GEN 0 yields ∀x0.(x5), then SPEC (x9) brings
           it back to (x5) (x0 not in the body, so the instantiation is identity). Pins that GEN carries the
           side-condition — the guard that keeps ∀-introduction sound — and composes with SPEC.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term) (Forall Int64 Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Abs v x)  (match b ((Term.Abs w q) (and (= v w) (term-eq x q))) (_ false)))
          ((Term.Forall v x) (match b ((Term.Forall w q) (and (= v w) (term-eq x q))) (_ false)))))
      (def (free-in (: v Int64) (: t Term))
        (match t
          ((Term.Var n)   (= n v))
          ((Term.Comb f x) (or (free-in v f) (free-in v x)))
          ((Term.Eq a b)  (or (free-in v a) (free-in v b)))
          ((Term.Abs w body) (if (= w v) false (free-in v body)))
          ((Term.Forall w body) (if (= w v) false (free-in v body)))))
      (def (max-id (: t Term))
        (match t
          ((Term.Var n) n)
          ((Term.Comb f x) (let ((a (max-id f)) (b (max-id x))) (if (> a b) a b)))
          ((Term.Eq a b) (let ((p (max-id a)) (q (max-id b))) (if (> p q) p q)))
          ((Term.Abs w body) (let ((m (max-id body))) (if (> w m) w m)))
          ((Term.Forall w body) (let ((m (max-id body))) (if (> w m) w m)))))
      (def (rename (: from Int64) (: to Int64) (: t Term))
        (match t
          ((Term.Var n) (if (= n from) (Term.Var to) (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (rename from to f) (rename from to x)))
          ((Term.Eq a b) (Term.Eq (rename from to a) (rename from to b)))
          ((Term.Abs w body) (if (= w from) (Term.Abs w body) (Term.Abs w (rename from to body))))
          ((Term.Forall w body) (if (= w from) (Term.Forall w body) (Term.Forall w (rename from to body))))))
      (def (subst (: v Int64) (: s Term) (: t Term))
        (match t
          ((Term.Var n) (if (= n v) s (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
          ((Term.Eq a b) (Term.Eq (subst v s a) (subst v s b)))
          ((Term.Abs w body)
            (if (= w v) (Term.Abs w body)
                (if (free-in w s)
                    (let ((fresh (+ 1 (let ((ms (max-id s)) (mt (max-id body))) (if (> ms mt) ms mt)))))
                      (Term.Abs fresh (subst v s (rename w fresh body))))
                    (Term.Abs w (subst v s body)))))
          ((Term.Forall w body)
            (if (= w v) (Term.Forall w body)
                (if (free-in w s)
                    (let ((fresh (+ 1 (let ((ms (max-id s)) (mt (max-id body))) (if (> ms mt) ms mt)))))
                      (Term.Forall fresh (subst v s (rename w fresh body))))
                    (Term.Forall w (subst v s body)))))))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (free-in-hyps (: v Int64) (: hs (List Term)))
        (match hs ((list) false) ((list h .. rest) (or (free-in v h) (free-in-hyps v rest)))))
      (def (gen (: x Int64) (: th Thm))
        (match th ((Thm.Seq g p)
          (if (free-in-hyps x g) (Option.None) (Option.Some (Thm.Seq g (Term.Forall x p)))))))
      (def (spec (: t Term) (: th Thm))
        (match (concl th)
          ((Term.Forall x body) (Option.Some (Thm.Seq (hyps th) (subst x t body))))
          (_ (Option.None))))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export assume)
      (export gen)
      (export spec)
      (export concl)))
  (input  (do
            (import "hol" (Term Thm term-eq assume gen spec concl))
            (def (main)
              (and
                (match (gen 0 (assume (Term.Eq (Term.Var 0) (Term.Var 0))))
                  ((Option.None) true) ((Option.Some _) false))
                (match (gen 0 (assume (Term.Var 5)))
                  ((Option.Some g)
                    (match (spec (Term.Var 9) g)
                      ((Option.Some s) (term-eq (concl s) (Term.Var 5)))
                      ((Option.None) false)))
                  ((Option.None) false))))
            (export main)))
  (output (: true Bool)))

(case "the kernel's SPEC (∀-elimination) instantiates the quantified body with the witness"
  (doc    "∀-elimination: from ⊢ ∀x.P derive ⊢ P[t/x], substituting the witness t for the bound variable
           through the body via the capture-avoiding subst. From ⊢ ∀x0.(x0 = x0) (a universally-quantified
           reflexivity, built here as a self-contained witness), SPEC (Var 42) yields ⊢ (Var 42 = Var 42) —
           the substitution FIRES, replacing both occurrences of the bound x0. Pins that SPEC genuinely
           instantiates (not a no-op) and reuses the Inc-5 capture-avoiding substitution under a binder.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term) (Forall Int64 Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Abs v x)  (match b ((Term.Abs w q) (and (= v w) (term-eq x q))) (_ false)))
          ((Term.Forall v x) (match b ((Term.Forall w q) (and (= v w) (term-eq x q))) (_ false)))))
      (def (free-in (: v Int64) (: t Term))
        (match t
          ((Term.Var n) (= n v))
          ((Term.Comb f x) (or (free-in v f) (free-in v x)))
          ((Term.Eq a b) (or (free-in v a) (free-in v b)))
          ((Term.Abs w body) (if (= w v) false (free-in v body)))
          ((Term.Forall w body) (if (= w v) false (free-in v body)))))
      (def (max-id (: t Term))
        (match t
          ((Term.Var n) n)
          ((Term.Comb f x) (let ((a (max-id f)) (b (max-id x))) (if (> a b) a b)))
          ((Term.Eq a b) (let ((p (max-id a)) (q (max-id b))) (if (> p q) p q)))
          ((Term.Abs w body) (let ((m (max-id body))) (if (> w m) w m)))
          ((Term.Forall w body) (let ((m (max-id body))) (if (> w m) w m)))))
      (def (rename (: from Int64) (: to Int64) (: t Term))
        (match t
          ((Term.Var n) (if (= n from) (Term.Var to) (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (rename from to f) (rename from to x)))
          ((Term.Eq a b) (Term.Eq (rename from to a) (rename from to b)))
          ((Term.Abs w body) (if (= w from) (Term.Abs w body) (Term.Abs w (rename from to body))))
          ((Term.Forall w body) (if (= w from) (Term.Forall w body) (Term.Forall w (rename from to body))))))
      (def (subst (: v Int64) (: s Term) (: t Term))
        (match t
          ((Term.Var n) (if (= n v) s (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
          ((Term.Eq a b) (Term.Eq (subst v s a) (subst v s b)))
          ((Term.Abs w body)
            (if (= w v) (Term.Abs w body)
                (if (free-in w s)
                    (let ((fresh (+ 1 (let ((ms (max-id s)) (mt (max-id body))) (if (> ms mt) ms mt)))))
                      (Term.Abs fresh (subst v s (rename w fresh body))))
                    (Term.Abs w (subst v s body)))))
          ((Term.Forall w body)
            (if (= w v) (Term.Forall w body)
                (if (free-in w s)
                    (let ((fresh (+ 1 (let ((ms (max-id s)) (mt (max-id body))) (if (> ms mt) ms mt)))))
                      (Term.Forall fresh (subst v s (rename w fresh body))))
                    (Term.Forall w (subst v s body)))))))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (refl-all (: x Int64)) (Thm.Seq (list) (Term.Forall x (Term.Eq (Term.Var x) (Term.Var x)))))
      (def (spec (: t Term) (: th Thm))
        (match (concl th)
          ((Term.Forall x body) (Option.Some (Thm.Seq (hyps th) (subst x t body))))
          (_ (Option.None))))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export refl-all)
      (export spec)
      (export concl)))
  (input  (do
            (import "hol" (Term Thm term-eq refl-all spec concl))
            (def (main)
              (match (spec (Term.Var 42) (refl-all 0))
                ((Option.Some s) (term-eq (concl s) (Term.Eq (Term.Var 42) (Term.Var 42))))
                ((Option.None) false)))
            (export main)))
  (output (: true Bool)))

(case "the ∀-extended kernel Thm stays unforgeable — Thm.Seq of a bogus universal outside is CDZ0214"
  (doc    "Re-asserts the soundness boundary after adding the Forall term form and GEN/SPEC: first-order
           quantification opens no forge path. An importer cannot fabricate a false universally-quantified
           theorem — building Thm.Seq directly (a bogus ⊢ ∀x0.(x0 = (Var 1)), which does NOT hold) outside
           the kernel is CDZ0214. Quantified propositions are terms an importer may build; asserting one as
           a THEOREM remains the exclusive province of the kernel's rules (GEN).")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term) (Forall Int64 Term))
      (type Thm (Seq (List Term) Term))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (export (. Term *))
      (export Thm)
      (export assume)))
  (input  (do
            (import "hol" (Term Thm assume))
            (def (main) (Thm.Seq (list) (Term.Forall 0 (Term.Eq (Term.Var 0) (Term.Var 1)))))
            (export main)))
  (error  CDZ0214))

; ============================================================================================
; Increment 9 — COMPOSED PROOFS: the kernel used as a real proof engine, chaining rules from DIFFERENT
; families into one derivation. Individual rules were pinned in Inc-2..8; these cases show they COMPOSE —
; each step consumes theorems only obtainable from prior rules and mints only through the private Thm
; constructor, exactly as a real HOL proof script does. Two witnesses: a proof spanning the equational (TRANS),
; λ (BETA), and logical (DISCH) families in one chain; and a purely-logical composition over a quantified
; proposition (DISCH then MP).
; ============================================================================================

(case "a composed proof chains the equational, λ, and logical rule families into one derivation"
  (doc    "The kernel as a proof engine across families. STEP 1 (λ): BETA on (λx0.x0) applied to (Var 7)
           gives ⊢ ((λx0.x0) 7) = 7. STEP 2 (equational): TRANS with refl(7) chains it (7=7 composes
           trivially) → ⊢ ((λx0.x0) 7) = 7. STEP 3 (logical): with P that equation, DISCH P (ASSUME P)
           derives ⊢ P ⇒ P — an implication whose antecedent/consequent is a β-reduction theorem. The case
           verifies the final theorem is (Imp P P) with empty hypotheses AND that P is exactly the
           id-application equation. Pins that rules from three families thread into a single derivation,
           each minting only through the kernel — the shape every real proof takes.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term) (Imp Term Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Abs v x)  (match b ((Term.Abs w q) (and (= v w) (term-eq x q))) (_ false)))
          ((Term.Imp x y)  (match b ((Term.Imp p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (remove (: t Term) (: hs (List Term)))
        (match hs ((list) (list)) ((list h .. rest) (if (term-eq h t) (remove t rest) (List.push (remove t rest) h)))))
      (def (subst (: v Int64) (: s Term) (: t Term))
        (match t
          ((Term.Var n) (if (= n v) s (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
          ((Term.Eq a b) (Term.Eq (subst v s a) (subst v s b)))
          ((Term.Abs w body) (if (= w v) (Term.Abs w body) (Term.Abs w (subst v s body))))
          ((Term.Imp a b) (Term.Imp (subst v s a) (subst v s b)))))
      (def (refl (: t Term)) (Thm.Seq (list) (Term.Eq t t)))
      (def (beta (: v Int64) (: body Term) (: arg Term))
        (Thm.Seq (list) (Term.Eq (Term.Comb (Term.Abs v body) arg) (subst v arg body))))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (disch (: p Term) (: th Thm))
        (match th ((Thm.Seq g q) (Thm.Seq (remove p g) (Term.Imp p q)))))
      (def (trans (: t1 Thm) (: t2 Thm))
        (match (concl t1)
          ((Term.Eq a b) (match (concl t2) ((Term.Eq b2 c) (if (term-eq b b2) (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (Term.Eq a c))) (Option.None))) (_ (Option.None))))
          (_ (Option.None))))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export refl)
      (export beta)
      (export trans)
      (export assume)
      (export disch)
      (export concl)
      (export hyps)))
  (input  (do
            (import "hol" (Term Thm term-eq refl beta trans assume disch concl hyps))
            (def (main)
              (let ((idfn-app (Term.Comb (Term.Abs 0 (Term.Var 0)) (Term.Var 7))))
                (let ((th-beta (beta 0 (Term.Var 0) (Term.Var 7))))
                  (match (trans th-beta (refl (Term.Var 7)))
                    ((Option.Some th-chain)
                      (let ((p (concl th-chain)))
                        (let ((impthm (disch p (assume p))))
                          (and (term-eq (concl impthm) (Term.Imp p p))
                               (and (match (hyps impthm) ((list) true) (_ false))
                                    (match p ((Term.Eq l r) (and (term-eq l idfn-app) (term-eq r (Term.Var 7)))) (_ false)))))))
                    ((Option.None) false)))))
            (export main)))
  (output (: true Bool)))

(case "a purely-logical composed proof over a quantified proposition: DISCH then MP"
  (doc    "Composition within the logical layer, over a QUANTIFIED proposition P = (∀x0. x0 = x0). DISCH P
           (ASSUME P) derives ⊢ P ⇒ P (empty hyps); then MP applied to that implication and {P} ⊢ P
           (ASSUME P) derives a theorem whose conclusion is P. Pins that implication introduction and
           elimination compose, and that the Imp and Forall term forms coexist in a single proof.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term) (Imp Term Term) (Forall Int64 Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Abs v x)  (match b ((Term.Abs w q) (and (= v w) (term-eq x q))) (_ false)))
          ((Term.Imp x y)  (match b ((Term.Imp p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Forall v x) (match b ((Term.Forall w q) (and (= v w) (term-eq x q))) (_ false)))))
      (def (remove (: t Term) (: hs (List Term)))
        (match hs ((list) (list)) ((list h .. rest) (if (term-eq h t) (remove t rest) (List.push (remove t rest) h)))))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (disch (: p Term) (: th Thm))
        (match th ((Thm.Seq g q) (Thm.Seq (remove p g) (Term.Imp p q)))))
      (def (mp (: imp Thm) (: th Thm))
        (match (concl imp)
          ((Term.Imp p q) (if (term-eq (concl th) p) (Option.Some (Thm.Seq (List.concat (hyps imp) (hyps th)) q)) (Option.None)))
          (_ (Option.None))))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export assume)
      (export disch)
      (export mp)
      (export concl)
      (export hyps)))
  (input  (do
            (import "hol" (Term Thm term-eq assume disch mp concl hyps))
            (def (main)
              (let ((p (Term.Forall 0 (Term.Eq (Term.Var 0) (Term.Var 0)))))
                (let ((imp (disch p (assume p))))
                  (and (term-eq (concl imp) (Term.Imp p p))
                       (match (mp imp (assume p))
                         ((Option.Some r) (term-eq (concl r) p))
                         ((Option.None) false))))))
            (export main)))
  (output (: true Bool)))
; --- The trust boundary is module COOPERATION: the deliberate-leak and transport faces --------------
; The unforgeability pins above establish that OUTSIDE code cannot forge or destructure a Thm
; without the kernel's cooperation. These pin the boundary's exact shape from the other side,
; promoted from passing breaker probes.

(case "a kernel may deliberately export its rule as a first-class value"
  (doc    "`(def (mk-forger) Thm.Proved)` exported — the kernel RETURNS its private ctor as a
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
  (input  (do
            (import "kernel" (Thm axiom thm-val mk-forger))
            (def (main (: d Int64))
              (thm-val ((mk-forger) 99)))
            (export main)))
  (call   main (: 0 Int64))
  (output (: 99 Int64)))

(case "a Thm rides a collection through outside code without destructure rights"
  (doc    "`(List.at (List.push (list) (axiom)) 0)` — outside code CARRIES a legitimately-obtained
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
  (input  (do
            (import "kernel" (Thm axiom thm-val))
            (def (main (: d Int64))
              (thm-val (Option.expect (List.at (List.push (list) (axiom)) 0) "t")))
            (export main)))
  (call   main (: 0 Int64))
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

(case "breaker holsubst: a shadowing binder blocks substitution"
  (doc    "Promoted breaker probe — see the section comment.")
  (input (do
           (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
           (def (subst (: v Int64) (: s Term) (: t Term))
             (match t
               ((Term.Var n)   (if (= n v) s (Term.Var n)))
               ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
               ((Term.Eq a b)  (Term.Eq (subst v s a) (subst v s b)))
               ((Term.Abs w body) (if (= w v) (Term.Abs w body) (Term.Abs w (subst v s body))))))
           (def (teq (: a Term) (: b Term))
             (match a
               ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
               ((Term.Comb x y) (match b ((Term.Comb p q) (and (teq x p) (teq y q))) (_ false)))
               ((Term.Eq x y) (match b ((Term.Eq p q) (and (teq x p) (teq y q))) (_ false)))
               ((Term.Abs w x) (match b ((Term.Abs u y) (and (= w u) (teq x y))) (_ false)))))
           (def (main (: d Int64))
             (if (teq (subst 1 (Term.Var 9) (Term.Abs 1 (Term.Var 1)))
                      (Term.Abs 1 (Term.Var 1))) 1 0))
           (export main)))
  (call main (: 0 Int64)) (output (: 1 Int64)))

(case "breaker holsubst: a free occurrence beside a shadow substitutes selectively"
  (doc    "Promoted breaker probe — see the section comment.")
  (input (do
           (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
           (def (subst (: v Int64) (: s Term) (: t Term))
             (match t
               ((Term.Var n)   (if (= n v) s (Term.Var n)))
               ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
               ((Term.Eq a b)  (Term.Eq (subst v s a) (subst v s b)))
               ((Term.Abs w body) (if (= w v) (Term.Abs w body) (Term.Abs w (subst v s body))))))
           (def (teq (: a Term) (: b Term))
             (match a
               ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
               ((Term.Comb x y) (match b ((Term.Comb p q) (and (teq x p) (teq y q))) (_ false)))
               ((Term.Eq x y) (match b ((Term.Eq p q) (and (teq x p) (teq y q))) (_ false)))
               ((Term.Abs w x) (match b ((Term.Abs u y) (and (= w u) (teq x y))) (_ false)))))
           (def (main (: d Int64))
             (if (teq (subst 1 (Term.Var 9) (Term.Comb (Term.Var 1) (Term.Abs 1 (Term.Var 1))))
                      (Term.Comb (Term.Var 9) (Term.Abs 1 (Term.Var 1)))) 1 0))
           (export main)))
  (call main (: 0 Int64)) (output (: 1 Int64)))

(case "breaker holsubst: the naive subst's documented capture hazard"
  (doc    "Promoted breaker probe — see the section comment.")
  (input (do
           (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
           (def (subst (: v Int64) (: s Term) (: t Term))
             (match t
               ((Term.Var n)   (if (= n v) s (Term.Var n)))
               ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
               ((Term.Eq a b)  (Term.Eq (subst v s a) (subst v s b)))
               ((Term.Abs w body) (if (= w v) (Term.Abs w body) (Term.Abs w (subst v s body))))))
           (def (teq (: a Term) (: b Term))
             (match a
               ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
               ((Term.Comb x y) (match b ((Term.Comb p q) (and (teq x p) (teq y q))) (_ false)))
               ((Term.Eq x y) (match b ((Term.Eq p q) (and (teq x p) (teq y q))) (_ false)))
               ((Term.Abs w x) (match b ((Term.Abs u y) (and (= w u) (teq x y))) (_ false)))))
           (def (main (: d Int64))
             (if (teq (subst 1 (Term.Var 2) (Term.Abs 2 (Term.Var 1)))
                      (Term.Abs 2 (Term.Var 2))) 1 0))
           (export main)))
  (call main (: 0 Int64)) (output (: 1 Int64)))


; --- Capture-avoiding subst: the α-rename's structural edges ----------------------------------------
; Inc 5's pins verify the substituted free var SURVIVES (free-in true). These pin the α-rename's
; STRUCTURE — the exact renamed term, promoted from passing breaker probes: the fresh id must clear
; BOTH s's and the body's ids (not just s's), and a non-capturing subst must take the plain path
; (no spurious rename).

(case "breaker capsubst: the fresh binder clears the body's ids, not only s's"
  (doc    "Promoted breaker probe — see the section comment.")
  (input (do
           (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
           (def (free-in (: v Int64) (: t Term))
             (match t
               ((Term.Var n) (= n v))
               ((Term.Comb f x) (or (free-in v f) (free-in v x)))
               ((Term.Eq a b) (or (free-in v a) (free-in v b)))
               ((Term.Abs w body) (if (= w v) false (free-in v body)))))
           (def (max-id (: t Term))
             (match t
               ((Term.Var n) n)
               ((Term.Comb f x) (let ((a (max-id f)) (b (max-id x))) (if (> a b) a b)))
               ((Term.Eq a b) (let ((p (max-id a)) (q (max-id b))) (if (> p q) p q)))
               ((Term.Abs w body) (let ((m (max-id body))) (if (> w m) w m)))))
           (def (rename (: from Int64) (: to Int64) (: t Term))
             (match t
               ((Term.Var n) (if (= n from) (Term.Var to) (Term.Var n)))
               ((Term.Comb f x) (Term.Comb (rename from to f) (rename from to x)))
               ((Term.Eq a b) (Term.Eq (rename from to a) (rename from to b)))
               ((Term.Abs w body) (if (= w from) (Term.Abs w body) (Term.Abs w (rename from to body))))))
           (def (subst (: v Int64) (: s Term) (: t Term))
             (match t
               ((Term.Var n) (if (= n v) s (Term.Var n)))
               ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
               ((Term.Eq a b) (Term.Eq (subst v s a) (subst v s b)))
               ((Term.Abs w body)
                 (if (= w v) (Term.Abs w body)
                     (if (free-in w s)
                         (let ((fresh (+ 1 (let ((ms (max-id s)) (mt (max-id body))) (if (> ms mt) ms mt)))))
                           (Term.Abs fresh (subst v s (rename w fresh body))))
                         (Term.Abs w (subst v s body)))))))
           (def (teq (: a Term) (: b Term))
             (match a
               ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
               ((Term.Comb x y) (match b ((Term.Comb p q) (and (teq x p) (teq y q))) (_ false)))
               ((Term.Eq x y) (match b ((Term.Eq p q) (and (teq x p) (teq y q))) (_ false)))
               ((Term.Abs w x) (match b ((Term.Abs u y) (and (= w u) (teq x y))) (_ false)))))
           (def (main (: d Int64))
             (if (teq (subst 0 (Term.Var 1) (Term.Abs 1 (Term.Comb (Term.Var 0) (Term.Var 7))))
                      (Term.Abs 8 (Term.Comb (Term.Var 1) (Term.Var 7)))) 1 0))
           (export main)))
  (call main (: 0 Int64)) (output (: 1 Int64)))

(case "breaker capsubst: a non-capturing substitution takes the plain path"
  (doc    "Promoted breaker probe — see the section comment.")
  (input (do
           (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
           (def (free-in (: v Int64) (: t Term))
             (match t
               ((Term.Var n) (= n v))
               ((Term.Comb f x) (or (free-in v f) (free-in v x)))
               ((Term.Eq a b) (or (free-in v a) (free-in v b)))
               ((Term.Abs w body) (if (= w v) false (free-in v body)))))
           (def (max-id (: t Term))
             (match t
               ((Term.Var n) n)
               ((Term.Comb f x) (let ((a (max-id f)) (b (max-id x))) (if (> a b) a b)))
               ((Term.Eq a b) (let ((p (max-id a)) (q (max-id b))) (if (> p q) p q)))
               ((Term.Abs w body) (let ((m (max-id body))) (if (> w m) w m)))))
           (def (rename (: from Int64) (: to Int64) (: t Term))
             (match t
               ((Term.Var n) (if (= n from) (Term.Var to) (Term.Var n)))
               ((Term.Comb f x) (Term.Comb (rename from to f) (rename from to x)))
               ((Term.Eq a b) (Term.Eq (rename from to a) (rename from to b)))
               ((Term.Abs w body) (if (= w from) (Term.Abs w body) (Term.Abs w (rename from to body))))))
           (def (subst (: v Int64) (: s Term) (: t Term))
             (match t
               ((Term.Var n) (if (= n v) s (Term.Var n)))
               ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
               ((Term.Eq a b) (Term.Eq (subst v s a) (subst v s b)))
               ((Term.Abs w body)
                 (if (= w v) (Term.Abs w body)
                     (if (free-in w s)
                         (let ((fresh (+ 1 (let ((ms (max-id s)) (mt (max-id body))) (if (> ms mt) ms mt)))))
                           (Term.Abs fresh (subst v s (rename w fresh body))))
                         (Term.Abs w (subst v s body)))))))
           (def (teq (: a Term) (: b Term))
             (match a
               ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
               ((Term.Comb x y) (match b ((Term.Comb p q) (and (teq x p) (teq y q))) (_ false)))
               ((Term.Eq x y) (match b ((Term.Eq p q) (and (teq x p) (teq y q))) (_ false)))
               ((Term.Abs w x) (match b ((Term.Abs u y) (and (= w u) (teq x y))) (_ false)))))
           (def (main (: d Int64))
             (if (teq (subst 0 (Term.Var 5) (Term.Abs 1 (Term.Var 0)))
                      (Term.Abs 1 (Term.Var 5))) 1 0))
           (export main)))
  (call main (: 0 Int64)) (output (: 1 Int64)))


; --- ∀-elimination with a real substitution (the non-identity SPEC face) ---------------------------
(case "breaker specsubst: GEN of refl then SPEC substitutes the bound var throughout the body"
  (doc    "The FIRST-THEOREM ∀ round-trip with a REAL substitution (Inc-8's GEN pin exercises only\n           identity-SPEC, where the bound var is absent from the body): `refl (Var 1)` is\n           `{} |- (Var1 = Var1)` with NO hypotheses, so GEN 1 is sound and the body MENTIONS 1 →\n           `{} |- ∀1.(Var1 = Var1)`; SPEC (Var 7) substitutes throughout → `{} |- (Var7 = Var7)`. Pins\n           that ∀-elimination genuinely instantiates a quantified body (not just a no-op), the\n           complement of the side-condition pin — a SPEC that dropped the subst would leave (Var1 = Var1).")
  (input (do
           (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term) (Forall Int64 Term))
           (type Thm (Seq (List Term) Term))
           (def (term-eq (: a Term) (: b Term))
             (match a
               ((Term.Var n) (match b ((Term.Var m) (= n m)) (_ false)))
               ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
               ((Term.Eq x y) (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
               ((Term.Abs v x) (match b ((Term.Abs w q) (and (= v w) (term-eq x q))) (_ false)))
               ((Term.Forall v x) (match b ((Term.Forall w q) (and (= v w) (term-eq x q))) (_ false)))))
           (def (free-in (: v Int64) (: t Term))
             (match t
               ((Term.Var n) (= n v))
               ((Term.Comb f x) (or (free-in v f) (free-in v x)))
               ((Term.Eq a b) (or (free-in v a) (free-in v b)))
               ((Term.Abs w body) (if (= w v) false (free-in v body)))
               ((Term.Forall w body) (if (= w v) false (free-in v body)))))
           (def (subst (: v Int64) (: s Term) (: t Term))
             (match t
               ((Term.Var n) (if (= n v) s (Term.Var n)))
               ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
               ((Term.Eq a b) (Term.Eq (subst v s a) (subst v s b)))
               ((Term.Abs w body) (if (= w v) (Term.Abs w body) (Term.Abs w (subst v s body))))
               ((Term.Forall w body) (if (= w v) (Term.Forall w body) (Term.Forall w (subst v s body))))))
           (def (free-in-hyps (: v Int64) (: hs (List Term)))
             (match hs ((list) false) ((list h .. rest) (or (free-in v h) (free-in-hyps v rest)))))
           (def (refl (: t Term)) (Thm.Seq (list) (Term.Eq t t)))
           (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
           (def (gen (: x Int64) (: th Thm))
             (match th ((Thm.Seq g p)
               (if (free-in-hyps x g) (Option.None unit) (Option.Some (Thm.Seq g (Term.Forall x p)))))))
           (def (spec (: t Term) (: th Thm))
             (match (concl th)
               ((Term.Forall x body) (Option.Some (Thm.Seq (match th ((Thm.Seq h _) h)) (subst x t body))))
               (_ (Option.None unit))))
           (def (main (: d Int64))
             (match (gen 1 (refl (Term.Var 1)))
               ((Option.Some g) (match (spec (Term.Var 7) g)
                                  ((Option.Some sp) (if (term-eq (concl sp) (Term.Eq (Term.Var 7) (Term.Var 7))) 1 0))
                                  ((Option.None _) -1)))
               ((Option.None _) -2)))
           (export main)))
  (call main (: 0 Int64)) (output (: 1 Int64)))

; ============================================================================================
; Increment 11 — SOUNDNESS FIX (breaker-found): the CHAINING rules must UNION their operands'
; hypotheses. TRANS and MK_COMB were outputting (Thm.Seq (list) …) — an EMPTY hypothesis set —
; regardless of their operands' hypotheses, so an assumption threaded through them was SILENTLY
; DISCHARGED (a later DISCH/MP could then "prove" the conclusion with NO premise — an unsound theorem).
; The fix (this file's TRANS/MK_COMB now do it): a rule that consumes N theorems and produces one must
; carry the UNION of their hypotheses (HOL: G1 ⊢ a=b and G2 ⊢ b=c yields G1∪G2 ⊢ a=c), exactly as MP /
; EQ_MP already did via List.concat. Leaf/axiom-schema rules (refl, beta, assume) correctly use
; (list)/(list p) — they have no operand theorems. These cases thread a real ASSUME through the chaining
; rules and assert the hypothesis SURVIVES (a hyp-dropping rule flips them to false). This is the exact
; soundness class the whole vertical exists to protect. (This file's earlier TRANS/MK_COMB uses only ever
; chained hypothesis-free refl/beta results, so the bug was LATENT — no graded case exposed it until now.)
; ============================================================================================

(case "TRANS unions its operands' hypotheses — an assumption threaded through TRANS survives (soundness)"
  (doc    "The breaker-found soundness pin for TRANS. th1 = ASSUME (a=b) : {a=b} ⊢ a=b; th2 = refl(b) :
           ⊢ b=b. TRANS chains them (b matches) → the result must be {a=b} ⊢ a=b — the hypothesis {a=b}
           SURVIVES. A TRANS that output (Thm.Seq (list) …) would silently discharge {a=b}, letting a
           later step 'prove' a=b with no premise (unsound). The case asserts the result's conclusion is
           a=b AND its single hypothesis is a=b. Pins that TRANS unions operand hypotheses (like MP does).")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (refl (: t Term)) (Thm.Seq (list) (Term.Eq t t)))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (trans (: th1 Thm) (: th2 Thm))
        (match (concl th1)
          ((Term.Eq a b) (match (concl th2) ((Term.Eq b2 c) (if (term-eq b b2) (Option.Some (Thm.Seq (List.concat (hyps th1) (hyps th2)) (Term.Eq a c))) (Option.None))) (_ (Option.None))))
          (_ (Option.None))))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export refl)
      (export assume)
      (export trans)
      (export concl)
      (export hyps)))
  (input  (do
            (import "hol" (Term Thm term-eq refl assume trans concl hyps))
            (def (main)
              (let ((a (Term.Var 1)) (b (Term.Var 2)))
                (let ((eqab (Term.Eq a b)))
                  (match (trans (assume eqab) (refl b))
                    ((Option.Some r)
                      (and (term-eq (concl r) eqab)
                           (match (hyps r) ((list h) (term-eq h eqab)) (_ false))))
                    ((Option.None) false)))))
            (export main)))
  (output (: true Bool)))

(case "MK_COMB unions its operands' hypotheses — an assumption threaded through MK_COMB survives (soundness)"
  (doc    "The soundness pin for MK_COMB (congruence). th1 = ASSUME (f=g) : {f=g} ⊢ f=g; th2 = refl(x) :
           ⊢ x=x. MK_COMB derives ⊢ (f x)=(g x) — and the hypothesis {f=g} must SURVIVE in the result, or
           the congruence step would silently discharge the assumption it depended on. The case asserts the
           conclusion is (f x)=(g x) AND the single hypothesis is f=g. Pins that MK_COMB unions operand
           hypotheses. (Companion to the TRANS pin — the breaker's audit flagged both chaining rules.)")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (refl (: t Term)) (Thm.Seq (list) (Term.Eq t t)))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (mk-comb (: th1 Thm) (: th2 Thm))
        (match (concl th1)
          ((Term.Eq f g)
            (match (concl th2)
              ((Term.Eq x y) (Option.Some (Thm.Seq (List.concat (hyps th1) (hyps th2)) (Term.Eq (Term.Comb f x) (Term.Comb g y)))))
              (_ (Option.None))))
          (_ (Option.None))))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export refl)
      (export assume)
      (export mk-comb)
      (export concl)
      (export hyps)))
  (input  (do
            (import "hol" (Term Thm term-eq refl assume mk-comb concl hyps))
            (def (main)
              (let ((f (Term.Var 1)) (g (Term.Var 2)) (x (Term.Var 3)))
                (let ((eqfg (Term.Eq f g)))
                  (match (mk-comb (assume eqfg) (refl x))
                    ((Option.Some r)
                      (and (term-eq (concl r) (Term.Eq (Term.Comb f x) (Term.Comb g x)))
                           (match (hyps r) ((list h) (term-eq h eqfg)) (_ false))))
                    ((Option.None) false)))))
            (export main)))
  (output (: true Bool)))

; ============================================================================================
; Increment 10 — the premise-matching rules should match up to α-EQUIVALENCE, not structural equality.
; A rule like MP (match the antecedent) or TRANS (match the middle term) that compares premises with
; structural `term-eq` is SOUND-BUT-INCOMPLETE: it REJECTS a premise that is an α-variant of what it
; expects (e.g. (λx0.x0) vs (λx5.x5)) — never derives a false theorem, but misses valid inferences a real
; HOL kernel accepts. Matching with `aconv` (Inc-6) is complete AND sound: a real HOL kernel matches
; premises up to α. These cases pin BOTH faces — the structural version DECLINES the α-variant (the
; incompleteness), the aconv version ACCEPTS it and derives the conclusion — so the completeness property
; is gate-protected. (Not a soundness fix like Inc-11; a completeness + HOL-fidelity improvement.)
; ============================================================================================

(case "MP should match its antecedent up to α-equivalence: aconv-MP accepts an α-variant a structural equality rejects"
  (doc    "Premise-matching completeness for MP. `imp` : ⊢ (λx0.x0) ⇒ (Var 100); `th` : ⊢ (λx5.x5) — the
           antecedent (λx0.x0) and th's conclusion (λx5.x5) are α-EQUIVALENT (same function, renamed bound
           var). A structural-`term-eq` MP DECLINES (0 ≠ 5) — sound but incomplete, blocking a valid step.
           An `aconv`-matching MP ACCEPTS it and derives ⊢ (Var 100). The case pins both: mp-struct → None,
           mp-aconv → Some with conclusion (Var 100). Pins that the correct premise-matching equality for a
           HOL kernel is α-equivalence, not structural — the completeness a real kernel needs.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term) (Imp Term Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Abs v x)  (match b ((Term.Abs w q) (and (= v w) (term-eq x q))) (_ false)))
          ((Term.Imp x y)  (match b ((Term.Imp p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (depth-of (: v Int64) (: stack (List Int64)))
        (match stack ((list) (- 0 1))
          ((list top .. rest) (if (= top v) 0 (let ((d (depth-of v rest))) (if (< d 0) d (+ d 1)))))))
      (def (aconv-env (: sa (List Int64)) (: sb (List Int64)) (: a Term) (: b Term))
        (match a
          ((Term.Var n) (match b ((Term.Var m) (let ((da (depth-of n sa)) (db (depth-of m sb))) (if (< da 0) (and (< db 0) (= n m)) (= da db)))) (_ false)))
          ((Term.Comb f x) (match b ((Term.Comb g y) (and (aconv-env sa sb f g) (aconv-env sa sb x y))) (_ false)))
          ((Term.Eq p q) (match b ((Term.Eq r s) (and (aconv-env sa sb p r) (aconv-env sa sb q s))) (_ false)))
          ((Term.Abs v body) (match b ((Term.Abs w b2) (aconv-env (List.push sa v) (List.push sb w) body b2)) (_ false)))
          ((Term.Imp p q) (match b ((Term.Imp r s) (and (aconv-env sa sb p r) (aconv-env sa sb q s))) (_ false)))))
      (def (aconv (: a Term) (: b Term)) (aconv-env (list) (list) a b))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (mp-struct (: imp Thm) (: th Thm))
        (match (concl imp) ((Term.Imp p q) (if (term-eq (concl th) p) (Option.Some (Thm.Seq (list) q)) (Option.None))) (_ (Option.None))))
      (def (mp-aconv (: imp Thm) (: th Thm))
        (match (concl imp) ((Term.Imp p q) (if (aconv (concl th) p) (Option.Some (Thm.Seq (list) q)) (Option.None))) (_ (Option.None))))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export aconv)
      (export assume)
      (export mp-struct)
      (export mp-aconv)
      (export concl)))
  (input  (do
            (import "hol" (Term Thm term-eq aconv assume mp-struct mp-aconv concl))
            (def (main)
              (let ((ante (Term.Abs 0 (Term.Var 0)))
                    (ante-variant (Term.Abs 5 (Term.Var 5)))
                    (q (Term.Var 100)))
                (let ((imp (assume (Term.Imp ante q)))
                      (th (assume ante-variant)))
                  (and
                    (match (mp-struct imp th) ((Option.None) true) ((Option.Some _) false))
                    (match (mp-aconv imp th) ((Option.Some r) (term-eq (concl r) q)) ((Option.None) false))))))
            (export main)))
  (output (: true Bool)))

(case "TRANS should match its middle term up to α-equivalence: aconv-TRANS chains an α-variant a structural equality rejects"
  (doc    "Premise-matching completeness for TRANS. t1 : ⊢ (Var 1) = (λx0.x0); t2 : ⊢ (λx5.x5) = (Var 2) —
           the middle terms (λx0.x0) and (λx5.x5) are α-EQUIVALENT. A structural-`term-eq` TRANS DECLINES
           (the middles differ syntactically) — sound but incomplete. An `aconv`-matching TRANS ACCEPTS and
           derives ⊢ (Var 1) = (Var 2). The case pins both: trans-struct → None, trans-aconv → Some with
           conclusion (Var 1) = (Var 2). Pins that TRANS (like MP) should match its connecting term up to
           α-equivalence for a complete + HOL-faithful kernel.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Abs v x)  (match b ((Term.Abs w q) (and (= v w) (term-eq x q))) (_ false)))))
      (def (depth-of (: v Int64) (: stack (List Int64)))
        (match stack ((list) (- 0 1))
          ((list top .. rest) (if (= top v) 0 (let ((d (depth-of v rest))) (if (< d 0) d (+ d 1)))))))
      (def (aconv-env (: sa (List Int64)) (: sb (List Int64)) (: a Term) (: b Term))
        (match a
          ((Term.Var n) (match b ((Term.Var m) (let ((da (depth-of n sa)) (db (depth-of m sb))) (if (< da 0) (and (< db 0) (= n m)) (= da db)))) (_ false)))
          ((Term.Comb f x) (match b ((Term.Comb g y) (and (aconv-env sa sb f g) (aconv-env sa sb x y))) (_ false)))
          ((Term.Eq p q) (match b ((Term.Eq r s) (and (aconv-env sa sb p r) (aconv-env sa sb q s))) (_ false)))
          ((Term.Abs v body) (match b ((Term.Abs w b2) (aconv-env (List.push sa v) (List.push sb w) body b2)) (_ false)))))
      (def (aconv (: a Term) (: b Term)) (aconv-env (list) (list) a b))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (trans-struct (: t1 Thm) (: t2 Thm))
        (match (concl t1) ((Term.Eq a b) (match (concl t2) ((Term.Eq b2 c) (if (term-eq b b2) (Option.Some (Thm.Seq (list) (Term.Eq a c))) (Option.None))) (_ (Option.None)))) (_ (Option.None))))
      (def (trans-aconv (: t1 Thm) (: t2 Thm))
        (match (concl t1) ((Term.Eq a b) (match (concl t2) ((Term.Eq b2 c) (if (aconv b b2) (Option.Some (Thm.Seq (list) (Term.Eq a c))) (Option.None))) (_ (Option.None)))) (_ (Option.None))))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export aconv)
      (export assume)
      (export trans-struct)
      (export trans-aconv)
      (export concl)))
  (input  (do
            (import "hol" (Term Thm term-eq aconv assume trans-struct trans-aconv concl))
            (def (main)
              (let ((a (Term.Var 1))
                    (mid1 (Term.Abs 0 (Term.Var 0)))
                    (mid2 (Term.Abs 5 (Term.Var 5)))
                    (c (Term.Var 2)))
                (let ((t1 (assume (Term.Eq a mid1)))
                      (t2 (assume (Term.Eq mid2 c))))
                  (and
                    (match (trans-struct t1 t2) ((Option.None) true) ((Option.Some _) false))
                    (match (trans-aconv t1 t2) ((Option.Some r) (term-eq (concl r) (Term.Eq a c))) ((Option.None) false))))))
            (export main)))
  (output (: true Bool)))
(case "TRANS unions the hypotheses of BOTH operands when each carries a distinct assumption"
  (doc "The soundness fix (TRANS/MK_COMB union operand hypotheses) is pinned for the one-operand case; this pins the actual union — both operands carry a distinct assumption and the result must retain BOTH. A TRANS that kept only one operand's hypotheses (or emptied them) would silently discharge a live assumption, letting an unproven equation escape. trans({a=b}|-a=b, {b=c}|-b=c) = {a=b, b=c}|-a=c, so hyps has length 2.")
  (input (do
           (type Term (Var Int64) (Comb Term Term) (Eq Term Term))
           (type Thm (Seq (List Term) Term))
           (def (term-eq (: p Term) (: q Term))
             (match p
               ((Term.Var n) (match q ((Term.Var m) (= n m)) (_ false)))
               ((Term.Comb a b) (match q ((Term.Comb c d) (and (term-eq a c) (term-eq b d))) (_ false)))
               ((Term.Eq a b) (match q ((Term.Eq c d) (and (term-eq a c) (term-eq b d))) (_ false)))))
           (def (assume (: t Term)) (Thm.Seq (List.push (list) t) t))
           (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
           (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
           (def (trans (: t1 Thm) (: t2 Thm))
             (match (concl t1)
               ((Term.Eq a b) (match (concl t2)
                                ((Term.Eq b2 c) (if (term-eq b b2) (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (Term.Eq a c))) (Option.None unit)))
                                (_ (Option.None unit))))
               (_ (Option.None unit))))
           (def (main (: d Int64))
             (match (trans (assume (Term.Eq (Term.Var 1) (Term.Var 2)))
                           (assume (Term.Eq (Term.Var 2) (Term.Var 3))))
               ((Option.Some r) (List.len (hyps r)))
               ((Option.None _) -1)))
           (export main)))
  (call main (: 0 Int64)) (output (: 2 Int64)))

(case "TRANS carries a hypothesis borne by the RIGHT operand only (mirror of the left-only pin)"
  (doc "The landed pin threads an assumption through TRANS's LEFT operand (refl on the right). This mirrors it: the hypothesis lives on the RIGHT operand and refl (no hypotheses) is on the left. trans(refl a, {a=c}|-a=c) = {a=c}|-a=c, so hyps has length 1. A TRANS reading only the left operand's hypotheses would drop the assumption entirely.")
  (input (do
           (type Term (Var Int64) (Comb Term Term) (Eq Term Term))
           (type Thm (Seq (List Term) Term))
           (def (term-eq (: p Term) (: q Term))
             (match p
               ((Term.Var n) (match q ((Term.Var m) (= n m)) (_ false)))
               ((Term.Comb a b) (match q ((Term.Comb c d) (and (term-eq a c) (term-eq b d))) (_ false)))
               ((Term.Eq a b) (match q ((Term.Eq c d) (and (term-eq a c) (term-eq b d))) (_ false)))))
           (def (assume (: t Term)) (Thm.Seq (List.push (list) t) t))
           (def (refl (: t Term)) (Thm.Seq (list) (Term.Eq t t)))
           (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
           (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
           (def (trans (: t1 Thm) (: t2 Thm))
             (match (concl t1)
               ((Term.Eq a b) (match (concl t2)
                                ((Term.Eq b2 c) (if (term-eq b b2) (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (Term.Eq a c))) (Option.None unit)))
                                (_ (Option.None unit))))
               (_ (Option.None unit))))
           (def (main (: d Int64))
             (match (trans (refl (Term.Var 1))
                           (assume (Term.Eq (Term.Var 1) (Term.Var 3))))
               ((Option.Some r) (List.len (hyps r)))
               ((Option.None _) -1)))
           (export main)))
  (call main (: 0 Int64)) (output (: 1 Int64)))

(case "MK_COMB unions the hypotheses of BOTH operands when each carries a distinct assumption"
  (doc "Companion to the TRANS two-operand union: MK_COMB combines {f=g} and {x=y} into (Comb f x)=(Comb g y), and the result must retain BOTH assumptions. mk-comb({f=g}|-f=g, {x=y}|-x=y) has hyps of length 2. An MK_COMB emitting an empty hypothesis set (the pre-fix bug) would let a congruence step launder away two live assumptions.")
  (input (do
           (type Term (Var Int64) (Comb Term Term) (Eq Term Term))
           (type Thm (Seq (List Term) Term))
           (def (assume (: t Term)) (Thm.Seq (List.push (list) t) t))
           (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
           (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
           (def (mk-comb (: t1 Thm) (: t2 Thm))
             (match (concl t1)
               ((Term.Eq f g) (match (concl t2)
                                ((Term.Eq x y) (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (Term.Eq (Term.Comb f x) (Term.Comb g y)))))
                                (_ (Option.None unit))))
               (_ (Option.None unit))))
           (def (main (: d Int64))
             (match (mk-comb (assume (Term.Eq (Term.Var 1) (Term.Var 2)))
                             (assume (Term.Eq (Term.Var 3) (Term.Var 4))))
               ((Option.Some r) (List.len (hyps r)))
               ((Option.None _) -1)))
           (export main)))
  (call main (: 0 Int64)) (output (: 2 Int64)))

(case "TRANS chained three deep accumulates every operand's hypotheses"
  (doc "Assumptions must accumulate transitively, not just pairwise. Chaining trans(trans({a=b},{b=c}), {c=d}) yields {a=b,b=c,c=d}|-a=d, so hyps has length 3. This guards against a union that resets or caps at the most recent step — the accumulation across nested proof structure is where a subtle drop would hide.")
  (input (do
           (type Term (Var Int64) (Comb Term Term) (Eq Term Term))
           (type Thm (Seq (List Term) Term))
           (def (term-eq (: p Term) (: q Term))
             (match p
               ((Term.Var n) (match q ((Term.Var m) (= n m)) (_ false)))
               ((Term.Comb a b) (match q ((Term.Comb c d) (and (term-eq a c) (term-eq b d))) (_ false)))
               ((Term.Eq a b) (match q ((Term.Eq c d) (and (term-eq a c) (term-eq b d))) (_ false)))))
           (def (assume (: t Term)) (Thm.Seq (List.push (list) t) t))
           (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
           (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
           (def (trans (: t1 Thm) (: t2 Thm))
             (match (concl t1)
               ((Term.Eq a b) (match (concl t2)
                                ((Term.Eq b2 c) (if (term-eq b b2) (Option.Some (Thm.Seq (List.concat (hyps t1) (hyps t2)) (Term.Eq a c))) (Option.None unit)))
                                (_ (Option.None unit))))
               (_ (Option.None unit))))
           (def (main (: d Int64))
             (match (trans (assume (Term.Eq (Term.Var 1) (Term.Var 2))) (assume (Term.Eq (Term.Var 2) (Term.Var 3))))
               ((Option.Some r12) (match (trans r12 (assume (Term.Eq (Term.Var 3) (Term.Var 4))))
                                    ((Option.Some r) (List.len (hyps r)))
                                    ((Option.None _) -1)))
               ((Option.None _) -2)))
           (export main)))
  (call main (: 0 Int64)) (output (: 3 Int64)))

; ============================================================================================
; Increment 12 — the HOL-FAITHFUL LOGICAL LAYER: logical constants as DEFINED constants (via
; new_basic_definition, which returns a defining theorem |- (Const c) = rhs), plus intro/elim rules,
; deriving theorems ABOUT them through the kernel. Where Inc-7/8 modelled ⇒/∀ as PRIMITIVE term forms
; (the natural-deduction shortcut), this increment shows the fusion.ml presentation: a constant is
; introduced by a definitional axiom and its theorems are DERIVED. Slice 1: truth T (defined + |- T
; derived) and conjunction ∧ (CONJ intro unions hypotheses — soundness-correct per Inc-11 — and
; CONJUNCT1/CONJUNCT2 elim preserve them). Later slices: ∨/¬/∃ and the three axioms (ETA/SELECT/INFINITY).
; ============================================================================================

(case "the kernel defines T (truth) as a constant and DERIVES |- T through the rules"
  (doc    "The HOL-faithful way to introduce truth: T is a defined CONSTANT, not a primitive. HOL defines
           T := ((λp.p) = (λp.p)); new_basic_definition returns the defining theorem |- (Const T) = rhs.
           |- T is then DERIVED (not asserted): REFL of (λp.p) gives |- rhs (i.e. |- (λp.p)=(λp.p)); SYM of
           the definition gives |- rhs = T; EQ_MP of those gives |- T. The case checks the derived theorem's
           conclusion is exactly (Const T). Pins that a constant can be defined by a definitional theorem
           and a theorem about it derived purely through the kernel's rules — the fusion.ml presentation
           (vs the primitive-Imp/Forall shortcut of Inc-7/8).")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term) (Const Int64))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Abs v x)  (match b ((Term.Abs w q) (and (= v w) (term-eq x q))) (_ false)))
          ((Term.Const c)  (match b ((Term.Const d) (= c d)) (_ false)))))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (refl (: t Term)) (Thm.Seq (list) (Term.Eq t t)))
      (def (sym (: th Thm))
        (match (concl th) ((Term.Eq a b) (Option.Some (Thm.Seq (hyps th) (Term.Eq b a)))) (_ (Option.None))))
      (def (eq-mp (: eq Thm) (: th Thm))
        (match (concl eq) ((Term.Eq p q) (if (term-eq (concl th) p) (Option.Some (Thm.Seq (List.concat (hyps eq) (hyps th)) q)) (Option.None))) (_ (Option.None))))
      (def (new-basic-def (: c Int64) (: rhs Term)) (Thm.Seq (list) (Term.Eq (Term.Const c) rhs)))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export refl)
      (export sym)
      (export eq-mp)
      (export new-basic-def)
      (export concl)))
  (input  (do
            (import "hol" (Term Thm term-eq refl sym eq-mp new-basic-def concl))
            (def (main)
              (let ((idp (Term.Abs 0 (Term.Var 0))))
                (let ((rhs (Term.Eq idp idp))
                      (tc (Term.Const 0)))
                  (let ((defn (new-basic-def 0 rhs))
                        (rhs-thm (refl idp)))
                    (match (sym defn)
                      ((Option.Some rhs-eq-t)
                        (match (eq-mp rhs-eq-t rhs-thm)
                          ((Option.Some t-thm) (term-eq (concl t-thm) tc))
                          ((Option.None) false)))
                      ((Option.None) false))))))
            (export main)))
  (output (: true Bool)))

(case "the kernel's conjunction: CONJ unions hypotheses, CONJUNCT1/2 project (hyps survive)"
  (doc    "Conjunction as an intro/elim connective. CONJ takes A ⊢ a and B ⊢ b and derives A∪B ⊢ a∧b —
           UNIONING the operand hypotheses (soundness-correct, per the Inc-11 breaker rule that a rule
           consuming multiple theorems must carry the union). CONJUNCT1/CONJUNCT2 take G ⊢ a∧b and derive
           G ⊢ a / G ⊢ b, PRESERVING the hypotheses. From ASSUME(a) : {a}⊢a and ASSUME(b) : {b}⊢b, CONJ →
           {a,b} ⊢ a∧b; CONJUNCT1 → ⊢ a with BOTH a,b still hypotheses. The case asserts the conjunction
           conclusion and that both hypotheses survive the projection. Pins that a binary logical intro
           rule unions hypotheses and its elim rules preserve them.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Conj Term Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Conj x y) (match b ((Term.Conj p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (conj (: th1 Thm) (: th2 Thm))
        (Thm.Seq (List.concat (hyps th1) (hyps th2)) (Term.Conj (concl th1) (concl th2))))
      (def (conjunct1 (: th Thm))
        (match (concl th) ((Term.Conj a b) (Option.Some (Thm.Seq (hyps th) a))) (_ (Option.None))))
      (def (conjunct2 (: th Thm))
        (match (concl th) ((Term.Conj a b) (Option.Some (Thm.Seq (hyps th) b))) (_ (Option.None))))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export assume)
      (export conj)
      (export conjunct1)
      (export conjunct2)
      (export concl)
      (export hyps)))
  (input  (do
            (import "hol" (Term Thm term-eq assume conj conjunct1 conjunct2 concl hyps))
            (def (main)
              (let ((a (Term.Var 1)) (b (Term.Var 2)))
                (let ((c (conj (assume a) (assume b))))
                  (and
                    (term-eq (concl c) (Term.Conj a b))
                    (match (conjunct1 c)
                      ((Option.Some c1)
                        (and (term-eq (concl c1) a)
                             (match (hyps c1) ((list h1 h2) (and (term-eq h1 a) (term-eq h2 b))) (_ false))))
                      ((Option.None) false))))))
            (export main)))
  (output (: true Bool)))

; ============================================================================================
; Increment 12 slice 2 — the remaining logical connectives: disjunction (∨) and the existential (∃).
; ∨ via its introduction rules DISJ1 (G⊢a → G⊢a∨b) and DISJ2 (G⊢b → G⊢a∨b), each preserving hypotheses.
; ∃ via EXISTS-introduction — the dual of SPEC: to prove ∃x.P from a theorem of P[witness/x], the rule
; CHECKS (via the capture-aware subst) that the premise's conclusion really is P[witness/x] before minting
; ∃x.P, carrying the premise's hypotheses. Together with T/∧ (slice 1) and the Inc-7/8 ⇒/∀, this completes
; the propositional + first-order connective set as kernel-minted theorems. (The three HOL axioms —
; ETA/SELECT/INFINITY — are a later slice.)
; ============================================================================================

(case "the kernel's disjunction introduction (DISJ1/DISJ2) preserves hypotheses"
  (doc    "∨-introduction. DISJ1 takes G ⊢ a and an arbitrary other disjunct b, deriving G ⊢ a∨b; DISJ2 is
           the mirror (from G ⊢ b derive G ⊢ a∨b). Both PRESERVE the premise's hypotheses. From ASSUME(a) :
           {a}⊢a, DISJ1 with b yields {a} ⊢ a∨b — the hypothesis {a} survives. The case checks the
           disjunction conclusion and that the hypothesis is preserved. Pins the ∨ intro rules + a Disj
           term form.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Disj Term Term) (Neg Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Disj x y) (match b ((Term.Disj p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Neg x)    (match b ((Term.Neg p) (term-eq x p)) (_ false)))))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (disj1 (: th Thm) (: b Term)) (Thm.Seq (hyps th) (Term.Disj (concl th) b)))
      (def (disj2 (: a Term) (: th Thm)) (Thm.Seq (hyps th) (Term.Disj a (concl th))))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export assume)
      (export disj1)
      (export disj2)
      (export concl)
      (export hyps)))
  (input  (do
            (import "hol" (Term Thm term-eq assume disj1 disj2 concl hyps))
            (def (main)
              (let ((a (Term.Var 1)) (b (Term.Var 2)))
                (let ((d (disj1 (assume a) b)))
                  (and (term-eq (concl d) (Term.Disj a b))
                       (match (hyps d) ((list h) (term-eq h a)) (_ false))))))
            (export main)))
  (output (: true Bool)))

(case "the kernel's existential introduction (EXISTS) checks the witness and preserves hypotheses"
  (doc    "∃-introduction, the dual of SPEC. To prove ∃x.P, EXISTS takes a witness t and a theorem of
           P[t/x]; it CHECKS (via the capture-aware subst) that the premise's conclusion really is P[t/x]
           before minting ∃x.P, carrying the premise's hypotheses. Here body = (x0 = a), witness = a, so
           P[a/x0] = (a = a); from ASSUME(a=a) : {a=a} ⊢ (a=a), EXISTS derives ⊢ ∃x0.(x0=a) with the
           hypothesis preserved. Pins that ∃-intro validates the witness through substitution (a wrong
           witness would fail the term-eq check → Option.None) and threads hypotheses — the existential
           counterpart to SPEC's universal instantiation.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term) (Exists Int64 Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Abs v x)  (match b ((Term.Abs w q) (and (= v w) (term-eq x q))) (_ false)))
          ((Term.Exists v x) (match b ((Term.Exists w q) (and (= v w) (term-eq x q))) (_ false)))))
      (def (subst (: v Int64) (: s Term) (: t Term))
        (match t
          ((Term.Var n) (if (= n v) s (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
          ((Term.Eq a b) (Term.Eq (subst v s a) (subst v s b)))
          ((Term.Abs w body) (if (= w v) (Term.Abs w body) (Term.Abs w (subst v s body))))
          ((Term.Exists w body) (if (= w v) (Term.Exists w body) (Term.Exists w (subst v s body))))))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (exists-intro (: x Int64) (: body Term) (: witness Term) (: th Thm))
        (if (term-eq (concl th) (subst x witness body))
            (Option.Some (Thm.Seq (hyps th) (Term.Exists x body)))
            (Option.None)))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export subst)
      (export assume)
      (export exists-intro)
      (export concl)
      (export hyps)))
  (input  (do
            (import "hol" (Term Thm term-eq subst assume exists-intro concl hyps))
            (def (main)
              (let ((a (Term.Var 5)))
                (let ((body (Term.Eq (Term.Var 0) a))
                      (pinst (Term.Eq a a)))
                  (match (exists-intro 0 body a (assume pinst))
                    ((Option.Some e)
                      (and (term-eq (concl e) (Term.Exists 0 body))
                           (match (hyps e) ((list h) (term-eq h pinst)) (_ false))))
                    ((Option.None) false)))))
            (export main)))
  (output (: true Bool)))

; ============================================================================================
; Increment 13 — pin the single-variant abstract-Thm MATCH-outside case as CDZ0214. A HOL-kernel `Thm` is
; a newtype-style SINGLE-VARIANT sum (e.g. (type Thm (MkThm …))). Matching its withheld constructor from
; outside the kernel module must be the withheld-constructor rejection CDZ0214 — exactly like construction
; and like a multi-variant match. This was previously CDZ0203 (a single-variant newtype-erases to a
; nominal, so its match hit the nominal wrong-ctor check that emitted the bare code without the withheld
; poison); v-patterns FIXED it (the newtype path now propagates the withheld poison first). This pins the
; fixed behavior for the exact shape the kernel uses, so the actionable "this ctor is withheld, use the
; accessor" message can't silently regress to the generic CDZ0203. (Was filed as the queue repro
; adv-single-variant-abstract-match-wrong-diag-cdz0203-not-cdz0214.sexp; now graded here.)
; ============================================================================================

(case "a single-variant abstract theorem's constructor match outside the kernel is a withheld-constructor rejection (CDZ0214)"
  (doc    "The kernel's `Thm` is a single-variant newtype sum; matching its withheld constructor outside the
           module must be CDZ0214 (the withheld-constructor code), NOT the generic CDZ0203 — exactly as
           construction (case 'an abstract theorem type cannot be forged…') and a multi-variant match (case
           'a multi-variant abstract proof type's constructor match…') already are. `hol` exports the
           abstract handle `Thm` + a smart constructor `refl` but not `Thm`'s constructor `MkThm`; the entry
           tries to destructure a Thm via `(match … (Thm.MkThm c) …)` → CDZ0214. Pins the single-variant
           match-outside diagnostic (v-patterns fixed CDZ0203→CDZ0214 for the newtype-erased path), so a
           kernel accessor written outside the module gets the actionable withheld-ctor message. Completes
           the withheld-constructor coverage: construct + single-variant match + multi-variant match all
           give CDZ0214.")
  (module "hol"
    (do
      (type Thm (MkThm Int64))
      (def (refl (: x Int64)) (Thm.MkThm x))
      (export Thm)
      (export refl)))
  (input  (do
            (import "hol" (Thm refl))
            (def (concl (: t Thm)) (match t ((Thm.MkThm c) c)))
            (export concl)))
  (error  CDZ0214))

; ============================================================================================
; Increment 14 — the HOL AXIOMS via new_axiom (the fusion.ml endgame). An axiom is asserted through
; new_axiom, which mints a HYPOTHESIS-FREE theorem |- tm — the ONE privileged, trusted entry that a
; kernel adds deliberately (HOL's three: extensionality/ETA, choice/SELECT, infinity/INFINITY). Everything
; else is DERIVED; new_axiom is the axiomatic base. Crucially, new_axiom is still the ONLY door to an
; axiom Thm — an importer cannot forge one directly (Thm stays abstract → CDZ0214). This is why a
; fold-role module can be "certified axiom-free": if new_axiom is a visible/tracked entry, a checker can
; see whether a proof depends on an axiom. These cases pin ETA (mints an unhypothetical theorem), axiom
; UNFORGEABILITY, and that an axiom COMPOSES with the inference rules like any other theorem.
; ============================================================================================

(case "the ETA axiom is minted via new_axiom as a hypothesis-free theorem"
  (doc    "The axiomatic base. new_axiom asserts a term as an axiom, minting |- tm with NO hypotheses — the
           one privileged entry a kernel adds deliberately (vs everything else being DERIVED). ETA_AX is
           HOL's function-extensionality axiom: |- (λx. f x) = f. Here eta-ax for f = (Var 9), bound x0,
           yields |- (λx0. (f x0)) = f with an EMPTY hypothesis set. The case checks the exact ETA
           conclusion and that it carries no hypotheses (an axiom holds unconditionally). Pins new_axiom as
           the axiomatic base + the ETA axiom's shape.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Abs v x)  (match b ((Term.Abs w q) (and (= v w) (term-eq x q))) (_ false)))))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (new-axiom (: tm Term)) (Thm.Seq (list) tm))
      (def (eta-ax (: x Int64) (: f Term)) (new-axiom (Term.Eq (Term.Abs x (Term.Comb f (Term.Var x))) f)))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export new-axiom)
      (export eta-ax)
      (export concl)
      (export hyps)))
  (input  (do
            (import "hol" (Term Thm term-eq new-axiom eta-ax concl hyps))
            (def (main)
              (let ((f (Term.Var 9)))
                (let ((th (eta-ax 0 f)))
                  (and (term-eq (concl th) (Term.Eq (Term.Abs 0 (Term.Comb f (Term.Var 0))) f))
                       (match (hyps th) ((list) true) (_ false))))))
            (export main)))
  (output (: true Bool)))

(case "an axiom theorem is unforgeable — new_axiom's Thm cannot be fabricated outside the kernel"
  (doc    "The axiomatic base does not weaken the trust boundary. new_axiom is the KERNEL's privileged
           entry for asserting an axiom; an importer cannot bypass it by building a Thm directly. Forging
           an arbitrary axiom (a bogus |- (Var 1) = (Var 2)) via Thm.Seq outside the kernel is CDZ0214.
           This is what lets a module be 'certified axiom-free': axioms enter ONLY through the tracked
           new_axiom entry, never by fabrication — so the axiomatic dependencies of a proof are exactly the
           new_axiom calls, visible and auditable.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
      (type Thm (Seq (List Term) Term))
      (def (new-axiom (: tm Term)) (Thm.Seq (list) tm))
      (export (. Term *))
      (export Thm)
      (export new-axiom)))
  (input  (do
            (import "hol" (Term Thm new-axiom))
            (def (main) (Thm.Seq (list) (Term.Eq (Term.Var 1) (Term.Var 2))))
            (export main)))
  (error  CDZ0214))

(case "an axiom composes with the inference rules like any other theorem (SYM of ETA)"
  (doc    "An axiom is an ordinary theorem once minted — it feeds the inference rules exactly like a
           derived one. From ETA_AX |- (λx0. (f x0)) = f, SYM derives |- f = (λx0. (f x0)). The case checks
           the flipped equality. Pins that the axiomatic base integrates with the derivation machinery: a
           proof mixes axioms (new_axiom) and derived theorems (rules) uniformly, all minted through the
           kernel, all unforgeable.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Abs Int64 Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Abs v x)  (match b ((Term.Abs w q) (and (= v w) (term-eq x q))) (_ false)))))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (new-axiom (: tm Term)) (Thm.Seq (list) tm))
      (def (eta-ax (: x Int64) (: f Term)) (new-axiom (Term.Eq (Term.Abs x (Term.Comb f (Term.Var x))) f)))
      (def (sym (: th Thm)) (match (concl th) ((Term.Eq a b) (Option.Some (Thm.Seq (hyps th) (Term.Eq b a)))) (_ (Option.None))))
      (export (. Term *))
      (export Thm)
      (export term-eq)
      (export eta-ax)
      (export sym)
      (export concl)))
  (input  (do
            (import "hol" (Term Thm term-eq eta-ax sym concl))
            (def (main)
              (let ((f (Term.Var 9)))
                (let ((lam (Term.Abs 0 (Term.Comb f (Term.Var 0)))))
                  (match (sym (eta-ax 0 f))
                    ((Option.Some s) (term-eq (concl s) (Term.Eq f lam)))
                    ((Option.None) false)))))
            (export main)))
  (output (: true Bool)))

; --- Increment 12 slice-2 ∨/∃ NEIGHBORS (breaker): the unforgeability + hyp-preservation faces skipped ---
; The slice-2 case tests EXISTS-intro's POSITIVE path (matching witness) and DISJ1. These pin the unpinned
; soundness-critical neighbors: EXISTS-intro's NEGATIVE path (a mismatched witness MUST be rejected — the
; unforgeability guard, without which ∃ could be minted from a non-instance), DISJ2 (the mirror intro),
; EXISTS over a body where the witness var is itself free (subst must replace all bound occurrences), and
; EXISTS-intro preserving MULTIPLE hypotheses assembled through kernel rules.

(case "breaker exists: EXISTS-intro rejects a witness whose instance does not match the premise (unforgeability)"
  (doc    "The soundness-critical NEGATIVE path the slice-2 case skips: EXISTS-intro checks the witness before
           minting. body = (Var 0)=(Var 5), claimed witness (Var 5) EXPECTS a premise (Var 5)=(Var 5); given a
           premise (Var 5)=(Var 7) instead (a NON-instance), the term-eq check fails and EXISTS-intro returns
           None. This is the unforgeability guard — an EXISTS-intro that minted ∃x.P from a non-matching
           premise would let a false existential be proven. Asserts the mismatched witness is REJECTED (None).")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Exists Int64 Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Exists v x) (match b ((Term.Exists w q) (and (= v w) (term-eq x q))) (_ false)))))
      (def (subst (: v Int64) (: s Term) (: t Term))
        (match t
          ((Term.Var n) (if (= n v) s (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
          ((Term.Eq a b) (Term.Eq (subst v s a) (subst v s b)))
          ((Term.Exists w body) (if (= w v) (Term.Exists w body) (Term.Exists w (subst v s body))))))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (exists-intro (: x Int64) (: body Term) (: witness Term) (: th Thm))
        (if (term-eq (concl th) (subst x witness body))
            (Option.Some (Thm.Seq (hyps th) (Term.Exists x body)))
            (Option.None)))
      (export (. Term *))
      (export Thm) (export term-eq) (export subst) (export assume) (export exists-intro) (export concl) (export hyps)))
  (input (do
           (import "hol" (Term Thm term-eq subst assume exists-intro concl hyps))
           (def (main)
             (let ((a (Term.Var 5)))
               (let ((body (Term.Eq (Term.Var 0) a))
                     (wrong-premise (Term.Eq a (Term.Var 7))))
                 (match (exists-intro 0 body a (assume wrong-premise))
                   ((Option.Some _) false)
                   ((Option.None) true)))))
           (export main)))
  (output (: true Bool)))

(case "breaker exists: DISJ2 preserves the premise's hypothesis"
  (doc    "The slice-2 case exercises DISJ1; this pins DISJ2, the mirror ∨-introduction. From ASSUME(b) : {b}⊢b,
           DISJ2 with an arbitrary left disjunct a derives {b} ⊢ a∨b — the hypothesis {b} survives. Asserts the
           disjunction conclusion a∨b and that the premise hypothesis is preserved.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Disj Term Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Disj x y) (match b ((Term.Disj p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (disj2 (: a Term) (: th Thm)) (Thm.Seq (hyps th) (Term.Disj a (concl th))))
      (export (. Term *))
      (export Thm) (export term-eq) (export assume) (export disj2) (export concl) (export hyps)))
  (input (do
           (import "hol" (Term Thm term-eq assume disj2 concl hyps))
           (def (main)
             (let ((a (Term.Var 1)) (b (Term.Var 2)))
               (let ((d (disj2 a (assume b))))
                 (and (term-eq (concl d) (Term.Disj a b))
                      (match (hyps d) ((list h) (term-eq h b)) (_ false))))))
           (export main)))
  (output (: true Bool)))

(case "breaker exists: EXISTS-intro with the witness variable also free in the body substitutes all occurrences"
  (doc    "EXISTS-intro when the witness variable is ALSO free in the body: body = (Var 0)=(Var 0), witness (Var 9).
           subst 0 (Var 9) body must replace BOTH bound occurrences, giving (Var 9)=(Var 9); the matching
           premise (Var 9)=(Var 9) is accepted and ∃0.((Var 0)=(Var 0)) is minted. Pins that the witness check
           substitutes every bound occurrence, not just the first.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Exists Int64 Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Exists v x) (match b ((Term.Exists w q) (and (= v w) (term-eq x q))) (_ false)))))
      (def (subst (: v Int64) (: s Term) (: t Term))
        (match t
          ((Term.Var n) (if (= n v) s (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
          ((Term.Eq a b) (Term.Eq (subst v s a) (subst v s b)))
          ((Term.Exists w body) (if (= w v) (Term.Exists w body) (Term.Exists w (subst v s body))))))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      (def (exists-intro (: x Int64) (: body Term) (: witness Term) (: th Thm))
        (if (term-eq (concl th) (subst x witness body))
            (Option.Some (Thm.Seq (hyps th) (Term.Exists x body)))
            (Option.None)))
      (export (. Term *))
      (export Thm) (export term-eq) (export subst) (export assume) (export exists-intro) (export concl) (export hyps)))
  (input (do
           (import "hol" (Term Thm term-eq subst assume exists-intro concl hyps))
           (def (main)
             (let ((body (Term.Eq (Term.Var 0) (Term.Var 0)))
                   (witness (Term.Var 9)))
               (let ((premise (Term.Eq witness witness)))
                 (match (exists-intro 0 body witness (assume premise))
                   ((Option.Some e) (term-eq (concl e) (Term.Exists 0 body)))
                   ((Option.None) false)))))
           (export main)))
  (output (: true Bool)))

(case "breaker exists: EXISTS-intro preserves multiple hypotheses from the premise"
  (doc    "EXISTS-intro carries ALL of the premise’s hypotheses, not just one. The premise is built through kernel
           rules only (conj of two assumptions gathers {p,q}, retarget swaps the conclusion to the witness
           instance) — no raw Thm construction outside the module (which Inc-13 correctly withholds, CDZ0214).
           The minted ∃ keeps both p,q as hypotheses. Asserts hyps has length 2 and both survive.")
  (module "hol"
    (do
      (type Term (Var Int64) (Comb Term Term) (Eq Term Term) (Exists Int64 Term) (Conj Term Term))
      (type Thm (Seq (List Term) Term))
      (def (term-eq (: a Term) (: b Term))
        (match a
          ((Term.Var n)    (match b ((Term.Var m) (= n m)) (_ false)))
          ((Term.Comb x y) (match b ((Term.Comb p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Eq x y)   (match b ((Term.Eq p q) (and (term-eq x p) (term-eq y q))) (_ false)))
          ((Term.Exists v x) (match b ((Term.Exists w q) (and (= v w) (term-eq x q))) (_ false)))
          ((Term.Conj x y) (match b ((Term.Conj p q) (and (term-eq x p) (term-eq y q))) (_ false)))))
      (def (subst (: v Int64) (: s Term) (: t Term))
        (match t
          ((Term.Var n) (if (= n v) s (Term.Var n)))
          ((Term.Comb f x) (Term.Comb (subst v s f) (subst v s x)))
          ((Term.Eq a b) (Term.Eq (subst v s a) (subst v s b)))
          ((Term.Exists w body) (if (= w v) (Term.Exists w body) (Term.Exists w (subst v s body))))
          ((Term.Conj a b) (Term.Conj (subst v s a) (subst v s b)))))
      (def (assume (: p Term)) (Thm.Seq (list p) p))
      (def (concl (: th Thm)) (match th ((Thm.Seq _ c) c)))
      (def (hyps (: th Thm)) (match th ((Thm.Seq h _) h)))
      ; A legitimate 2-hyp premise assembled ONLY through kernel rules (no raw Thm.Seq outside the module):
      ; conj two assumptions to gather {p,q}, then re-label the conclusion to the target via a trusted
      ; rewrite rule `retarget` that keeps the hypotheses and swaps the conclusion for a supplied one.
      (def (conj (: th1 Thm) (: th2 Thm))
        (Thm.Seq (List.concat (hyps th1) (hyps th2)) (Term.Conj (concl th1) (concl th2))))
      (def (retarget (: th Thm) (: c Term)) (Thm.Seq (hyps th) c))
      (def (exists-intro (: x Int64) (: body Term) (: witness Term) (: th Thm))
        (if (term-eq (concl th) (subst x witness body))
            (Option.Some (Thm.Seq (hyps th) (Term.Exists x body)))
            (Option.None)))
      (export (. Term *))
      (export Thm) (export term-eq) (export subst) (export assume) (export conj) (export retarget)
      (export exists-intro) (export concl) (export hyps)))
  (input (do
           (import "hol" (Term Thm term-eq subst assume conj retarget exists-intro concl hyps))
           (def (main)
             (let ((w (Term.Var 5)))
               (let ((body (Term.Eq (Term.Var 0) w))
                     (p (Term.Var 1)) (q (Term.Var 2)))
                 ; premise built through rules: conj(assume p, assume q) -> {p,q} then retarget concl to (Eq w w)
                 (let ((premise (retarget (conj (assume p) (assume q)) (Term.Eq w w))))
                   (match (exists-intro 0 body w premise)
                     ((Option.Some e)
                       (match (hyps e) ((list h1 h2) (and (term-eq h1 p) (term-eq h2 q))) (_ false)))
                     ((Option.None) false))))))
           (export main)))
  (output (: true Bool)))
