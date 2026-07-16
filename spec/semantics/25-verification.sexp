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
      (def (mk-comb (: th1 Thm) (: th2 Thm))
        (match (concl th1)
          ((Term.Eq f g)
            (match (concl th2)
              ((Term.Eq x y) (Option.Some (Thm.Seq (list) (Term.Eq (Term.Comb f x) (Term.Comb g y)))))
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
      (def (mk-comb (: th1 Thm) (: th2 Thm))
        (match (concl th1)
          ((Term.Eq f g)
            (match (concl th2)
              ((Term.Eq x y) (Option.Some (Thm.Seq (list) (Term.Eq (Term.Comb f x) (Term.Comb g y)))))
              (_ (Option.None))))
          (_ (Option.None))))
      (def (trans (: th1 Thm) (: th2 Thm))
        (match (concl th1)
          ((Term.Eq a b)
            (match (concl th2)
              ((Term.Eq b2 c) (if (term-eq b b2) (Option.Some (Thm.Seq (list) (Term.Eq a c))) (Option.None)))
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
