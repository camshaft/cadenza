; BREAKER FINDING 2026-07-17 (trunk d1d09dfcc) — FIXED-NAME-SET GAP: declaring the SAME EFFECT NAME
; twice is silently accepted with FIRST-WINS semantics, unlike every sibling in the duplicate-name
; family, which all reject CDZ0201:
;   (type T (A)) (type T (B))                    -> CDZ0201 "type `T` is declared more than once" ✓
;   record dup field / sum dup variant           -> CDZ0201 ✓ (05-compound family)
;   module dup def / dup module name             -> CDZ0201 ✓ (11-modules)
;   ONE effect declaring op `a` twice            -> CDZ0201 "operation `a` declared more than once" ✓
;   (bind E "x/one") (bind E "x/two")            -> CDZ0201 bound-more-than-once ✓
; but:
;   (effect E (op a (-> Unit Int64)))
;   (effect E (op b (-> Unit Int64)))            -> COMPILES; E has ONLY op `a` (first wins).
;     performing E.b -> "effect `E` has no operation `b` — closest matches: `a`" (baffling: the user
;     is looking at a declaration of `b` three lines up).
;   Conflicting OP SIGNATURE in the redeclaration — (effect E (op a (-> Unit Int64))) then
;   (effect E (op a (-> Int64 Int64))) -> COMPILES, first signature silently wins (performing at the
;   second signature: "expects Unit, but Int64 was performed").
;   Identical redeclaration -> also silently accepted (harmless but same hole).
;
; 05-compound-types.sexp:127 names the family rule: a record's fields, a sum's variants, a module's
; definitions, and an effect's OPERATIONS are fixed name sets — and 14-effects' `(bind …)`-dup and
; dup-op pins enforce the effect-adjacent instances. The EFFECT NAME ITSELF in the module scope is the
; one member of the family left unenforced (a type name is; an effect name isn't). Same
; statement-registry pattern as [[inline-unit-define-bypasses-cdz0502-uniqueness]]: the declaration
; scan keys effects by name with silent first-wins instead of the fixed-name-set collision check.
;
; SEVERITY: reject-gap, not a miscompile (behavior is consistent first-wins) — but it produces
; baffling diagnostics at USE sites ("no operation `b`" while `b`'s declaration is visibly present)
; and silently drops conflicting signatures, the setup for wrong-effect-row surprises as programs
; grow multi-file (two files each declaring their own `Log` effect will merge first-wins rather than
; collide loudly).
;
; Expected: CDZ0201, matching the type-name twin.
(case "an effect declared more than once is rejected like a duplicate type name"
  (doc    "`(effect E (op a …))` followed by `(effect E (op b …))` declares the name `E` twice in one
           scope — the same fixed-name-set collision as `(type T (A)) (type T (B))` (rejected CDZ0201
           'declared more than once') and the module/record/variant/op-name siblings. Currently the
           second declaration is silently dropped (first wins): `E` has only `a`, performing `E.b`
           reports 'no operation `b`' with `b`'s declaration in plain sight, and a conflicting op
           signature in the redeclaration silently loses. Pins the effect NAME into the family every
           other declaration kind already enforces.")
  (input  (do
            (effect E (op a (-> Unit Int64)))
            (effect E (op b (-> Unit Int64)))
            (def (main) 0)
            (export main)))
  (error  CDZ0201))

; ---
; ROUTED to v-inference (corpus-bugfix 2026-07-17, VERIFIED trunk d1d09dfcc: check rc=0, should CDZ0201).
; Declaration-scan reject-gap: duplicate (effect E ...) silently first-wins (2nd E's ops dropped ->
; baffling "no operation b" at use). Every sibling dup (type/record/sum/module/op-within-effect/bind)
; rejects CDZ0201; the effect NAME is the one unenforced fixed-name-set member. Same missed-check-site
; class as the inline Unit.define CDZ0502 bypass. Owner = resolve declaration scan (v-inference; bounce
; to v-effects if effect-name registration is theirs). Not spawning (fixer cap). Promote when fixed.
