; ── LOOP STATUS (2026-07-14, RE-TRIAGED): DATA-CTOR HALF now WORKS + GUARDED @b142578a; only the
; TYPE/MODULE-NAME construct half remains, a design residual needing an operator call. ──
; RE-TRIAGE (verified on spec 53b5bf5d/04a88736): the finding as originally written (iter-258) is now
; MOSTLY resolved. The remaining failure is NARROWER than "all prelude-colliding variant names":
;
;   (1) MATCH-PATTERN half — FIXED @85faf395 (both Int and Some patterns): a bare colliding pattern head
;       resolves against the scrutinee sum's variant set first. Corpus + unit test landed.
;   (2) DATA-CONSTRUCTOR collision (`Some`/`None`/`Ok`/`Err`) in CONSTRUCT position — NOW WORKS and is
;       GUARDED @b142578a. A bare `(Some 5)` on `(type T (Some Int64) (Other Int64))` genuinely builds
;       T's own variant (verified: the two-variant discriminant returns 105/5, not the built-in Option).
;       WHY it works: the built-in sums inject their data-ctor names into `prelude` AFTER
;       `variant_ctor_index`'s `prelude_type_module_names` snapshot (db.rs), so a user variant of that
;       name IS indexed and wins at resolve step 3c. Corpus cases + unit test
;       `a_bare_data_constructor_colliding_variant_constructs_and_matches_as_the_local_variant` landed.
;   (3) TYPE/MODULE-NAME collision (`Int`/`List`/`Name`/`Bytes`/…) in CONSTRUCT position — STILL a
;       residual, an OPERATOR CALL. `variant_ctor_index` DELIBERATELY SKIPS a variant colliding with a
;       prelude TYPE/MODULE name (db.rs:1051, guard against `prelude_type_module_names`) so that bare
;       `Int` stays the width TYPE constructor `(Int 42)` = the 42-bit int type, NOT a data value — the
;       `9f326a2d` global-corruption fix (without the skip, `(type T (Int Int64))` broke bare `Int`
;       everywhere it means the width ctor: a payload `(Int Int64)`, an annotation reduction). So a bare
;       `(Int 42)` construct has TWO live readings (prelude width-type vs T's variant) and only an
;       EXPECTED TYPE disambiguates — which the resolver (running BEFORE inference, no expected-type
;       channel in synthesis mode) cannot see. Making the construct expected-type-directed is a larger,
;       riskier change that re-opens the `9f326a2d` risk; use the qualified `(T.Int 42)` for now.
;
; ⇒ OPEN DESIGN QUESTION for the operator (unchanged from the original deferral, now precisely scoped):
;   should a bare `(Int 42)` construct in a position with a known expected type `T` (annotation /
;   unification) resolve to T's colliding variant, at the cost of expected-type-directed construct
;   elaboration (a bidirectional-check extension) and the re-opened `9f326a2d` invariant? Until decided,
;   the type/module-name-colliding variant is constructed qualified. This file stays ONLY for (3).
;
; ADVERSARIAL FINDING (producer, iter-258, 2026-07-13) — OVER-REJECTION: a user sum variant whose name
; COLLIDES with a prelude entry (Int / List / Name / Some / Ok / …) cannot be CONSTRUCTED or MATCHED via
; the BARE variant name — only the qualified `T.Variant` form works. The bare form resolves to the prelude
; entry (the type constructor `Int`, the collection module `List`, the Option/Result ctor `Some`/`Ok`)
; instead of the local variant, so `(match t ((Int n) n))` on `(type T (Int Int64))` is rejected CDZ0203
; "this constructor pattern is not the constructor of the matched type T". A residual of the variant-shadows-
; prelude bug that `9f326a2d` fixed for TYPE/MODULE usage but not for the bare CONSTRUCT/PATTERN positions.
;
; REPRODUCER (rejected CDZ0203):
;   (do (type T (Int Int64))
;       (def (f (: t T)) (match t ((Int n) n)))          ; bare (Int n) pattern → CDZ0203
;       (def (main) (f (Int 42))) (export main))         ; bare (Int 42) construct also fails
; WORKS (qualified — proving the variant IS valid, only bare resolution shadows):
;   (do (type T (Int Int64)) (def (f (: t T)) (match t ((T.Int n) n))) (def (main) (f (T.Int 42))) …) → 42
; BASELINE (non-colliding name — bare works fine):
;   (do (type T (Foo Int64)) (def (f (: t T)) (match t ((Foo n) n))) (def (main) (f (Foo 42))) …) → 42
; ALSO AFFECTS other prelude-colliding names: (type T (Some Int64)) / (type T (Ok Int64)) with a bare
; (Some n) / (Ok n) pattern → CDZ0203 (the prelude Option/Result variant name shadows the local one).
; BOTH positions fail independently: bare construct with qualified pattern → REJ; qualified construct with
; bare pattern → CDZ0203.
;
; ROOT CAUSE (hypothesis): `9f326a2d` root-caused that a bare variant name resolves BEFORE the prelude
; (`resolve` step 3c precedes the prelude map, step 4), so a variant colliding with a prelude entry shadowed
; it — and fixed the fallout for a name used as a TYPE / MODULE. But the CONSTRUCT and PATTERN resolution of
; a bare variant still consults the prelude for a colliding name: `(Int 42)` / `(Int n)` resolve `Int` to the
; prelude Int type constructor, not `T`'s `Int` variant, so the variant-payload / pattern-constructor check
; sees the wrong constructor and rejects. The qualified `T.Int` form bypasses the prelude and works.
;
; FIX (hypothesis): in construct and match-pattern head resolution, when the head is a bare name that IS a
; variant of the (expected / matched) sum type, prefer the local variant over a colliding prelude entry —
; the same precedence the qualified `T.Int` already gets. In a match the scrutinee's type is known, so the
; pattern head can resolve against its variant set first; in a construct the expected type (from the
; annotation / unification) similarly disambiguates.
;
; SEVERITY: over-rejection — a well-formed program (a user sum with a naturally-named variant like `Int`,
; `Name`, `Some`) is rejected unless every construct/pattern is qualified `T.Variant`. NOT a miscompile
; (rejects, no wrong value). Common in practice: an AST sum `(type Ast (Int …) (Name …) (List …))` — the
; very shape the Ast vertical uses — cannot be pattern-matched with bare variant names. Grades over-rejection.

; REMAINING CASE (residual (3) only — a TYPE/MODULE-name collision `Int` in CONSTRUCT position). The
; pattern half and the data-ctor construct half are both resolved + guarded (see LOOP STATUS); this case
; is the one that still fails, gated on the open expected-type-directed-construct design question. It uses
; the qualified `T.Int` in the PATTERN (which works) so the ONLY failing element is the bare `(Int 42)`
; construct — isolating residual (3). Grades over-rejection (rejects, no wrong value) — gate-invisible.
(case "a bare type-name-colliding variant name constructs as the local variant"
  (doc    "`(type T (Int Int64))` declares a variant named `Int`, colliding with the prelude `Int` TYPE
           constructor (the width ctor `(Int 42)` = the 42-bit int TYPE). In construct position a bare
           `(Int 42)` has two live readings — the prelude width-type vs T's `Int` variant — and only an
           EXPECTED TYPE (`T`, from an annotation / unification) disambiguates, which the resolver (before
           inference) cannot see, so it stays the prelude width type and the program is rejected. This is
           the LAST residual of the variant-shadows-prelude fix (9f326a2d): its `variant_ctor_index` guard
           DELIBERATELY skips a type/module-colliding variant name so bare `Int` keeps meaning the width
           ctor everywhere else. The MATCH half (bare `(Int n)` pattern) and the DATA-ctor construct half
           (bare `(Some 42)`) are both FIXED + guarded; only this type-name construct remains, gated on an
           open operator design call (expected-type-directed construct elaboration). Uses the qualified
           `T.Int` pattern to isolate the construct. Expected once resolved: f (Int 42) → 42.")
  (input  (do
            (type T (Int Int64))
            (def (f (: t T)) (match t ((T.Int n) n)))
            (def (main) (f (Int 42)))
            (export main)))
  (output (: 42 Int64)))
