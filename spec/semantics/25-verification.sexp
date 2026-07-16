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
