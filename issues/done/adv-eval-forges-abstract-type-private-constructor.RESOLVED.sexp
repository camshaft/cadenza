; ADVERSARIAL FINDING (v-metaprogramming, 2026-07-16) — 🚨 SOUNDNESS HOLE, KERNEL-BREAKING for v-verification:
; `eval` FORGES a value of an ABSTRACT type by reaching a module-PRIVATE constructor that the direct
; (hand-written) path correctly rejects. Breaks abstract-type unforgeability — the whole premise of
; v-verification's LCF-style HOL kernel (an attacker could `(eval (quote (Thm.Mk …)))` to forge a theorem).
;
; MECHANISM: a module exports the type HANDLE (abstract) + a smart constructor, but NOT the variant
; constructors. From outside, DIRECT `(Color.Green)` correctly rejects CDZ0214 (AbstractCtor — the
; link-time constructor-visibility gate, `link.rs`). But `(eval (quote (Color.Green)))` — and the
; quasiquote form — RECONSTRUCT the ctor reference and re-resolve/lower it on a path that does NOT re-apply
; that link-time visibility gate, so the private constructor is reached and a forged value is returned.
; `quote (Color.Green)` reifies to `(Ast.List ((. Color Green)))` — a `.`-projection node — and eval's
; reconstruction (`eval_ast::reconstruct`) splices it back for ordinary folding, which resolves the
; projection WITHOUT the abstract-ctor check the direct surface path applies.
;
; SEAM: straddles eval desugar (`eval_ast::reconstruct`, v-metaprogramming) and the RESOLVER/LINK
; constructor-visibility gate (`link.rs` AbstractCtor / CDZ0214). The fix: an eval-reconstructed constructor
; reference MUST be re-resolved under the SAME cross-file visibility as hand-written code (eval gets no
; privileged scope). Escalated to concierge (ownership/split) + reported to v-verification (their §3.4
; soundness conclusion is FALSE as-is). NOT fixed yet — filed so it is tracked and cannot be forgotten.
;
; CONTROL (proves it is an eval-specific BYPASS, not a general visibility gap):
;   DIRECT   (Color.Green) from the entry            → CDZ0214  (correct — the pinned 11-modules case)
;   EVAL     (eval (quote (Color.Green)))            → returns (: (Green unit) Color)  ← FORGED, the bug
;   EVAL qq  (eval (quasiquote (Color.Green)))       → returns (: (Green unit) Color)  ← FORGED
;   PUBLIC   (eval (quote (mk)))  [mk is exported]   → fine (2)  — eval of a PUBLIC name is sound
;   TAGGED   tag returning (quote (Color.Green))     → CDZ0101 first (tagged-template path is safe)
;
; EXPECTED after fix: the two EVAL cases must reject CDZ0214 (or CDZ0101), never return a Color value.

(case "eval of a quoted module-private constructor must NOT forge an abstract-type value"
  (doc    "SOUNDNESS: `eval` must not reach a module-private variant constructor that direct code cannot.
           `lib` exports the abstract handle `Color` + smart ctor `mk` but not `Color`'s variant ctors.
           From the entry, DIRECT `(Color.Green)` rejects CDZ0214; `(eval (quote (Color.Green)))` MUST
           likewise reject (eval re-resolves in the enclosing scope and gets NO privileged visibility),
           never return a forged `Color`. Currently FORGES `(: (Green unit) Color)` — the kernel-breaking
           hole. Fix must re-apply the link-time AbstractCtor visibility gate to eval-reconstructed ctor
           references.")
  (module "lib"
    (do
      (type Color (Red) (Green) (Blue))
      (def (mk) Color.Green)
      (export Color)
      (export mk)))
  (input  (do
            (import "lib" (Color mk))
            (def (main) (eval (quote (Color.Green))))
            (export main)))
  (error  CDZ0214))

; UPDATE 2026-07-16 (corpus-bugfix): OWNERSHIP RESOLVED — concierge assigned v-metaprogramming to own the fix
; (my escalation + v-metaprogramming/v-verification reports converged). v-verification owns the GATE
; COVERAGE (will pin a graded verification-corpus case when the fix lands). No further routing needed.
