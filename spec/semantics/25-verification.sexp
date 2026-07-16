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
