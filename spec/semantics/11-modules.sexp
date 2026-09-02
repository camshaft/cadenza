; Modules — a module binds its name in the enclosing scope to a record of its exports, and carries its
; capability manifest and entry as metadata reached by a (meta …) key that is distinct from every export
; (core-semantics.md §"A Module Binds Its Name In Its Enclosing Scope", §"A Module Evaluates To A Record
; Of Its Exports", §"A Module Carries Its Manifest And Entry As Metadata"; options/code-shape/ for the
; `module`/`def`/`use`/`do`/`.`/`meta` forms). A module is not a separate construct from a record — it is
; a scope-builder whose value IS the record of what its definitions export, so member access
; `(. m export)` reads an export exactly as it reads any record field
; (../learnings/2026-07-03-one-accessor-modules-are-records.md).
;
; A module declaration BINDS its name rather than being an anonymous expression: writing `(module m …)`
; puts `m` in the enclosing scope, so a following form uses `m` directly — no `let` wrapping. The scope
; in which the following form sees the binding is a `(do …)` sequencing block (core-semantics.md
; §Sequencing), which is why these cases are `(do (module m …) <form-using-m>)`.
;
; Scope of this file: SINGLE-module semantics, which the seed realizes (options/realized-capability-set/).
; The seed runs the core cases; a case comparing against a built manifest list needs collections, so a
; generation without collections declines it.
;
; MULTI-FILE PACKAGE composition — explicit imports, visibility, cyclic-dependency rejection, colliding-
; import rejection (modules-and-namespaces.md) — IS now witnessed, at the end of this file, via the
; multi-file case surface: a case carries sibling `(module "name" <prog>)` clauses (library files) whose
; public names its `(input …)` entry may `(import "name" (names…))`. The compiler links the files into
; one component (DESIGN-package-linking.md). A `(module "NAME" …)` clause with a STRING name is a library
; file of a package; the single-file `(module NAME …)` form (bare name) is the in-scope single-module
; record witnessed above — the two are distinct surfaces. (A library body should carry ≥2 forms in its
; `(do …)`: a single-form `do` collapses on the ML surface, so the markdown round-trip would not
; preserve it — write at least a def plus its `(export …)`.)
(diagnostic-quality)

(case
  "a module declaration binds its name in the enclosing scope"
  (doc
    "Witnesses core-semantics.md #A Module Binds Its Name In Its Enclosing Scope: the module
           declaration `(module m …)` puts `m` in scope for the following form of the (do …) block —
           no `let` is needed to name it. Together with #A Module Evaluates To A Record Of Its Exports
           (3rd sentence: an exported definition is reachable by member access), `(. m answer)` reaches
           the export and the outer parens apply the nullary export function.")
  (input
    (do
      (module m
        (def (answer) 42))
      (m.answer unit)))
  (output (: 42 Int64)))

(case
  "a nullary module export applied to a non-unit argument is rejected"
  (doc
    "A nullary export `(def (answer) 42)` is a `Unit -> Int64` function (core-semantics.md §A Nullary
           Function's Argument Type Is Unit), INVOKED by applying it to `unit`. Applying it to a non-unit
           argument — `((. m answer) 5)`, `5 : Int64` — is a type error and MUST be rejected CDZ0203
           (cannot unify Unit with Int64), exactly as a written `(def (f (: u Unit)) 42)` applied to `(f 5)`
           is. The module synthesizes the field as a lambda over one ignored param; that param is ANNOTATED
           `Unit`, not bare — a bare param would take a fresh type variable (typing the export `∀a. a ->
           Int64`) and SILENTLY ACCEPT the wrong argument, running the ill-typed program to 42.")
  (input
    (do
      (def
        (main)
        (do
          (module m
            (def (answer) 42))
          (m.answer 5)))
      (export main)))
  (error CDZ0203))

(case
  "a nullary module export's non-unit argument does not swallow the argument's own fault"
  (doc
    "The companion of the reject above: because the synthesized param is `Unit`-typed rather than a
           free type variable, a non-unit argument is not silently dropped — so its OWN fault surfaces too.
           `((. m answer) (/ 1 0))` is rejected rather than running to 42 with the `(/ 1 0)` discarded (a
           bare-param free variable would accept and drop it). The argument disagrees with Unit → CDZ0203.")
  (input
    (do
      (def
        (main)
        (do
          (module m
            (def (answer) 42))
          (m.answer (/ 1 0))))
      (export main)))
  (error CDZ0203))

(case
  "a module in a top-level do sequence type-checks its members"
  (doc
    "A `(module …)` may sit as an ELEMENT of a top-level `(do …)` sequence root. Its members must be
           type-checked exactly as a bare top-level `(module …)`'s are: an ill-typed nullary member — here
           `(def (bad) (+ 1 2.0))`, a Float/Int mix — MUST be rejected CDZ0301, not silently accepted. This
           position was a type-check hole: the top-level scan registers only def/export/type/effect items
           (no `module` branch), and the nested-declaration walk SKIPS a top-level item as already-scanned,
           so a module here was registered by NEITHER path — its members escaped `collect_faults` while a
           bare `(module m …)` and a def-body-nested one were both checked. Now a top-level module item is
           registered via the shared module-gather, so its member bodies are type-checked wherever the
           module sits (core-semantics.md #A program that is not well-typed MUST be rejected). Also holds
           for a Bool/Int mix and an unbound name in the member.")
  (input
    (do
      (module m
        (def (bad) (+ 1 2.0)))
      (def (main) 5)
      (export main)))
  (error CDZ0301))

(case
  "a top-level-do module member's Bool/Int mix is type-checked and rejected CDZ0203"
  (doc
    "The non-numeric face of the same member-body type-check: a top-level-do module member `(def (bad)
           (+ true 1))` mixes a Bool and an Int64 operand — not a numeric no-promotion clash (CDZ0301) but a
           genuine type mismatch on the `+` operator scheme (CDZ0203), confirming the member-body check
           surfaces EVERY ill-typing, not only the numeric-mix one the Float/Int case above pins. (Migrated
           from rcdzc a_module_in_a_top_level_do_type_checks_its_members.)")
  (input
    (do
      (module m
        (def (bad) (+ true 1)))
      (def (main) 5)
      (export main)))
  (error CDZ0203))

(case
  "a top-level module is named by a top-level def's body"
  (doc
    "Witnesses core-semantics.md #A Module Binds Its Name In Its Enclosing Scope from a NEW position:
           a `(module Temp …)` that is a top-level `(do …)` SEQUENCE ELEMENT — a sibling of the top-level
           defs — binds `Temp` PROGRAM-WIDE, so a reference from another top-level def's BODY (`main`
           calling `(. Temp c-to-f)`) resolves and reaches the export. This shape was rejected CDZ0101
           `unbound name Temp`: a top-level module is registered in `db.modules`, but `resolve_name` walked
           lexical scope (which stops at the root `do`, binding nothing) then defs/types/effects/prelude —
           NONE consulting the module set — so a member-access head naming a top-level module fell off the
           end as unbound, even though the same module referenced DIRECTLY from the root `do` (not through a
           def body) resolved. `resolve_name` now consults top-level modules (a `Ref` to the synth record,
           after defs/types/effects, before the prelude — like a top-level def/type/effect), so `Temp` is
           in scope in every top-level body. The `(export c-to-f)` member — emitted by the ML surface
           `export { c-to-f }` — must not block the module's registration (it is a modeled member). 100°C
           → 212°F.")
  (input
    (do
      (module Temp
        (def (c-to-f c) (+ (/ (* c 9) 5) 32))

        (export c-to-f))
      (def (main) (Temp.c-to-f 100))
      (export main)))
  (output (: 212 Int64)))

(case
  "each definition in a module registers a reachable export field"
  (doc
    "Witnesses core-semantics.md #A Module Evaluates To A Record Of Its Exports (2nd sentence:
           each definition registers its name and value as a field of the module's record): a module
           with two definitions exposes each as a field of its record, so both are reachable by member
           access. Applying both exports and summing shows neither shadows nor displaces the other —
           the record carries a field per definition.")
  (input
    (do
      (module m
        (def (one) 1)

        (def (two) 2))
      (+ (m.one unit) (m.two unit))))
  (output (: 3 Int64)))

; VISIBILITY IS EXPLICIT (modules-and-namespaces.md §Visibility Is Explicit: "Whether a definition is
; visible outside its module MUST be determined by an explicit rule fixed by this specification… A
; definition that is not made visible MUST NOT be importable by another module."). A module's `(export a
; b …)` clause IS that explicit rule — it names exactly the definitions the module's record carries. A
; member NOT named is PRIVATE: absent from the record, so `(. m private)` is the closed-record CDZ0201,
; and no other module can reach it. This is what the ML surface `export { a, b }` compiles to. A module
; with NO export clause is the export-EVERYTHING default (the cases above), unchanged. A private member is
; still MUTUALLY VISIBLE to its siblings inside the module (a private helper stays internally callable);
; only its OUTWARD reachability through the record is withheld.
(case
  "a module member not named by the export clause is private"
  (doc
    "The module exports only `pub`, so `secret` is a private definition — absent from the module's
           export record. Projecting `(. m secret)` names a field the record does not carry, the
           closed-record CDZ0201 (modules-and-namespaces.md §Visibility Is Explicit: a definition not made
           visible MUST NOT be reachable outside the module). Before this, a module's record carried EVERY
           definition regardless of the export clause, so `(. m secret)` reached a private helper — an
           over-exposure the explicit-visibility rule forbids. The export clause now filters the record.
           The message names the MODULE category — 'the `m` module has no member `secret`' — not the
           internal 'record has no field' (a module is a module to the author, not a bare record; the export
           record is an implementation detail). (message pin migrated from rcdzc
           an_absent_user_module_member_names_the_module_not_a_record.)")
  (input
    (do
      (module m
        (def (pub x) (+ x 1))

        (def (secret x) (+ x 100))

        (export pub))
      (def (main) (m.secret 5))
      (export main)))
  (error CDZ0201 (message "the `m` module has no member `secret`")))

(case
  "an absent member of a PRELUDE module names the module, not a record"
  (doc
    "The prelude-module face of the member-miss message: `(. Int64 bogus)` projects a member the
           `Int64` module does not carry. It rejects the same closed-record CDZ0201 a user module does, and
           the message names the MODULE category — 'the `Int64` module has no member `bogus`' — not the
           generic 'record has no field' (a prelude module is not a record to the author). Realized /
           unrealized / absent members are one uniform projection. (migrated from rcdzc
           an_absent_builtin_field_rejects_like_a_closed_record.)")
  (input (do (def (main) Int64.bogus) (export main)))
  (error CDZ0201 (message "the `Int64` module has no member `bogus`")))

(case
  "a plain prelude-member typo gets the ordinary unknown-member error, not a rename hint"
  (doc
    "The retired-rename hint (CDZ0603 '… was renamed …', offered on a fixed set of former member
           names) fires ONLY on that retired set, so a name that was NEVER a member — `Map.siz`, a plain
           typo — still takes the ordinary closed-record CDZ0201 unknown-member error ('the `Map` module
           has no member `siz`'), and the rename hint MUST NOT fire: it never shadows a normal typo. Guards
           the diagnostic-only, tight-retired-set invariant — the negative companion of the retired-name
           rename cases. (migrated from rcdzc a_genuine_member_typo_still_gets_the_ordinary_unknown_member_error.)")
  (input (do (def (main) (Map.siz #map((= 1 2)))) (export main)))
  (error CDZ0201 (message "the `Map` module has no member `siz`") (not "was renamed")))

; The absent-member message names the operand's REAL category, not always "record has no field": an EFFECT's
; op set → "operation", a prelude MODULE → "member" (above), a user SUM type → "variant", a user RECORD →
; "field". Each still carries the same "did you mean `<near>`?" hint. Pins that the category-naming is uniform
; across the member-miss surfaces, so the author sees the right word for the operand they wrote. (Migrated from
; rcdzc an_absent_member_names_the_operand_category_not_always_record.)
(case
  "an absent EFFECT operation names the effect and 'operation', with a did-you-mean"
  (input
    (do
      (effect E (op emit (-> Int64 Unit)) (op log (-> Int64 Unit)))
      (def (main) (host (E) (E.emt 5)))
      (export main)))
  (error CDZ0201 (message "effect `E` has no operation `emt`") (message "did you mean `emit`?")))

(case
  "an absent user-sum VARIANT names the type and 'variant', with a did-you-mean"
  (input (do (type Color (Red) (Green) (Blue)) (def (main) (Color.Gren 5)) (export main)))
  (error
    CDZ0201
    (message "the type `Color` has no variant `Gren`")
    (message "did you mean `Green`?")))

(case
  "an absent user-RECORD field keeps 'record has no field' (a record is a record to the author)"
  (input (do (def (g (: r (Record (: foo Int64)))) r.fooo) (export g)))
  (error CDZ0212 (message "record has no field `fooo`")))

; The far-miss face at the MODULE category (the flagship "match the Rust bar" case): `(. List get)` — `List`
; has no `get` (it is `at`), and `get` is too far to be a confident typo, so the diagnostic LISTS the real
; operations ("closest matches: … `at` …") instead of a baseless "did you mean?" or a dead-end miss, putting
; the fix route in the message. A prelude module rides the same member-miss a user record takes but names the
; MODULE category. (Migrated from rcdzc an_unknown_module_operation_lists_the_available_operations.)
(case
  "an unknown prelude-module operation lists the available operations, not a confident single"
  (input List.get)
  (error
    CDZ0201
    (message "the `List` module has no member `get`")
    (message "closest matches:")
    (message "`at`")
    (not "did you mean")))

; The TIER-1 (confident) complement of the far-miss `List get` above: a CONFIDENT typo of a real module
; member — `List.ln` for `len`, one edit — gets a "did you mean `len`?" AND an APPLYABLE Replace fix on the
; member-key token, so an editor rewrites `ln`→`len` directly (the module-member twin of the record-field
; confident-typo rename). The rename guess is heuristic → the fix is UNVERIFIED. Pins the confident→fix half
; of the two-tier did-you-mean invariant (the far-miss no-fix half is pinned above). (Migrated from rcdzc
; a_confident_module_member_typo_carries_an_applyable_rename_fix.)
(case
  "a confident prelude-module member typo carries an applyable rename fix"
  (input List.ln)
  (error
    CDZ0201
    (message "the `List` module has no member `ln`")
    (message "did you mean `len`?")
    (fix (kind replace) (replacement "len") (unverified))))

; The member suggestions offered for a miss are the operand's REAL members only — internal META CHANNELS (the
; `"meta"`-namespaced `(meta t)`/`(meta apply)`/… type/apply channels a prelude sum module carries) are the
; compiler's own, not user-facing, so they are FILTERED from the closest-matches list. `(Option.Ok 5)` names
; a variant `Option` does not have; the suggestions are the real variants `Some`/`None`, never the internal
; `t`. (Migrated from rcdzc a_meta_channel_field_is_not_offered_as_a_member_suggestion.)
(case
  "an internal meta channel is not offered as a member suggestion"
  (input (Option.Ok 5))
  (error CDZ0201 (message "closest matches:") (message "`Some`") (message "`None`") (not "`t`")))

; The member-operand candidate pool drops prelude VARIANT CONSTRUCTORS (no members → a fix wouldn't resolve),
; but that filter keys on the prelude BINDING's own shape (its `(meta variant)` channel), NOT a name-set
; collision. A prelude name can be BOTH a member-accessible MODULE and some sum's variant — `List` is the
; collection-operations module AND the `Ast.List` node kind — so a name-collision filter would wrongly drop
; the `List` MODULE and suggest the equidistant `Ast` for `Lst`. The module is a real member target, so it
; stays in the pool: `(. Lst len)` suggests `List`. (Migrated from rcdzc
; a_member_accessible_module_sharing_a_variant_name_is_still_suggested — the diagnostic half; the internal
; `nearest()` shared-first-char tie-break stays a rust unit residual.)
(case
  "a member-operand typo suggests a member-accessible module sharing a variant name, with a rename fix"
  (input (do (def (main) (Lst.len #list(1 2))) (export main)))
  (error
    CDZ0101
    (message "did you mean `List`?")
    (fix (kind replace) (replacement "List") (unverified))))

; An `(export X)` where `X` IS declared (not a typo) names the real situation by CATEGORY, not the stale
; "names no definition" (which reads as "unknown name"). A bare TYPE export is the opaque-types abstract-
; HANDLE export — valid but meaningful only to a peer importer, so in a single module it is flagged and points
; at the value / `(. T *)` alternatives. An EFFECT export is a true category error ("names an effect, not a
; value definition"). A GENUINELY unknown name keeps the plain "names no definition" (never "not a value").
; (Migrated from rcdzc exporting_a_type_or_effect_names_the_category_not_names_no_definition.)
(case
  "exporting a bare TYPE handle in a single module names the type-handle export"
  (input (do (type Color R G B) (export Color)))
  (error CDZ0101 (message "names a TYPE") (message "HANDLE export") (message "(. Color *)")))

(case
  "exporting an EFFECT names the category, not a value definition"
  (input (do (effect E (op foo (-> Int64))) (export E)))
  (error CDZ0101 (message "names an effect, not a value definition")))

(case
  "a genuinely unknown export name keeps the plain names-no-definition message"
  (input (do (def (main) 1) (export zzzz)))
  (error CDZ0101 (message "names no definition") (not "not a value")))

; An export naming no definition that is a NEAR-MISS of a defined name (`computee` for `compute`) is a typo:
; CDZ0101 names the candidate ("did you mean `compute`?") AND carries a Replace fix on the export's name atom
; (the export-position analogue of the unbound-name did-you-mean rename). A FAR-miss (nothing close enough)
; states the fault but carries NO fix and no baseless suggestion (a wrong "did you mean?" is worse than none).
; (Migrated from rcdzc a_misspelled_export_carries_a_replace_fix_not_just_a_did_you_mean_string.)
(case
  "a near-miss misspelled export carries a did-you-mean and an applyable replace fix"
  (input (do (def (compute) 1) (export computee)))
  (error
    CDZ0101
    (message "did you mean `compute`?")
    (fix (kind replace) (replacement "compute") (unverified))))

(case
  "a far-miss misspelled export states the fault but offers no baseless suggestion or fix"
  (input (do (def (compute) 1) (export zzzzzzzz)))
  (error CDZ0101 (message "names no definition") (not "did you mean")))

(case
  "a module member named by the export clause is reachable"
  (doc
    "The visible companion of the private case: `pub` IS named by `(export pub)`, so it is a field
           of the module's record and `(. m pub)` reaches it — pub(5) = 6. Pins that filtering the record
           to the export clause does not withhold a NAMED export (only the unnamed `secret` is hidden).")
  (input
    (do
      (module m
        (def (pub x) (+ x 1))

        (def (secret x) (+ x 100))

        (export pub))
      (def (main) (m.pub 5))
      (export main)))
  (output (: 6 Int64)))

(case
  "a private module member is still visible to a sibling"
  (doc
    "Explicit visibility withholds a member's OUTWARD reachability, not its INTRA-module visibility:
           `helper` is not exported (so `(. m helper)` from outside would be CDZ0201), but `pub` — which IS
           exported — calls `helper` by name in its own body, exactly as any two module definitions are
           mutually visible (§A Module Function Calls A Sibling Export By Name). So the export clause hides
           `helper` from the record while `pub`'s body still reaches it: pub(3) = helper(3) + 1 = 7. Pins
           that the visibility filter touches only the export record (`modules::module_record`), not the
           sibling scope (`resolve::module_sibling_binds`), so a private helper stays internally callable.")
  (input
    (do
      (module m
        (def (helper x) (* x 2))

        (def (pub x) (+ (helper x) 1))

        (export pub))
      (def (main) (m.pub 3))
      (export main)))
  (output (: 7 Int64)))

(case
  "a private sibling defined after its exported caller still resolves"
  (doc
    "The DEFINITION-ORDER companion of the private-sibling case above: there `helper` precedes its
           caller; here the exported `pub` comes FIRST and forward-references the private `helper` defined
           after it. Sibling visibility is order-independent (every member sees every member), and the
           privacy filter must not interact with the forward-reference path: pub(21) = helper(21) = 42.
           A resolver that binds siblings in definition order — or one that consults the (filtered) export
           record for a not-yet-seen name — breaks exactly this shape.")
  (input
    (do
      (module m
        (export pub)

        (def (pub (: x Int64)) (helper x))

        (def (helper (: x Int64)) (* x 2)))
      (m.pub 21)))
  (output (: 42 Int64)))

(case
  "a mutually-recursive pair fully named by the export clause resolves"
  (doc
    "A mutual-recursion CYCLE inside a module with an export clause naming BOTH members:
           even↔odd, `((. m even) 4)` = 1 (4 is even; the cycle bottoms out through 4→3→2→1→0). The
           knot-tying for a mutually-recursive module group must survive the presence of an export
           clause — this pins the both-exported face (the export-everything default cycle already works;
           the one-private face is the open false-rejection filed as
           adv-private-module-member-in-mutual-recursion-false-reject).")
  (input
    (do
      (module m
        (export even odd)

        (def (even (: n Int64)) (if (= n 0) 1 (odd (- n 1))))

        (def (odd (: n Int64)) (if (= n 0) 0 (even (- n 1)))))
      (m.even 4)))
  (output (: 1 Int64)))

(case
  "a private module member participates in mutual recursion with an exported sibling"
  (doc
    "The ONE-PRIVATE face of the both-exported cycle above (the false-rejection filed as
           adv-private-module-member-in-mutual-recursion-false-reject): `even` is exported, `odd` is
           PRIVATE (absent from the export clause), and the two are mutually recursive, so `((. m even)
           4)` = 1. Sibling visibility is independent of the export clause (§A Module Function Calls A
           Sibling Export By Name; the clause governs OUTWARD reachability through the record, not sibling
           scope) — so a private cycle member must resolve its exported co-member exactly as the both-
           exported case does. The privacy landing built NO synth field for the private `odd`, so `odd`'s
           body — unlike an exported member's, which is reparented under its synth field lambda beneath the
           module record where sibling resolution (`module_sibling_binds`) fires — kept its source parent
           and its scope walk ascended through the `(module …)` form, which did not resolve siblings; so
           `odd`'s call to `even` rejected CDZ0101 exactly when `odd` participated in the cycle (a one-
           directional reference to a private sibling, from an EXPORTED body that does reach the record,
           resolved fine either order — the cases above). The `(module …)` form now resolves siblings too
           (`resolve::module_form_sibling_binds`), so a private member's body sees its siblings as an
           exported member's does. Hiding the private half of a recursive helper pair is the privacy
           feature's canonical use. Expected: 1.")
  (input
    (do
      (module m
        (export even)

        (def (even (: n Int64)) (if (= n 0) 1 (odd (- n 1))))

        (def (odd (: n Int64)) (if (= n 0) 0 (even (- n 1)))))
      (m.even 4)))
  (output (: 1 Int64)))

(case
  "one module's export clause does not affect a same-named member of another module"
  (doc
    "Privacy is PER-MODULE state: module `a` exports only `pub` (hiding its `helper`), while module
           `b` has NO export clause, so ITS `helper` keeps the export-everything default — `(. b helper)`
           reaches it (7 × 3 = 21) even though a same-named member of `a` is private. A privacy filter
           keyed by NAME rather than by (module, name) — e.g. a global hidden-names set — would let `a`'s
           clause shadow `b`'s member. Pins the filter's scope is the declaring module's record only.")
  (input
    (do
      (module a
        (export pub)

        (def (helper (: x Int64)) (* x 2))

        (def (pub (: x Int64)) (helper x)))
      (module b
        (def (helper (: x Int64)) (* x 3)))
      (b.helper 7)))
  (output (: 21 Int64)))

; A module definition need not be a function: the glossary defines a Definition as "a named binding
; introduced by a module: a value, function, type, …", and core-semantics.md #A Module Evaluates To A
; Record Of Its Exports says "Each definition MUST register its name and value as a field of the module's
; record." So a VALUE definition `(def v 7)` registers `v` as a field bound to 7, and `(. m v)` projects
; it directly — no application, because the field IS the value, not a nullary function. This is the
; value-definition companion of the function-export case above (which reaches each export by APPLYING it,
; `((. m one) unit)`). A compiler that registers only function definitions as fields drops the value
; definition: `(. m v)` then names a field the record does not carry and — since the module component was
; still emitted — TRAPS at run time (member access on a missing field, core-semantics.md #Member Access),
; rather than yielding 7. Emitting a component that traps on a well-typed projection is a decline-don't-
; miscompile violation (the correct not-yet-covered behavior is to decline, never to trap on a valued
; program). A generation that does not yet register a module's value definitions declines rather than
; emitting a component whose export access traps.
(case
  "a module value definition registers a reachable export field"
  (doc
    "The value-definition companion of the case above: `(def v 7)` is a value definition, not a
           function, so `(. m v)` projects the field directly (no `unit` application) and yields 7 —
           core-semantics.md #A Module Evaluates To A Record Of Its Exports (each definition registers its
           name and value as a field) with the glossary's Definition = 'a value, function, type'. A
           compiler that registers only function definitions drops `v`; `(. m v)` then traps at run time
           on a missing field of an emitted component — a decline-don't-miscompile violation, since the
           program is well-typed and its value is 7. A generation that does not yet register value
           definitions MUST decline rather than emit a component that traps.")
  (input
    (do
      (module m
        (def v 7))
      m.v))
  (output (: 7 Int64)))

; Because each definition registers its name as a FIELD of the module's record (core-semantics.md #A
; Module Evaluates To A Record Of Its Exports, "Each definition MUST register its name and value as a
; field of the module's record"), and a record is "a fixed SET of statically-known field names" (#A
; Record Has A Fixed Set Of Named Fields), TWO definitions of the same name in one module would register
; the field twice — the exact ill-formedness the record-literal case `(record (a 1) (a 2))` is rejected
; for (CDZ0201, "names the field more than once"). A module with two `(def (f) …)` is therefore ill-typed
; and MUST be rejected (CDZ0201), not resolved by an implicit precedence — the same principle
; modules-and-namespaces.md #Importing states for imports ("Importing two definitions under the same name
; into one scope MUST be a compile-time error rather than resolved by an implicit precedence"), here for
; two definitions written in one module. A compiler that keeps the FIRST definition and silently discards
; the second (so `(f)` = 1) resolves the collision by an implicit first-wins precedence the record field
; set forbids — the module-definition companion of the duplicate-record-field gap. A generation that does
; not yet check for a duplicate definition declines rather than silently choosing one.
(case
  "a module with two definitions of the same name is rejected"
  (doc
    "`(def (f) 1)` and `(def (f) 2)` both register the field `f` of the module's record — but a
           record has a FIXED SET of field names (core-semantics.md #A Record Has A Fixed Set Of Named
           Fields), so registering `f` twice is the same ill-formedness the record literal `(record (a 1)
           (a 2))` is rejected for (CDZ0201). The module MUST be rejected, not resolved by keeping the
           first definition and discarding the second (which yields `(f)` = 1) — an implicit first-wins
           precedence the fixed field set forbids, exactly as modules-and-namespaces.md #Importing forbids
           resolving two same-named imports by precedence. Pins that the duplicate-field check reaches a
           module's definitions, not only a record literal's fields (core-semantics.md #A Module Evaluates
           To A Record Of Its Exports: each definition registers its name as a field). A generation that
           does not yet detect a duplicate definition declines rather than silently choosing one.")
  (input (do (def (f) 1) (def (f) 2) (def (main) (f)) (export main)))
  (error CDZ0201))

(case
  "two sibling modules may each define a private helper of the same name"
  (doc
    "The duplicate-definition check is PER-MODULE, not global across a linked package: a module's
           name set is fixed WITHIN one module (the rejection above), but two SEPARATE files may each carry
           a private helper of the same name. `lib` defines a private `foo` (`+ x 1`) that its exported
           `bump` calls; the entry defines its OWN private `foo` (`* x 2`). Each `foo` binds to its own
           module's definition — `(foo 5)` in the entry is 10 (entry's `* 2`), `(bump 5)` is 6 (lib's `+ 1`
           via lib's `foo`), summing to 16 — so the two same-named helpers do NOT collide and neither wins
           the other's calls. The value twin of the per-module TYPE-declaration case (two `Box` types in
           separate modules); the idiomatic multi-module layout where a shared type module is imported by
           several passes, each with its own generically-named local helper (`node-count`, `foo`). A global
           seen-set falsely flagged this as a duplicate (CDZ0201); the check keys on `(file, name)`.")
  (module "lib"
    (do (def (foo (: x Int64)) (+ x 1)) (def (bump (: x Int64)) (foo x)) (export bump)))
  (input
    (do
      (import "lib" (bump))
      (def (foo (: x Int64)) (* x 2))
      (def (main) (+ (foo 5) (bump 5)))
      (export main)))
  (output (: 16 Int64)))

; The two-sibling case above pins that internal CALLS resolve per-file. Its export-boundary TWIN: the
; ENTRY file's EXPORTED name must also bind per-file. When a sibling defines the SAME name as the entry's
; export (e.g. both `main`), the component's exported `main` MUST be the ENTRY's own def — a same-named
; sibling def (spliced first, even a private one) must not hijack the export. The export used to bind
; through the package-wide first-wins `def_of_name`, so the alphabetically-first file's `main` won the
; export and BOTH entry choices ran the wrong file's code — a SILENT wrong program (no diagnostic, a
; plausible value). Internal calls resolved per-file (the case above); only the export boundary missed it.
(case
  "a package entry's exported def wins over a same-named def in a library file"
  (doc
    "Two package files each define `main`; the ENTRY (`main`, `* 7`) and a library `aaa` (`* 100`).
           The component's exported `main` must be the ENTRY file's own def — n=3 → 21 — not the library's
           (n*100 = 300). The flat cross-file `def_of_name` bound the export to the first-spliced file's
           `main` (`aaa`, a library), so the entry's exported `main` silently ran `aaa`'s code. The
           export boundary now resolves the exported name in the file that WROTE the `(export …)` clause
           — the same per-file rule internal calls use (DESIGN-package-linking.md §4). A private (un-
           exported) sibling `main` would hijack identically; both are the one missed resolution site.")
  (module "aaa"
    (do (def (main (: n Int64)) (* n 100)) (export main)))
  (input (do (def (main (: n Int64)) (* n 7)) (export main)))
  (call main (: 3 Int64))
  (output (: 21 Int64)))

; A module declaration BINDS its name in the enclosing scope (the case at the top of this file), so two
; `(module a …)` in ONE scope are a fixed-name-set collision — the same ill-formedness as a duplicate `def`
; or type, rejected CDZ0201 ('module `a` is declared more than once'). Distinct from two SEPARATE-file
; modules sharing a private helper name (the per-(file,name) case above): that is cross-module and fine;
; this is two modules claiming the SAME name in the SAME scope. Two DISTINCT-named modules coexist (control).
(case
  "a package entry's exported def wins over an alphabetically-later same-named library def"
  (doc
    "The mirror of the case above, pinning that the entry wins regardless of the library's sort
           order: here the ENTRY's `main` (`* 100`) must win over a library `zzz`'s same-named `main`
           (`* 7`), where the library name sorts AFTER the entry. n=3 → 300, not 21. Together with the
           earlier-sorted-library case above (a library `aaa` losing to the entry), this covers both
           splice orderings — the export always binds the entry file's own def, never a same-named
           sibling's, no matter which file the flat cross-file scan would have spliced first.")
  (module "zzz"
    (do (def (main (: n Int64)) (* n 7)) (export main)))
  (input (do (def (main (: n Int64)) (* n 100)) (export main)))
  (call main (: 3 Int64))
  (output (: 300 Int64)))

(case
  "a duplicate module declaration in one scope is rejected"
  (doc
    "Two `(module a …)` declarations in the same `(do …)` scope both bind the name `a` — a fixed-name-
           set collision, so the second rejects CDZ0201, exactly as a duplicate `def a` or a duplicate type
           `a` does (a module binds its name in the enclosing scope, so the name-uniqueness rule applies).
           Contrast the two-separate-files same-named-helper case above (per-(file,name), allowed): here both
           modules are in ONE scope claiming ONE name.")
  (input
    (do
      (module a
        (def (g) 1)

        (export g))
      (module a
        (def (h) 2)

        (export h))
      (def (main) 0)
      (export main)))
  (error CDZ0201 (message "module `a` is declared more than once") (fix (kind delete))))

; A bare-name `(module NAME …)` body is a SEQUENCE OF DEFINITIONS listed directly as members — not a
; `(do …)` block. A `(do …)` member is a category error: the module registers no exports and its name
; binds nothing, which used to surface ONLY as a misleading bare CDZ0101 "unbound name `m`" at the USE
; site (a silent misbind — no error at the module form). Now it is named AT THE WRAPPER, CDZ0201, so an
; author who copied the STRING-name library/file form `(module "lib" (do …))` — where the `(do …)` IS
; the file's whole program (linker path) — onto the bare-name declaration form gets a form-site fix
; instead of hunting a phantom unbound name. Bare-def bodies (every other case in this file) are the
; correct form and bind normally; only the `(do …)` WRAPPER on the bare-name form is rejected.
(case
  "a bare-name module body wrapped in a (do …) block is rejected at the module form"
  (doc
    "`(module m (do (def (answer) 42)))` wraps the module's members in a `(do …)` block. A bare-name
           `(module NAME …)` body is a def-SEQUENCE (`(module m (def …) (def …))`), so a `(do …)` member is
           not a definition — the module registers no exports and `m` binds nothing. This used to fail SILENTLY
           at the module form and surface only as a misleading bare CDZ0101 `unbound name m` at the `(m.answer
           unit)` use site (breaker finding). It now rejects CDZ0201 AT the `(do …)` wrapper, naming the rule
           and the fix (remove the `(do …)`, list the defs as members). The confusion is real because the
           STRING-name library form `(module \"lib\" (do …))` DOES take a `(do …)` body — there the `(do …)`
           is the file's whole program, resolved by the linker — so this pins the bare-name declaration form's
           distinct grammar. The downstream `unbound name m` still reports as the symptom; the CDZ0201 root
           sorts first (anchored at the earlier `(do …)` member).")
  (input
    (do
      (def (main)
        (do
          (module m (do (def (answer) 42)))
          (m.answer unit)))
      (export main)))
  (error CDZ0201 (message "body is a sequence of definitions")))

; The GENERAL form of the do-wrapper case above: a `(module NAME …)` member position is STRICTLY a
; declaration (`def`/`type`/`effect`/`op`/`module`/`doc`/`export`, or a `pragma` directive) — unlike a
; top-level `(do …)` block there is no expression-statement reading of a module member. So ANY
; non-declaration member (an application `(foo …)`, a `let`/`if`/`match`/`fn` expression, a bare
; literal) makes the module fail to register and used to surface only as a misleading bare CDZ0101
; unbound-name at the use site. It is now rejected CDZ0201 AT the member, naming the declaration set.
(case
  "a non-declaration member of a bare-name module is rejected at the member"
  (doc
    "`(module m (foo 1) (def (answer) 42))` puts an APPLICATION `(foo 1)` where a declaration must be.
           A `(module NAME …)` member is a declaration (def/type/effect/op/module/doc/export or a pragma),
           never an expression — so a non-declaration member is a category error: the module registers no
           exports and `m` binds nothing. It used to fail SILENTLY at the module form and surface only as
           `unbound name m` at the `(m.answer unit)` use site; it now rejects CDZ0201 AT the `(foo 1)`
           member, naming the declaration set (the general form of the do-wrapper case above). The
           downstream `unbound name m` still reports as the symptom; the CDZ0201 root sorts first.")
  (input
    (do
      (def (main)
        (do
          (module m (foo 1) (def (answer) 42))
          (m.answer unit)))
      (export main)))
  (error CDZ0201 (message "member must be a declaration")))

; A NESTED module's `(export …)` clause naming a member the module does not declare is the nested-module
; analogue of the top-level "export names no definition" (CDZ0101). The top-level export check reads the
; PROGRAM's exports, never a nested `(module …)`'s clause, so a nested `(export b)` with no `(def b …)`
; used to be SILENTLY accepted (the export set only FILTERS the module's record). It is now rejected
; CDZ0101 at the offending export name, with a did-you-mean over the module's declared member names.
(case
  "a nested module exporting a name it does not declare is rejected"
  (doc
    "`(module m (def (a) 1) (export a b))` exports `b`, which the module never declares. A module's
           exports must name its own definitions (core-semantics.md §A Module Evaluates To A Record Of Its
           Exports), so this is ill-formed — the nested-module twin of the top-level `export names no
           definition`. It used to compile silently (the export set is only a record filter, so an unknown
           name filtered nothing); it now rejects CDZ0101 at `b`. The valid export `a` is unaffected; a
           near-miss (e.g. `ax`) would carry a did-you-mean over the declared members.")
  (input
    (do
      (def (main)
        (do
          (module m (def (a) 1) (export a b))
          (m.a unit)))
      (export main)))
  (error CDZ0101 (message "names no definition in the module")))

(case
  "two distinct-named modules in one scope coexist"
  (doc
    "The control: two modules with DIFFERENT names `a` and `b` in one scope are both bound and both
           reachable — `((. a g) unit)` = 1, `((. b h) unit)` = 2, summing to 3. Pins that the duplicate-
           module rejection fires ONLY on a name collision, never on two legitimately-distinct modules in
           the same scope (a global over-tight check would wrongly reject this).")
  (input
    (do
      (module a
        (def (g) 1)

        (export g))
      (module b
        (def (h) 2)

        (export h))
      (def (main) (+ (a.g unit) (b.h unit)))
      (export main)))
  (output (: 3 Int64)))

; A duplicate EXPORT clause is the export-side analogue of the duplicate definition above: a module's
; exports are a record whose fields are the exported names (core-semantics.md #A Module Evaluates To A
; Record Of Its Exports), and a record has a fixed set of field names, so exporting the same name twice
; places two entries under one field — the same CDZ0201 ill-formedness. It MUST be rejected before
; emitting: two export entries of one name are forbidden by the component binary format, so emitting
; them produces a component that fails to parse — a decline-don't-miscompile violation.
(case
  "a duplicate export clause for the same name is rejected"
  (doc
    "`(export a)` twice names the export `a` twice. A module's exports are a record with a fixed
           set of field names, so a repeated export is the CDZ0201 duplicate-field ill-formedness — the
           export analogue of the duplicate definition above and of `(record (a 1) (a 2))`. The compiler
           MUST reject it (CDZ0201), never emit a component with two export entries named `a` (which the
           component binary format forbids, so the emitted bytes fail to parse). Carries a DELETE fix on the
           redundant later `(export a)` (the earlier one already makes `a` public). Fix-quality migrated from
           rcdzc a_duplicate_export_carries_a_delete_the_duplicate_fix.")
  (input (do (def (a) 42) (export a) (export a)))
  (error CDZ0201 (message "exported more than once") (fix (kind delete))))

(case
  "a duplicate export of the entry is rejected"
  (doc
    "The `main` sibling: `(export main)` twice. Same CDZ0201 duplicate-export rejection — the
           defect is independent of the exported name, not special to the entry-selection path.")
  (input (do (def (main) 42) (export main) (export main)))
  (error CDZ0201))

(case
  "a duplicate name WITHIN one multi-name export clause is rejected with a delete fix"
  (doc
    "The within-CLAUSE form of the duplicate export (the cross-clause form is above): `(export main
           main)` names `main` twice in ONE clause. Same CDZ0201 'exported more than once' + a DELETE fix on
           the redundant occurrence (the first `main` survives — the whole clause is not deleted). Pins that
           the multi-name export scanner checks EACH name for duplicates, not only across clauses.")
  (input (do (def (main) 1) (export main main)))
  (error CDZ0201 (message "exported more than once") (fix (kind delete))))

(case
  "an undefined name in the 2nd+ position of a multi-name export clause is caught with a did-you-mean"
  (doc
    "A diagnostic on the 2nd (or later) name of a multi-name export anchors to THAT name, not the
           clause's first: `(export main helpr)` — `helpr` names no definition → CDZ0101 with a did-you-mean
           over the defined names (`helper`). Pins that the multi-name export scanner resolves EVERY name (a
           bug once read only `tail.first()`, silently dropping the rest).")
  (input (do (def (main) 1) (def (helper) 2) (export main helpr)))
  (error CDZ0101 (message "helpr") (message "did you mean `helper`?")))

(case
  "a duplicate type declaration is rejected with a delete fix"
  (doc
    "A module's TYPE names are a fixed set exactly as its def / export / variant / operation names
           (the sixth closed name-set). `(type T (A)) (type T (B))` declares `T` twice — the same fixed-
           name-set collision as a duplicate def or export, CDZ0201 (declared more than once), carrying a
           DELETE fix on the redundant second `(type …)`. Before, `T` silently resolved to the FIRST so a
           `T.B` reference failed confusingly. Migrated from rcdzc
           a_duplicate_type_declaration_is_rejected_and_carries_a_delete_fix (fix-quality now graded via C1).")
  (input (do (type T (A)) (type T (B)) (def (f (: x T)) x) (export f)))
  (error CDZ0201 (message "declared more than once") (fix (kind delete))))

(case
  "two distinct type names are not a duplicate"
  (doc
    "NO OVERREACH twin of the duplicate-type reject: two DIFFERENTLY-named types coexist (the closed
           name-set collision keys on the NAME, not on there being two `(type …)` forms). `(type T (A))`
           + `(type U (B))` compiles clean and `main` returns `(T.A)` — a nullary variant of `T`.")
  (input (do (type T (A)) (type U (B)) (def (main) (T.A)) (export main)))
  (call main)
  (output (: unit T)))

; --- A non-kebab export name crosses under a normalized kebab-case extern name ------------------------
; A Cadenza identifier may contain uppercase letters (`fA`, `Foo`) or underscores (`my_func`) — all valid
; source names — but the component model requires an export's extern name to be KEBAB-CASE (lowercase
; words, hyphen-separated). Emitting a non-kebab name verbatim produces a component that fails to validate
; (an unloadable artifact). The compiler NORMALIZES a non-kebab export name to a valid kebab extern name
; (`fA` → `f-a`, `my_func` → `my-func`) — deterministically, so a caller still names the export by its
; source identifier and the runner resolves it through the same rule. Two DISTINCT source names that
; normalize to the SAME extern name is a collision the compiler rejects (CDZ0201), like a duplicate export.
(case
  "an export whose name is not kebab-case crosses under a normalized extern name"
  (doc
    "`(def (fA (: x Int64)) (+ x 1))` with `(export fA)` — `fA` is a valid Cadenza identifier
           (uppercase identifiers are legal) but NOT a valid component extern name. Rather than emit an
           unloadable component (the old miscompile: `export name fA is not a valid extern name`), the
           compiler normalizes the extern name to kebab-case `f-a`; the export is invoked by its SOURCE
           name `fA`, which the runner resolves through the same normalization. `(fA 5)` = 6. Pins that a
           non-kebab export name produces a LOADABLE component, not a silently-invalid artifact.")
  (input (do (def (fA (: x Int64)) (+ x 1)) (export fA)))
  (call fA (: 5 Int64))
  (output (: 6 Int64)))

(case
  "an underscore export name crosses under a normalized extern name"
  (doc
    "The underscore shape: `(def (my_func (: x Int64)) (+ x 1))` with `(export my_func)` normalizes
           to the kebab extern name `my-func`. `(my_func 5)` = 6. Confirms the normalization covers the
           underscore separator, not only camelCase — every non-kebab source name yields a loadable
           component.")
  (input (do (def (my_func (: x Int64)) (+ x 1)) (export my_func)))
  (call my_func (: 5 Int64))
  (output (: 6 Int64)))

(case
  "two exports normalizing to the same kebab extern name are rejected"
  (doc
    "`(export fA)` and `(export f-a)` both normalize to the extern name `f-a` — a collision the
           component boundary cannot carry (two exports of one name). The compiler rejects it CDZ0201, the
           same duplicate-export ill-formedness as two identical export names, rather than silently
           merging or dropping one. Distinct from the duplicate-export cases above: here the SOURCE names
           differ (`fA` vs `f-a`) but their normalized extern names coincide.")
  (input
    (do (def (fA (: x Int64)) (+ x 1)) (def (f-a (: y Int64)) (+ y 2)) (export fA) (export f-a)))
  (error CDZ0201))

(case
  "an export name with a digit-led kebab segment is rejected, not silently invalid"
  (doc
    "A hyphen is a legal Cadenza identifier character, so `step-by-2` is a valid source name — but a
           component-model extern name requires each `-`-separated segment to START WITH A LETTER (the
           `KebabStr` grammar wasmparser validates against), so the trailing segment `2` makes `step-by-2`
           NOT a valid extern name. Unlike the camelCase/underscore names above, `kebab_extern_name` cannot
           normalize it — it keeps `-`/digits verbatim, so it maps `step-by-2` to ITSELF (still invalid).
           Emitting it produced a component wasmtime rejects WHOLESALE at load, with NO compiler diagnostic
           — for a `@test` build every test in the file 'failed'; for a plain build the artifact was
           unloadable (the kebab-extern-name gotcha, silent-miscompile face). The compiler now rejects it
           CDZ0201 before emit, naming the offending name and the fix (rename so every segment begins with
           a letter — `step-by-two` / `step-by2`), the export-NAME analogue of the interface-name and
           collision rejects above.")
  (input (do (def (step-by-2 (: x Int64)) (+ x 1)) (export step-by-2)))
  (error CDZ0201))

(case
  "an export name with a non-ASCII segment is rejected, not silently invalid"
  (doc
    "The third invalid-kebab-extern-name face (alongside the digit-led segment above and the
           normalization collision): a NON-ASCII source name. `café` is a legal Cadenza identifier, but a
           component-model extern name's `KebabStr` grammar admits only ASCII letters/digits/hyphens, so
           `café` (the `é`) cannot form a valid extern name and `kebab_extern_name` cannot normalize it to
           one. Emitting it produced a component wasmtime rejects at load with NO compiler diagnostic (the
           non-ASCII-export-name mangle, silent-miscompile face). The compiler now rejects it CDZ0201 before
           emit — on BOTH backends (the rust backend, which emits no component, previously emitted a `pub fn`
           silently where wasm rejected; the rust `emit` now runs the same `invalid_kebab_export_name` check
           at its top). Pins the non-ASCII member of the export-name-validity check at both-backend parity.")
  (input (do (def (café) 42) (export café)))
  (error CDZ0201))

(case
  "a top-level value definition binds a name usable by the program's functions"
  (doc
    "A definition is 'a value, function, type' (glossary), and each registers its name in the module
           (core-semantics.md #A Module Evaluates To A Record Of Its Exports). So a VALUE definition
           `(def answer 42)` at the program's top level MUST bind `answer` for the module's functions to
           reference, exactly as a function definition binds its name — `(def (main) answer)` yields 42. The
           nested-module value-def case earlier in this file (`(do (module m (def v 7)) (. m v))`) pins the
           same rule for a module in do-position; this pins it for the OUTER program module. A compiler that
           accepts only function definitions `(def (f …) …)` at top level rejects this well-typed program
           (\"def without a signature\") — but a value definition is an ordinary definition form, so it MUST
           bind here. (This is load-bearing for a Cadenza-authored compiler whose shared tables — e.g. an
           opcode record generated as `(def op (record …))` — are top-level value definitions.)")
  (input (do (def answer 42) (def (main) answer) (export main)))
  (output (: 42 Int64)))

(case
  "a top-level value definition binds a record projected by the program's functions"
  (doc
    "The record companion of the scalar value-def above: a top-level value definition may bind a
           RECORD, and a function projects its fields by member access (core-semantics.md #Member Access
           Projects A Record Field). `(def tbl (record (a 7) (b 8)))` binds `tbl`; `(. tbl b)` is 8. This is
           exactly the shape a Cadenza-authored compiler's generated opcode table takes — `(def op (record
           (i64-const 0x42) …))` — a top-level record value read by `(. op i64-const)`, so it is load-bearing
           for self-hosting. A compiler that accepts only function definitions at top level rejects this
           well-typed program (\"def without a signature\"); a value definition binding a record MUST bind
           here and project.")
  (input (do (def tbl #record((= a 7) (= b 8))) (def (main) tbl.b) (export main)))
  (output (: 8 Int64)))

(case
  "a top-level value definition may reference a value defined later in the module"
  (doc
    "A module's definitions form a mutually-visible scope, not a top-to-bottom sequence: a value
           definition may reference a name bound by a LATER definition (core-semantics.md #A Module
           Evaluates To A Record Of Its Exports — every top-level name is in scope in every definition's
           body). `(def b (+ a 4))` uses `a`, which is defined AFTER it as `(def a 3)`; the module resolves
           `a` = 3 regardless of order, so `b` = 7. Pins that value-definition resolution is order-independent
           (a compiler that resolved names strictly top-to-bottom would report `a` unbound in `b`'s body),
           the same forward visibility a function definition already enjoys.")
  (input (do (def b (+ a 4)) (def a 3) (def (main) b) (export main)))
  (output (: 7 Int64)))

(case
  "a value definition may carry a leading doc, like a function definition"
  (doc
    "A `(doc …)` form immediately after the definition's name/signature documents it and is not part
           of the value; a FUNCTION definition already accepts one (`(def (f) (doc \"…\") body)`), and a
           VALUE definition MUST accept one symmetrically — a definition is 'a value, function, type'
           (glossary), so the doc affordance cannot depend on which. `(def answer (doc \"the answer\") 42)`
           binds `answer` = 42 with the doc ignored for the value. A compiler that reads a value def as
           exactly name+value rejects the doc'd form (\"value def without a single value expression\") while
           accepting the doc'd function form — an asymmetry a definition form must not have. Load-bearing
           for a Cadenza-authored compiler whose generated shared tables are documented value defs (e.g.
           `(def op (doc \"opcode bytes\") (record …))`).")
  (input (do (def answer (doc "the answer") 42) (def (main) answer) (export main)))
  (output (: 42 Int64)))

(case
  "a function definition may carry a leading doc, ignored for the computation"
  (doc
    "The function-def face of the leading-doc affordance (companion to the value-def case above): a
           `(doc …)` right after the signature documents `dbl` and is not part of its body, so `dbl(3)` =
           6. Pins the symmetry a definition form requires — the doc affordance cannot depend on the def
           kind.")
  (input (do (def (dbl x) (doc "doubles x") (* x 2)) (def (main) (dbl 3)) (export main)))
  (output (: 6 Int64)))

(case
  "a documented value definition binding a record is projected by a sibling"
  (doc
    "The compiler-table idiom the leading-doc affordance is load-bearing for: a documented value def
           binds a record (`(def op (doc \"opcode bytes\") (record …))`), projected by a sibling — the doc
           is stripped so `(. op sub)` reads the real record field, 2.")
  (input
    (do
      (def op (doc "opcode bytes") #record((= add 1) (= sub 2)))
      (def (main) op.sub)
      (export main)))
  (output (: 2 Int64)))

(case
  "a type declaration may carry a leading doc, ignored for the variants"
  (doc
    "A `(doc …)` right after the type NAME documents the type and is NOT a variant — the type
           analogue of a def's leading doc (a `///` doc-comment on a `type`). The doc is skipped, so the
           variant set is `Red Green Blue` and `Color.Green` matches its arm — 1. A reader that mis-reads
           the doc AS a variant rejects the type (a spurious duplicate/extra variant).")
  (input
    (do
      (type Color (doc "an RGB channel tag") Red Green Blue)
      (def (main) (match Color.Green ((Color.Red) 0) ((Color.Green) 1) ((Color.Blue) 2)))
      (export main)))
  (output (: 1 Int64)))

(case
  "a documented payload sum keeps its variant discriminants"
  (doc
    "A PAYLOAD-carrying documented sum: the leading `(doc …)` does not shift the variant
           discriminants, so `(Box.Mk 7)` matches `((Box.Mk n) n)` binding n = 7. Pins that the doc-skip is
           positional-safe for payload variants, not only nullary ones.")
  (input
    (do
      (type Box (doc "a one-field wrapper") (Mk Int64))
      (def (main) (match (Box.Mk 7) ((Box.Mk n) n)))
      (export main)))
  (output (: 7 Int64)))

(case
  "an in-definition doc clause is valid and the definition computes"
  (doc
    "The CANONICAL placement of a `(doc …)`: INSIDE the definition, right after the signature — the
           shape a `///` doc-comment renders to. `(def (main) (doc \"the main fn\") 42)` documents `main`
           and computes 42, the doc ignored for the value.")
  (input (do (def (main) (doc "the main fn") 42) (export main)))
  (output (: 42 Int64)))

(case
  "a doc wrapping a definition from outside names the in-definition placement"
  (doc
    "Unlike a `//` COMMENT (peeled as a wrapper), a `(doc …)` documents a definition from INSIDE it,
           not as a top-level wrapper. A user who WRAPS a def in `(doc \"…\" (def …))` (a natural guess
           from the comment behavior) used to get a generic unbound-name `doc` plus a misleading
           export-names-no-definition cascade; the diagnosis now names the real placement instead.")
  (input (do (doc "the main fn" (def (main) 1)) (export main)))
  (error CDZ0201 (message "documents a definition from INSIDE it")))

(case
  "a line comment wrapping a top-level form does not hide it"
  (doc
    "A leading `//` line comment on a top-level form reifies (by the reader) to `(comment \"<text>\"
           <form>)` wrapping the WHOLE form — the comment companion of the leading `(doc …)` above. The
           comment is SEMANTICALLY INERT (self-hosting-surface.md §the tree carries comments and
           documentation — the compiler sees through comments as it sees through docs), so the compiler must
           peel it to the wrapped form. `(comment \"note\" (def (f (: x Int64)) x))` defines `f`, and
           `(f 7)` = 7. A compiler that peels a leading `(doc …)` but NOT a `(comment …)` reads `comment` as
           an unknown top-level declaration head → the wrapped `def` is invisible ('unbound name comment' +
           `f` unbound). Load-bearing for a Cadenza-authored compiler whose own sources carry ordinary
           top-level `//` comments.")
  (input
    (do
      ; note
      (def (f (: x Int64)) x)
      (def (main) (f 7))
      (export main)))
  (output (: 7 Int64)))

(case
  "stacked line comments on a top-level form are all seen through"
  (doc
    "Stacked `//` lines on one form NEST — `// a` then `// b` above `def f` is `(comment \"a\"
           (comment \"b\" (def …)))` — so the compiler must peel to the INNERMOST form, not just one layer.
           `f` still defines and `(f 7)` = 7. Pins that the comment peel follows the whole nested chain, the
           multi-line-comment shape a real source file's header block takes.")
  (input
    (do
      ; a
      ; b
      (def (f (: x Int64)) x)
      (def (main) (f 7))
      (export main)))
  (output (: 7 Int64)))

(case
  "a line comment wrapping a type declaration is seen through"
  (doc
    "The comment peel is not `def`-specific — it must see through a comment wrapping ANY top-level
           form. `(comment \"the color\" (type C (R) (G)))` declares the type `C`; the program then
           constructs and matches its variants → 1 for `C.R`, 2 for `C.G`, selected by a runtime Bool. Pins
           that a leading `//` on a `type` declaration does not hide it (the type-decl companion of the
           def case above), so a commented type in a compiler's IR-sum module stays visible.")
  (input
    (do
      ; the color
      (type C (R) (G))
      (def (main (: b Bool)) (match (if b (C.R) (C.G)) ((C.R) 1) ((C.G) 2)))
      (export main)))
  (call main (: true Bool))
  (output (: 1 Int64))
  (call main (: false Bool))
  (output (: 2 Int64)))

(case
  "a line comment wrapping the entry point is seen through"
  (doc
    "The comment peel reaches the ENTRY too: `(comment \"run it\" (def (main …) …))` wraps the exported
           entry, which must still be found and run. `dbl` is defined plainly; the commented `main` doubles
           its argument → 10 for 5. Pins that a `//` on the entry point does not hide it from the export
           scan (a commented `main`/entry is the natural top of a source file), the entry companion of the
           def and type cases.")
  (input
    (do
      (def (dbl (: x Int64)) (+ x x))
      ; run it
      (def (main (: x Int64)) (dbl x))
      (export main)))
  (call main (: 5 Int64))
  (output (: 10 Int64)))

(case
  "a module function calls a sibling export by name"
  (doc
    "Witnesses core-semantics.md #A Module Binds Its Name In Its Enclosing Scope (2nd sentence:
           module bindings resolve under the same lexical scope rules as any other binding) together
           with #A Module Evaluates To A Record Of Its Exports: a module's exported definitions are in
           scope in each other's bodies, exactly as top-level definitions are mutually visible. `f`
           calls its sibling `dbl` by name; f(3) = dbl(3) + 1 = 7. Intra-module references are the norm
           — a prelude or a group of compiler passes is a module whose functions call one another.")
  (input
    (do
      (module lib
        (def (dbl x) (* x 2))

        (def (f x) (+ (dbl x) 1)))
      (lib.f 3)))
  (output (: 7 Int64)))

(case
  "a module function is recursive"
  (doc
    "Witnesses core-semantics.md #A Module Evaluates To A Record Of Its Exports with a
           self-reference: an exported function is in scope in its own body, so it may recurse.
           `fac` calls itself; fac(5) = 120. A recursive export resolves by the same lexical scope a
           top-level recursive def does AND lowers the same way — the member is registered as a standalone
           emittable function, so the self-call is a runtime `Core::Call`, not an unbounded inline. A
           compiler that resolves the recursion but cannot emit a non-top-level recursive callee declines
           (a Todo); one that models it runs `fac` to 120.")
  (input
    (do
      (module lib
        (def (fac n) (if (= n 0) 1 (* n (fac (- n 1))))))
      (lib.fac 5)))
  (output (: 120 Int64)))

(case
  "a module export is a CLOSURE FACTORY capturing a private sibling"
  (doc
    "The first-class-value face of a module member: exported `mk` returns `(fn (v) (+ (secret v)
           k))` — a closure that captures BOTH the caller's runtime `k` AND the module-PRIVATE `secret`.
           The closure escapes the module (applied in `main`), and its body still reaches the private
           helper — privacy is an IMPORT restriction, not a runtime barrier (the legitimate-escape case
           above pins a private VALUE escaping; this pins private CODE riding a closure out). (((. m mk)
           2) 4) = secret(4) + 2 = 42.")
  (input
    (do
      (module m
        (def (secret (: x Int64)) (* x 10))

        (def (mk (: k Int64)) (fn ((: v Int64)) (+ (secret v) k)))

        (export mk))
      (def (main (: k Int64)) ((m.mk k) 4))
      (export main)))
  (call main (: 2 Int64))
  (output (: 42 Int64)))

(case
  "a module-exported closure factory builds a PERFORMING closure homed at the apply site"
  (doc
    "The effects composition of the closure factory: `mk`'s closure body PERFORMS `Ctr.next` —
           declared in the importer's scope, handled at the importer's apply site. Applied TWICE under
           the handler, each application is a fresh perform against the current state (100+5, then
           100+6 → 211). Composes three pinned facts across the module boundary: factory capture,
           apply-site homing (the closure crosses the module boundary carrying an unhomed perform), and
           per-application state threading. A homing analysis keyed to the closure's DEFINITION module
           would reject or misroute the perform.")
  (input
    (do
      (effect Ctr (op next (-> Unit Int64)))
      (module m
        (def (mk (: k Int64)) (fn ((: u Unit)) (+ k (Ctr.next unit))))

        (export mk))
      (def
        (main (: n Int64))
        (handle
          Ctr
          n
          ((next (u) s (resume s (+ s 1))))
          (let ((f (m.mk 100))) (+ (f unit) (f unit)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 211 Int64)))

(case
  "two module exports are selected by a runtime branch and applied as values"
  (doc
    "Module members as branch-selected function values: `((if b (. ops inc) (. ops dbl)) x)` — the
           member projections are first-class arrow values, the `if` joins them, and the application
           dispatches to whichever the runtime Bool picked (6 / 10 at x=5). The module-member twin of the
           named-def branch selection in 09-functions; the projection must yield an applyable value in
           value position, not only in call-head position.")
  (input
    (do
      (module ops
        (def (inc (: x Int64)) (+ x 1))

        (def (dbl (: x Int64)) (* x 2))

        (export inc)

        (export dbl))
      (def (main (: b Bool) (: x Int64)) ((if b ops.inc ops.dbl) x))
      (export main)))
  (call main (: true Bool) (: 5 Int64))
  (output (: 6 Int64))
  (call main (: false Bool) (: 5 Int64))
  (output (: 10 Int64)))

(case
  "a module export rides an OUTER combinator's fn parameter a runtime number of times"
  (doc
    "A module member handed to a combinator defined OUTSIDE the module: `(times (. m step) n 1)` —
           the projection crosses the module boundary as a fn value and is applied per recursive step of
           the outer `times` (n=5 doublings → 32). Composes the member-projection-as-value with the
           iterate-combinator pin (09-functions): the indirect call inside `times` must dispatch to the
           module member exactly as to a top-level def.")
  (input
    (do
      (module m
        (def (step (: x Int64)) (* x 2))

        (export step))
      (def
        (times (: f (-> Int64 Int64)) (: n Int64) (: x Int64))
        (if (< n 1) x (times f (- n 1) (f x))))
      (def (main (: n Int64)) (times m.step n 1))
      (export main)))
  (call main (: 5 Int64))
  (output (: 32 Int64))
  (live-objects known-leak))

(case
  "two module functions are mutually recursive"
  (doc
    "Mutual recursion between two module members: `ev` calls `od`, `od` calls `ev` — neither reaches
           a normal form by inlining, so BOTH lower to standalone runtime functions calling each other
           (core-semantics.md #A Module Evaluates To A Record Of Its Exports: the members are mutually
           visible, so each names the other, and each is emittable). ev(10) is true → 1. Pins that the
           member-registration reaches an EACH-OTHER call group, not only a single self-recursive member.")
  (input
    (do
      (module m
        (def (ev n) (if (= n 0) true (od (- n 1))))

        (def (od n) (if (= n 0) false (ev (- n 1)))))
      (if (m.ev 10) 1 0)))
  (output (: 1 Int64)))

(case
  "a recursive function in a nested module runs through the projection chain"
  (doc
    "A recursive function in a NESTED module is reached AND lowered through the member-access chain:
           `(. (. outer inner) fac)` projects the inner module's `fac`, whose self-call lowers to a runtime
           `Core::Call` to the same registered member (its `Member`-headed call site reduces to the field
           lambda's body, the def identity the recursion emits against). fac(5) = 120. Composes the
           module-in-module nesting with the recursive-member lowering.")
  (input
    (do
      (module outer
        (module inner
          (def (fac n) (if (= n 0) 1 (* n (fac (- n 1)))))))
      (outer.inner.fac 5)))
  (output (: 120 Int64)))

; ── MODULE-IN-MODULE (nested modules) ──────────────────────────────────────────────────────────────────
; A module MAY contain another module as a member. Because a module IS a record of its exports
; (core-semantics.md #A Module Evaluates To A Record Of Its Exports) and a nested module is itself a
; definition grouped under a name, the inner module registers as a FIELD of the outer's record whose
; VALUE is the inner module's own record — a nested record. So an outer/inner export is reached by a
; MEMBER-ACCESS CHAIN `(. (. outer inner) v)`, exactly two ordinary projections, with nothing privileged
; by name (the same one accessor a flat export uses, applied twice). Nesting is arbitrary-depth. A
; compiler that registers only `(def …)` members drops a nested module: `(. outer inner)` then names a
; field the outer record does not carry and TRAPS at run time on the emitted component — a decline-don't-
; miscompile violation, so a generation that does not model nested modules DECLINES rather than emitting a
; component whose projection traps.
(case
  "a module nested in a module projects as a nested record field"
  (doc
    "Witnesses core-semantics.md #A Module Evaluates To A Record Of Its Exports for a NESTED module:
           `(module inner (def v 7))` written as a member of `(module outer …)` registers `inner` as a
           field of the outer's record whose value is the inner module's OWN record, so `(. (. outer inner)
           v)` is two ordinary member projections (core-semantics.md #Member Access Projects A Record Field)
           and yields 7 — the nested-record analogue of a flat export, nothing privileged by name. A compiler
           that registers only `(def …)` members drops the nested module; `(. outer inner)` then names a
           missing field and TRAPS on the emitted component — a decline-don't-miscompile violation, so a
           generation that does not model nested modules declines rather than emitting a trapping projection.")
  (input
    (do
      (module outer
        (module inner
          (def v 7)))
      outer.inner.v))
  (output (: 7 Int64)))

(case
  "a module may nest three deep"
  (doc
    "Nesting is arbitrary-depth: `(module a (module b (module c (def v 42))))` reaches `v` through
           three member accesses `(. (. (. a b) c) v)`. Pins that a nested module is itself a record whose
           fields may be records recursively — no depth privilege, the same `synth_by_occ`-embed at each
           level (`modules::synthesize` builds inner-first so each enclosing module embeds an already-built
           record).")
  (input
    (do
      (module a
        (module b
          (module c
            (def v 42))))
      a.b.c.v))
  (output (: 42 Int64)))

(case
  "a nested module's function export is applied through the projection chain"
  (doc
    "A nested module's FUNCTION export is reached AND applied through the member-access chain: the
           inner field value is the same `(fn (params) body)` lambda a flat export carries, so `((. (. outer
           inner) f) 21)` β-reduces by the ordinary application path — f(21) = 42. Pins that the nested
           record's fields carry lambdas identically to a top-level module's, not only bare values.")
  (input
    (do
      (module outer
        (module inner
          (def (f x) (* x 2))))
      (outer.inner.f 21)))
  (output (: 42 Int64)))

; The direct `(. m secret)` private-reject and the nested EXPORTED-projection cases are pinned above. These
; pin the encapsulation boundary at their intersection: a private member of a NESTED module must NOT be
; reachable THROUGH the projection chain (each hop is an ordinary closed-record projection, so a member
; absent from the inner export record is CDZ0201 just as at the top level), while the nested EXPORTED member
; IS reachable. And, dually, encapsulation withholds a NAME, not a VALUE: a private helper's value escapes
; legitimately when an EXPORTED member returns/calls it.
(case
  "a private member of a nested module is not reachable through the projection chain"
  (doc
    "The nested-projection private-reject: `inner` exports only `pub`, so `secret` is absent from
           inner's export record. `(. (. outer inner) secret)` projects a field the inner record does not
           carry — the closed-record CDZ0201, exactly as a top-level `(. m secret)` is. Pins that the
           member-access chain does not privilege access: each hop is an ordinary projection, so nesting does
           not leak a private member (the negative companion of the nested EXPORTED-projection cases).")
  (input
    (do
      (module outer
        (module inner
          (def (secret x) (+ x 1))

          (def (pub x) (secret x))

          (export pub))

        (export inner))
      (def (main) (outer.inner.secret 5))
      (export main)))
  (error CDZ0201))

(case
  "a nested module's exported member IS reachable through the chain despite a private sibling"
  (doc
    "The visible companion: the SAME nested `inner` — a private `secret` and an exported `pub` that
           calls it — has `pub` reachable through the chain: `((. (. outer inner) pub) 5)` = secret(5) = 6.
           Pins that hiding `secret` from the inner record does not withhold the named `pub` export nor break
           `pub`'s internal call to its private sibling, through the nesting.")
  (input
    (do
      (module outer
        (module inner
          (def (secret x) (+ x 1))

          (def (pub x) (secret x))

          (export pub))

        (export inner))
      (def (main) (outer.inner.pub 5))
      (export main)))
  (output (: 6 Int64)))

(case
  "a private helper's value escapes legitimately when an exported member calls it"
  (doc
    "Encapsulation withholds a NAME, not a VALUE: `secret` is private (so `(. m secret)` is CDZ0201),
           but the exported `pub` calls `secret` in its body, so `secret`'s COMPUTED RESULT flows out through
           the public API — `((. m pub) 5)` = secret(5) = 105. Pins that the visibility filter hides the
           private field from the record without severing the value's legitimate escape via an export — the
           value-vs-name distinction of #Visibility Is Explicit.")
  (input
    (do
      (module m
        (def (secret x) (+ x 100))

        (def (pub x) (secret x))

        (export pub))
      (def (main) (m.pub 5))
      (export main)))
  (output (: 105 Int64)))

(case
  "two adjacent modules declared inside a function body compose"
  (doc
    "A function body is a `(do …)` sequence that may hold BODY-LOCAL module declarations: `main`'s
           body declares `Inc` then `Scale` then uses both — `Scale.g(Inc.f(4))` = 50. Pins that two
           ADJACENT nested modules in a def body both register (each its own nested record) and the trailing
           expression reaches them. The point beyond the value: the ML surface round-trip. The ML printer
           emits a non-final declaration-keyword statement PARENTHESIZED — `(module Inc { … }); (module Scale
           { … }); …` — because the reader's `;`-sequence otherwise BREAKS before a bare `module` keyword
           (treating it as the next top-level form), truncating the body after the first module. The parens
           make each a bracketed expression the reader collects into the body, so the printer emits ML the
           reader reads back to this same tree (the roundtrip harness exercises exactly that path). Without
           the wrapping the printer produced ML it then rejected — a printer/reader round-trip failure.")
  (input
    (do
      (def
        (main)
        (do
          (module Inc
            (def (f x) (+ x 1)))
          (module Scale
            (def (g x) (* x 10)))
          (Scale.g (Inc.f 4))))
      (export main)))
  (call main)
  (output (: 50 Int64)))

(case
  "a module member body resolves a sibling module in the enclosing scope"
  (doc
    "A module member's body resolves names by ordinary lexical scope INCLUDING the module's enclosing
           scope — so a member of `app` may call a SIBLING MODULE `lib` declared beside it. `(module app
           (def (go) ((. lib answer) unit)))` reaches `(. lib answer)` = 42 because `app`'s synthesized
           record scope chains up to the enclosing do-block where `lib` binds. A scope walk that dead-ended
           at `app`'s own record would spuriously reject `lib` (CDZ0101).")
  (input
    (do
      (module lib
        (def (answer) 42))
      (module app
        (def (go) (lib.answer unit)))
      (app.go unit)))
  (output (: 42 Int64)))

(case
  "an outer definition references a sibling nested module by bare name"
  (doc
    "A module's members are mutually visible (core-semantics.md #A Module Evaluates To A Record Of
           Its Exports), and a nested module is a member — so an outer `(def …)` may reference the sibling
           nested module by BARE name. `f`'s body reads `(. inner dbl)`, resolving `inner` to the inner
           module's record via the same in-module sibling scope a bare def reference uses; f(21) = dbl(21) =
           42. Pins that the nested module participates in in-module scope as a member, not only as a
           qualified projection target.")
  (input
    (do
      (module outer
        (module inner
          (def (dbl x) (* x 2)))

        (def (f y) (inner.dbl y)))
      (outer.f 21)))
  (output (: 42 Int64)))

(case
  "a nested module and a sibling def in the same outer module both stay reachable"
  (doc
    "A nested module and an ordinary `(def …)` coexist as members of one outer module — the outer's
           record carries a field per member, module or def alike, and neither displaces the other. `(. (.
           outer inner) v)` projects the nested module's export (5) and `(. outer w)` the sibling def (9);
           their sum is 14. Pins that nesting a module does not shadow or drop a sibling value def.")
  (input
    (do
      (module outer
        (module inner
          (def v 5))

        (def w 9))
      (+ outer.inner.v outer.w)))
  (output (: 14 Int64)))

(case
  "a nested module's inner function calls its inner sibling by bare name"
  (doc
    "In-module sibling scope holds INSIDE a nested module too: the inner module's `g` references its
           inner sibling `dbl` by bare name (the same Case-R sibling resolution the outer level uses), and
           `g` is reached through the projection chain. g(20) = dbl(20) + 1 = 41. Pins that mutual member
           visibility is per-module at every nesting depth, not only at the outermost level.")
  (input
    (do
      (module outer
        (module inner
          (def (dbl x) (* x 2))

          (def (g x) (+ (dbl x) 1))))
      (outer.inner.g 20)))
  (output (: 41 Int64)))

(case
  "a module member reading a sibling constant with an unannotated param compiles and runs"
  (doc
    "The Circle variant of the module-qualified-call shape (companion to the Temp c-to-f case): a
           module member `area` reads a sibling CONSTANT `pi` and multiplies an UNANNOTATED param `r`,
           reached by the qualified call `((. Circle area) 10)`. The unannotated param is grounded by the
           body (`pi * (r * r)` is Int64) with the export clause not blocking registration. area(10) = 3 *
           (10 * 10) = 300.")
  (input
    (do
      (module Circle
        (def pi 3)

        (def (area r) (* pi (* r r)))

        (export area))
      (Circle.area 10)))
  (output (: 300 Int64)))

(case
  "a module's delegated capability is reachable as metadata, not as an export"
  (doc
    "Witnesses core-semantics.md #A Module Carries Its Manifest And Entry As Metadata:
           the capabilities are reached by the (meta …) key, distinct from the export
           namespace, so they never collide with an export. The module declares the routing-agnostic
           effect `log` and its entry `main` DELEGATES it to the host with `(host (log) …)`; the manifest
           is the union of the entry's delegations, so the capabilities metadata contains \"log\" (the
           delegation — not the declaration — is the grant, capabilities-and-effects.md #The Program
           Manifest Is The Union Of Its Entrypoints' Delegations).")
  (input
    (do
      (module m
        (effect log (op emit (-> String Unit)))

        (def (main) (host (log) (log.emit "hi"))))
      (= (. m (meta capabilities)) #list("log"))))
  (output (: true Bool)))

(case
  "a delegated capability is not itself an export field"
  (doc
    "Witnesses core-semantics.md #A Module Carries Its Manifest And Entry As Metadata (1st
           sentence): a delegated capability is carried as metadata SEPARATE from the exported fields,
           so it is not itself an export. The module's entry delegates `log` to the host but the module
           exports only `main`; a module IS a record of its exports, and `log` is not among them, so
           projecting it is a COMPILE-TIME type error (CDZ0201) — naming a field the record does not
           contain (core-semantics.md #Member Access Projects A Record Field), rejected before lowering
           rather than deferred to a runtime trap. The capability lives in `(meta capabilities)`
           (witnessed by the case above), not among the export fields, so `log` resolves to no export.")
  (input
    (do
      (module m
        (effect log (op emit (-> String Unit)))

        (def (main) (host (log) (log.emit "hi"))))
      m.log))
  (error CDZ0201))

; ============================================================================================
; Module pragmas — the compiler-directive channel; unknown key REJECTED, not ignored
; ============================================================================================
; A module MAY carry directives written `(pragma <key> <arg>…)` that instruct the compiler how to
; compile it (modules-and-namespaces.md §Module Directives; options/module-pragmas/). The load-bearing
; rule — unlike C's advisory #pragma — is that an UNRECOGNIZED key is REJECTED at compile time (CDZ0601),
; never ignored: a meaning-changing directive that some toolchain silently dropped would let one source
; compile to two meanings, the drift the one-executable-semantics / canonical-form principles forbid
; (constitution §IX, §X). A recognized key with the wrong argument shape is CDZ0602. The pinned registry
; today defines one key, `default-integer` (its behavior witnessed by the numeric cases in
; 06-numeric-model.sexp); these cases pin the general mechanism. The pragma
; channel is realized by a later generation, so the seed declines these — they pin the contract.
(case
  "an unrecognized pragma key is rejected rather than ignored"
  (doc
    "`(pragma frobnicate 3)` names a key the pinned registry does not define, so the module is
           REJECTED (CDZ0601, modules-and-namespaces.md #An Unrecognized Module Directive Is Rejected),
           not silently ignored. THE reason the channel is strict: a dropped meaning-changing directive
           would make one source mean two things on two toolchains. The general-mechanism companion of
           the numeric `default-integer` cases.")
  (input
    (do
      (module m
        (pragma frobnicate 3)

        (def (answer) 42))
      (m.answer unit)))
  (error CDZ0601))

(case
  "a recognized pragma with a malformed argument list is rejected"
  (doc
    "`(pragma default-integer)` names a registered key but omits its one required argument, so it
           is rejected against the shape the key defines (CDZ0602, modules-and-namespaces.md #A Module
           Directive Is Drawn From A Fixed Set, 2nd sentence). Distinct from CDZ0601 (unknown key) and
           from CDZ0303 (a well-formed directive whose type argument fails the integer-domain predicate):
           here the directive is structurally malformed.")
  (input
    (do
      (module m
        (pragma default-integer)

        (def (answer) 42))
      (m.answer unit)))
  (error CDZ0602))

(case
  "the default-fraction pragma with its type argument omitted is malformed (CDZ0602)"
  (doc
    "The fraction twin of the default-integer arity check: `(pragma default-fraction)` names a
           registered key but omits its one required type argument, so it is the structural CDZ0602
           (malformed args) — distinct from the numeric-domain CDZ0303 a well-formed-but-wrong-type
           argument would get. (migrated from rcdzc a_default_fraction_pragma_with_wrong_arity_is_cdz0602.)")
  (input
    (do
      (module m
        (pragma default-fraction)

        (def (x) 5))
      (m.x unit)))
  (error CDZ0602))

; The `overflow` key's shape contract is RICHER than the single-type-arg keys above: each argument is a
; nested `(signed <mode>)` / `(unsigned <mode>)` sub-form whose mode is drawn from the fixed set {trap, wrap}.
; A well-formed pragma (either or both signednesses) is ACCEPTED and does not block the module's registration
; — the member `(. m f)` resolves and runs; an unspecified signedness simply falls through to the default.
; A mode OUTSIDE {trap, wrap} (`(signed nonesuch)`), or NO signedness sub-form at all (`(pragma overflow)`),
; is the structural CDZ0602 — the same malformed-shape reject as the arity checks above, applied to this
; key's own shape. (Migrated from rcdzc an_overflow_pragma_validates_its_shape_and_does_not_block_registration;
; the overflow pragma's runtime WRAP/TRAP behavior is witnessed by the `(pragma overflow …)` cases in
; 06-numeric-model.sexp — here we pin only its shape contract.)
(case
  "a well-formed overflow pragma is accepted and does not block module registration"
  (input
    (do
      (module m
        (pragma overflow (signed wrap) (unsigned trap))

        (def (f) 1))
      (m.f unit)))
  (output (: 1 Int64)))

(case
  "an overflow pragma with an unknown mode is a malformed directive (CDZ0602)"
  (input
    (do
      (module m
        (pragma overflow (signed nonesuch))

        (def (f) 1))
      (m.f unit)))
  (error CDZ0602))

(case
  "an overflow pragma with no signedness sub-form is malformed (CDZ0602)"
  (input
    (do
      (module m
        (pragma overflow)

        (def (f) 1))
      (m.f unit)))
  (error CDZ0602))

; The REMOVED contract-identity directives `contract`/`input`/`output` (a contract's identity is now derived
; from its evaluated `descriptor`, not dedicated directives — the D3 pragma deprecation, #4542) are no longer
; in PRAGMA_REGISTRY, so each is now an UNKNOWN module directive: rejected CDZ0601 naming it as not-a-directive,
; exactly as any invented key is. Pins the removal — a future re-add of any of these keys to the registry flips
; these cases, forcing a deliberate decision. Migrated from rcdzc
; contract_input_output_pragmas_are_removed_and_now_reject_as_unknown.
(case
  "the removed `contract` module directive is now an unknown-directive reject"
  (input
    (do
      (module m
        (pragma contract "x")

        (def (answer) 42))
      (m.answer unit)))
  (error CDZ0601 (message "`contract` is not a module directive")))

(case
  "the removed `input` module directive is now an unknown-directive reject"
  (input
    (do
      (module m
        (pragma input "x")

        (def (answer) 42))
      (m.answer unit)))
  (error CDZ0601 (message "`input` is not a module directive")))

(case
  "the removed `output` module directive is now an unknown-directive reject"
  (input
    (do
      (module m
        (pragma output "x")

        (def (answer) 42))
      (m.answer unit)))
  (error CDZ0601 (message "`output` is not a module directive")))

(case
  "an export and a like-named metadata key do not collide"
  (doc
    "Witnesses core-semantics.md #A Module Carries Its Manifest And Entry As Metadata (2nd
           sentence): metadata is reached by a key distinct from every export name, so metadata access
           cannot collide with an export. This module's entry delegates `log` to the host AND the module
           exports a definition literally named `capabilities`. The export `(. m capabilities)` resolves
           to that definition (applied, it yields 7), while `(. m (meta capabilities))` resolves to the
           manifest — the same spelling in the two channels denotes two different things, which is the
           whole reason metadata lives behind (meta …).")
  (input
    (do
      (module m
        (effect log (op emit (-> String Unit)))

        (def (capabilities) 7)

        (def (main) (host (log) (log.emit "hi"))))
      (if (= (m.capabilities unit) 7) (= (. m (meta capabilities)) #list("log")) false)))
  (output (: true Bool)))

; ── MULTI-FILE PACKAGE composition (modules-and-namespaces.md; DESIGN-package-linking.md) ──────────────
; Each case below carries one or more `(module "name" <prog>)` LIBRARY files; the `(input …)` is the
; ENTRY (named `main`). A library's public surface is its `(export …)` list; the entry (or another
; library) reaches it only through an explicit `(import "name" (names…))`.
(case
  "an imported name resolves to a sibling file's exported definition"
  (doc
    "Witnesses modules-and-namespaces.md #Imports Are Explicit: a name defined in another module
           is brought into scope by an explicit import, and a call to it resolves across the file
           boundary into one linked component. `lib` exports `helper` (→ 40); `main` imports and calls
           it, adding 2.")
  (module "lib"
    (do (def (helper) 40) (export helper)))
  (input (do (import "lib" (helper)) (def (main) (+ (helper) 2)) (export main)))
  (output (: 42 Int64)))

; WHOLE-MODULE ALIAS import `(import path alias)` (bare-name spec, positionally distinct from the named-list
; form): binds the whole module under `alias`, reached by qualified projection `(. alias member)`. This is
; the collision-free path when two modules export a UNIFORMLY-NAMED member (`descriptor`) that the flat
; named-list form would bind twice into one scope (v-platform-itest multi-contract dispatch). Scope: DEFS
; project today; qualified TYPES/CTORS via `(. alias T)` are a separate, larger gap.
(case
  "two whole-module alias imports project a uniformly-named export with no collision"
  (doc
    "Modules `aaa` and `bbb` both export a def `descriptor`; the flat named-list import of both would
           bind `descriptor` twice (a colliding-import CDZ0201). The whole-module alias form binds each
           module under its own local handle `a`/`b`, and `(. a descriptor)` / `(. b descriptor)` project
           each module export: 10 + 20 = 30. Pins that a uniform export name is reachable from 2+ modules
           without collision.")
  (module "aaa"
    (do (def (descriptor) 10) (export descriptor)))
  (module "bbb"
    (do (def (descriptor) 20) (export descriptor)))
  (input
    (do (import "aaa" a) (import "bbb" b) (def (main) (+ a.descriptor b.descriptor)) (export main)))
  (output (: 30 Int64)))

(case
  "a whole-module alias projects an exported FUNCTION and applies it"
  (doc
    "The alias handle projects a function export too: `aaa` exports `descriptor`, a one-parameter
           function; `((. a descriptor) 41)` = 42. Qualified projection reaches the def and applies it as
           any member access does.")
  (module "aaa"
    (do (def (descriptor (: x Int64)) (+ x 1)) (export descriptor)))
  (input (do (import "aaa" a) (def (main) (a.descriptor 41)) (export main)))
  (output (: 42 Int64)))

(case
  "an unimported sibling definition is not in scope"
  (doc
    "Witnesses modules-and-namespaces.md #Imports Are Explicit (2nd sentence: an import introduces
           no names beyond those it names) + #Visibility Is Explicit: WITHOUT an `(import …)`, a sibling
           file's exported name is invisible — referencing it is an unbound-name rejection (CDZ0101),
           not an implicit cross-file resolution.")
  (module "lib"
    (do (def (helper) 40) (export helper)))
  (input (do (def (main) (+ (helper) 2)) (export main)))
  (error CDZ0101))

(case
  "importing a name a module does not export is rejected"
  (doc
    "Witnesses modules-and-namespaces.md #Visibility Is Explicit (2nd sentence: a definition not
           made visible is not importable): `lib` defines `helper` and exports only `other`, so
           importing `helper` is rejected — visibility is the export list, not mere definition.")
  (module "lib"
    (do (def (helper) 40) (def (other) 1) (export other)))
  (input (do (import "lib" (helper)) (def (main) (helper)) (export main)))
  (error CDZ0201))

(case
  "two definitions imported under the same name are rejected"
  (doc
    "Witnesses modules-and-namespaces.md #Colliding Imported Names Are Rejected: importing two
           definitions under the same local name into one scope is a compile-time error (CDZ0201),
           never resolved by an implicit precedence.")
  (module "a"
    (do (def (x) 1) (export x)))
  (module "b"
    (do (def (x) 2) (export x)))
  (input (do (import "a" (x)) (import "b" (x)) (def (main) (x)) (export main)))
  (error CDZ0201))

(case
  "a cycle of module imports is rejected"
  (doc
    "Witnesses modules-and-namespaces.md #Cyclic Module Dependencies Are Rejected: a set of
           modules whose import relationships form a cycle is rejected at compile time (CDZ0201). Here
           the entry imports `lib`, and `lib` imports back from the entry — a dependency loop.")
  (module "lib"
    (do (import "main" (seed)) (def (helper) (seed)) (export helper)))
  (input
    (do (import "lib" (helper)) (def (seed) 1) (def (main) (helper)) (export main) (export seed)))
  (error CDZ0201))

(case
  "an imported helper reaches its own file's private definition when inlined"
  (doc
    "Witnesses that linking preserves each file's scope through monomorphization: `lib` exports
           `pub-helper`, whose body calls a PRIVATE sibling `priv-helper` (defined in `lib`, not
           exported, not imported by the entry). When `pub-helper` inlines into `main`, its body's
           reference to `priv-helper` still resolves in `lib`'s scope — cross-file β-copy hygiene.")
  (module "lib"
    (do (def (priv-helper) 40) (def (pub-helper) (+ (priv-helper) 1)) (export pub-helper)))
  (input (do (import "lib" (pub-helper)) (def (main) (+ (pub-helper) 1)) (export main)))
  (output (: 42 Int64)))

; --- A sum value crosses a module boundary ---------------------------------------------------------
; core-semantics.md #Sum Types Are Structural Types + modules-and-namespaces.md #Imports Are Explicit:
; a sum is an ordinary value, so an exported function may RETURN one and the importing entry matches it
; exactly as a local sum value. These pin that the sum construct/match machinery composes with linking —
; a variant value built in one file dispatches correctly in another after the exported producer inlines.
(case
  "a prelude Option value crosses a module boundary as an export result"
  (doc
    "`lib` exports `parse` returning an `Option Int64` (`(Some b)` for a positive input); the entry
           imports it and matches the result. The Option value built in `lib` carries its variant tag
           across the link so the entry's `(Some n)` arm binds n = 5. Pins that a prelude sum is an
           ordinary cross-module value — its construction in one file and its match in another compose
           through linking, no special handling for a sum at the boundary.")
  (module "lib"
    (do (def (parse (: b Int64)) (if (> b 0) (Some b) (None))) (export parse)))
  (input
    (do (import "lib" (parse)) (def (main) (match (parse 5) ((Some n) n) ((None) 0))) (export main)))
  (output (: 5 Int64)))

(case
  "a recursive user sum built in a lib is folded by the entry over the imported type"
  (doc
    "`lib` declares a cons-list sum `L`, exports it CONCRETELY with the wildcard `(. L *)` (the
           handle + ALL constructors) plus `mk` building `(Cons 5 (Cons 6 Nil))`; the entry `(import
           \"lib\" (L mk))` brings the type + its constructors into scope and folds the imported value
           with `sm`, MATCHING on `L.Nil`/`L.Cons`. The recursive sum's spine built in one file is walked
           variant-by-variant in another, summing to 11. A user sum's identity is its declaration
           (type-system.md #Nominal Is An Orthogonal Modifier Over Any Structural Type — identity is the
           fully-qualified name, so re-declaring a same-shape `L` in the entry yields a DISTINCT type a
           value of the lib's `L` does not satisfy); the composing form is to IMPORT the one type both
           files share, exactly as `#a sum TYPE and its constructors are imported by a wildcard` does for
           a flat sum. Because the entry MATCHES `L`'s variants, `lib` exports them (`L.*`) — a bare
           `(export L)` would export the HANDLE ONLY (abstract), and the entry's match would be CDZ0214.
           Pins that a RECURSIVE user sum composes across the module boundary through a concrete import —
           its heap spine and variant discriminants are read by the entry's match over the SAME nominal
           type the lib built.")
  (module "lib"
    (do (type L (Nil) (Cons Int64 L)) (def (mk) (L.Cons 5 (L.Cons 6 (L.Nil)))) (export L.* mk)))
  (input
    (do
      (import "lib" (L mk))
      (def (sm (: l L)) (match l ((L.Nil) 0) ((L.Cons h t) (+ h (sm t)))))
      (def (main) (sm (mk)))
      (export main)))
  (output (: 11 Int64))
  (live-objects 0))

(case
  "a GENERIC user sum crosses a module boundary at a concrete instantiation"
  (doc
    "The generic companion of the recursive-sum crossing above: `lib` declares a GENERIC `(type Box
           (W a) (E))` and exports `mk` building `(Box.W 42)` at `a = Int64`; the entry declares its own
           structurally-identical `Box` and matches the imported value, binding the payload at Int64 → 42.
           Pins that a generic user sum composes across the module boundary at a concrete instantiation —
           the crossing value carries its variant + payload exactly as a monomorphic one does, and the two
           modules each declaring `Box` is NOT a duplicate (each module has its own type namespace; the
           duplicate-declaration check is per-module). Both `Box` declarations are user types of the same
           structural shape, so the imported `(Box.W 42)` matches the entry's `(Box.W n)` arm.")
  (module "lib"
    (do (type Box (W a) (E)) (def (mk) (Box.W 42)) (export mk)))
  (input
    (do
      (import "lib" (mk))
      (type Box (W a) (E))
      (def (main) (match (mk) ((Box.W n) n) ((Box.E) 0)))
      (export main)))
  (output (: 42 Int64)))

(case
  "a module-PRIVATE heap Map is read through an exported accessor across repeated calls"
  (doc
    "A module-level HEAP def (the member pins above cover scalars/closures/effects): private
           tbl is a built Map, readable ONLY through the exported accessor, called repeatedly from
           the importer — the module-level heap value must initialize once (or rebuild consistently)
           per call, and its handle survives the module boundary with no owner in the importer's
           frame. Hit ×2 + miss faces.")
  (input
    (do
      (import "table" (get))
      (def (main (: k Int64)) (+ (* (get 1) 10) (+ (get k) 1)))
      (export main)))
  (module "table"
    (do
      (def tbl #map((= 1 10) (= 2 20)))
      (def (get (: k Int64)) (match (Map.lookup tbl k) ((Some v) v) ((None _u) 0)))
      (export get)))
  (call main (: 2 Int64))
  (output (: 121 Int64))
  (call main (: 9 Int64))
  (output (: 101 Int64)))

(case
  "an exported UNANNOTATED helper instantiates at TWO types from the IMPORTER's call sites"
  (doc
    "The generic-VALUE crossing above lands one instantiation chosen by the LIB; here the
           exported unannotated helper's TWO instantiations come from the IMPORTER's call sites
           ((List Int64) + (List String)) — specialization must resolve against calls the defining
           module never sees. Runtime n rides the int path, byte-len reads the string path.")
  (input
    (do
      (import "lib" (first))
      (def
        (main (: n Int64))
        (do
          (def a (first #list(n 6)))
          (def s (first #list("ab" "c")))
          (+ (* a 10) (String.byte-len s))))
      (export main)))
  (module "lib"
    (do (def (first xs) (match xs (#list(h (.. _t)) h) (_ (trap "empty")))) (export first)))
  (call main (: 5 Int64))
  (output (: 52 Int64))
  (call main (: 0 Int64))
  (output (: 2 Int64)))

(case
  "an exported OPEN-ROW projector instantiates at importer widths the module never saw"
  (doc
    "The row companion of the generic-instantiation pin above: get-x instantiated by the
           IMPORTER at a 3-field record AND a 1-field record — x sits at different physical offsets
           under the sorted erasure, resolved per call site the defining module never saw.")
  (input
    (do
      (import "lib" (get-x))
      (def
        (main (: n Int64))
        (+ (* (get-x #record((= x 5) (= y 6) (= z n))) 10) (get-x #record((= x 3)))))
      (export main)))
  (module "lib"
    (do (def (get-x r) r.x) (export get-x)))
  (call main (: 7 Int64))
  (output (: 53 Int64))
  (call main (: 0 Int64))
  (output (: 53 Int64)))

(case
  "a module-private Symbol-keyed table resolves the IMPORTER's own symbol literals by content"
  (doc
    "Symbol content-identity ACROSS the module boundary: the module's literals key a private
           map; the importer probes with ITS OWN literals — reader-interned literals of equal
           content are ONE value at the champ hash (a per-module intern table leaking identity
           would miss). The compiler symbol-table idiom cross-module. Miss face -1.")
  (input
    (do
      (import "ops" (op-code))
      (def (main (: mode Int64)) (op-code (if (= mode 1) #"add" (if (= mode 2) #"mul" #"div"))))
      (export main)))
  (module "ops"
    (do
      (def tbl (Map.insert (Map.insert Map.empty #"add" 1) #"mul" 2))
      (def (op-code (: s Symbol)) (match (Map.lookup tbl s) ((Some v) v) ((None _u) -1)))
      (export op-code)))
  (call main (: 1 Int64))
  (output (: 1 Int64))
  (call main (: 2 Int64))
  (output (: 2 Int64))
  (call main (: 3 Int64))
  (output (: -1 Int64)))

(case
  "a sum TYPE and its constructors are imported by a wildcard and constructed in the entry"
  (doc
    "Beyond exporting a sum VALUE (the cases above, where the entry RE-DECLARES a structurally-
           identical type), here `lib` EXPORTS the nominal sum TYPE `Color` CONCRETELY with the wildcard
           `(. Color *)` — the handle + every constructor — plus a consumer `to-int`, and the entry
           `(import \"lib\" (Color to-int))` brings the TYPE + its constructors into scope and CONSTRUCTS
           `(Color.Green)` locally. The imported type's identity crosses the link, so a value the entry
           builds with the imported constructor dispatches correctly in the lib's `to-int` match →
           `Green` = 2. The wildcard `Color.*` is what makes the constructors importable: a bare
           `(export Color)` exports the HANDLE ONLY (abstract — the entry could name `Color` and hold its
           values but not construct one; a `(Color.Green)` there would be CDZ0214). Pins that a nominal
           sum type + all its constructors compose across an explicit import via the wildcard (not only
           sum VALUES with a re-declared type) — the value built against the imported type is the SAME
           nominal type the lib's consumer expects.")
  (module "lib"
    (do
      (type Color (Red) (Green) (Blue))
      (def (to-int (: c Color)) (match c ((Color.Red) 1) ((Color.Green) 2) ((Color.Blue) 3)))
      (export Color.*)
      (export to-int)))
  (input (do (import "lib" (Color to-int)) (def (main) (to-int (Color.Green))) (export main)))
  (output (: 2 Int64)))

(case
  "a wildcard-exported variant whose name shadows a prelude type is constructible in an importer"
  (doc
    "The prelude-collision case of the wildcard import above: `lib` declares `(type T (Foo Int64)
           (List (List T)))` — a variant NAMED `List`, which also names a prelude type — and exports it
           concretely with `(. T *)`. The entry imports `T` and constructs `(T.List (list))`, which `sz`'s
           `((T.List es) 9)` arm yields 9 for. The constructor selector `T.List` must resolve through `T`'s
           OWN imported constructor set, NOT as the free prelude name `List`: a member-access tail on a
           known nominal type is a constructor selector, not a shadowable name. This was wrongly rejected
           CDZ0214 ('`T`'s constructor `List` is withheld') when the importer resolved the tail to the
           prelude `List` — not List-specific (a `Bool` variant collided identically) and not construct-only
           (a match arm failed too), while the SAME construction in the declaring file worked. The
           non-colliding-names control (`NInt`/`NList`) always ran to 9, pinning the oracle. Pins that an
           imported type's constructor is reachable through its wildcard export even when its name shadows a
           prelude type — the shape the compiler port's `Ast` (with `List`/`Bool` variants) needs across
           files.")
  (module "lib"
    (do
      (type T (Foo Int64) (List (List T)))
      (def (sz (: n T)) (match n ((T.Foo _) 1) ((T.List es) 9)))
      (export T.*)
      (export sz)))
  (input (do (import "lib" (T sz)) (def (main) (sz (T.List #list()))) (export main)))
  (output (: 9 Int64)))

; --- ABSTRACT (opaque) types: export the type HANDLE, keep the CONSTRUCTOR private -----------------
; A type declaration's handle and its constructors are INDEPENDENTLY exportable (modules-and-namespaces.md
; §Visibility Is Explicit — the one per-name export surface, applied to a type's handle vs its variants).
; Exporting the handle bare `(export Color)` publishes an ABSTRACT type: an importer may NAME `Color`
; (annotate, hold, pass its values) and use the module's exported functions over it, but MUST NOT
; construct or match its variants — that is the module's private business. This is the abstract-data-type
; / smart-constructor discipline: an invariant a type carries is established once, in the module's private
; constructor, and no importer can forge a value that skips it. Exporting the constructors too — the
; wildcard `(. Color *)` (all) or a specific `(. Color Green)` — makes the type CONCRETE (the cases above).
(case
  "an abstract type's constructor is not reachable outside its module"
  (doc
    "`lib` exports the type HANDLE `Color` (bare `(export Color)`) and a smart constructor `mk`, but
           NOT `Color`'s variant constructors. The entry imports `(Color mk)` and tries to CONSTRUCT
           `(Color.Green)` directly — reaching a constructor the module kept private. That is rejected
           CDZ0214: `Color`'s handle is visible here (the entry may name the type and hold its values) but
           its constructor `Green` is withheld, so a `Color` value is built only through `mk`. Pins that a
           bare type-handle export is ABSTRACT — the constructor is hidden on purpose, distinct from a
           plain unbound name (the type IS in scope). The fix is to call the module's exported `mk`, or for
           the module to export `Color.*`.")
  (module "lib"
    (do (type Color (Red) (Green) (Blue)) (def (mk) Color.Green) (export Color) (export mk)))
  (input (do (import "lib" (Color mk)) (def (main) (Color.Green)) (export main)))
  (error CDZ0214))

; --- a withheld constructor is unreachable in PATTERN position too (not only construction) --------------
; The abstract-type guarantee must also gate MATCHING: pattern-matching an abstract value through its
; withheld variant reads the module's PRIVATE payload. The QUALIFIED `((Temp.T v))` always rejected (the
; `(. T A)` selector's withheld poison); the BARE `((T v))` PUNNED past the gate + read the private payload
; — a one-token bypass of ADT opacity — until the bare pattern head got the SAME withheld-ctor gate. Both
; are CDZ0214, and it reaches a GUARD-nested inner match too (shared lowering). (migrated from rcdzc
; a_bare_ctor_pattern_over_an_abstract_type_is_rejected_cdz0214_like_the_qualified_spelling.)
(case
  "a bare withheld-constructor pattern over an abstract value is rejected CDZ0214"
  (module "lib"
    (do (type Temp (T Int64)) (def (mk (: c Int64)) (Temp.T (* c 10))) (export Temp) (export mk)))
  (input
    (do
      (import "lib" (Temp mk))
      (def (main (: k Int64)) (match (mk k) ((T v) v) (_ -1)))
      (export main)))
  (error CDZ0214))

(case
  "a qualified withheld-constructor pattern over an abstract value is rejected CDZ0214"
  (module "lib"
    (do (type Temp (T Int64)) (def (mk (: c Int64)) (Temp.T (* c 10))) (export Temp) (export mk)))
  (input
    (do
      (import "lib" (Temp mk))
      (def (main (: k Int64)) (match (mk k) ((Temp.T v) v) (_ -1)))
      (export main)))
  (error CDZ0214))

(case
  "a guard-nested withheld-constructor pattern over an abstract value is rejected CDZ0214"
  (module "lib"
    (do (type Temp (T Int64)) (def (mk (: c Int64)) (Temp.T (* c 10))) (export Temp) (export mk)))
  (input
    (do
      (import "lib" (Temp mk))
      (def
        (main (: k Int64))
        (match (mk k) ((guard w (match w ((T v) (> v 20)) (_ false))) 1) (_ -1)))
      (export main)))
  (error CDZ0214))

; --- eval/quote CANNOT forge a private constructor: expansion is checked as if written directly ---------
; The abstract-type guarantee above (a withheld constructor is unreachable outside its module — CDZ0214)
; MUST hold whether the constructor reference arrives via DIRECT source OR via `(eval (quote …))`. eval's
; desugar reconstructs the source its `Ast` argument denotes and re-resolves it AT THE EVAL CALL SITE — eval
; gets NO privileged scope (`metaprogramming.md` §Expansion Precedes And Feeds The Core Guarantees: expanded
; AST is capability/visibility-checked "exactly as if it had been written directly"). So `(eval (quote
; (Color.Green)))` from a file where `Color`'s handle is visible but `Green` is withheld rejects CDZ0214,
; exactly as the direct `(Color.Green)` above does — it does NOT forge a `Color` value. This closes a
; soundness hole (found 2026-07-16): eval-reconstructed nodes are appended outside every file's demux
; range, so the file-identity-keyed visibility gate saw no file and did not fire; the fix walks the
; reconstructed node's parent chain to the enclosing eval call's file (`Db::visibility_file_of`), so the
; existing gate fires unchanged. This is the make-or-break for any opaque-type trust boundary (an
; LCF-kernel `Thm`, a capability token): a value of an abstract type cannot be forged from outside via eval.
(case
  "eval of a quoted private constructor is withheld exactly as a direct reference — no forge"
  (doc
    "SOUNDNESS: `(eval (quote (Color.Green)))` from the entry — where `Color`'s handle is visible but
           its `Green` constructor is withheld — rejects CDZ0214, NOT a forged `Color`. eval re-resolves the
           reconstructed `(Color.Green)` at the eval call site under the SAME constructor-visibility gate as
           the direct `(Color.Green)` case above (eval gets no privileged scope). Pins that eval/quote cannot
           reach a module-private constructor — the abstract-type forge hole is closed.")
  (module "lib"
    (do (type Color (Red) (Green) (Blue)) (def (mk) Color.Green) (export Color) (export mk)))
  (input (do (import "lib" (Color mk)) (def (main) (eval (quote (Color.Green)))) (export main)))
  (error CDZ0214))

(case
  "eval of an exported smart constructor still works — the forge fix does not over-reject"
  (doc
    "The companion that guards against over-rejecting: eval of the EXPORTED `mk` (a public door) must
           still fold. `(rank (eval (quote (mk))))` obtains a `Color` through the exported `mk` and inspects
           it through the exported `rank` → `Green` = 2. Pins that the forge fix withholds only the
           PRIVATE constructor path, not a legitimate eval of an exported function.")
  (module "lib"
    (do
      (type Color (Red) (Green) (Blue))
      (def (mk) Color.Green)
      (def (rank c) (match c ((Color.Red) 1) ((Color.Green) 2) ((Color.Blue) 3)))
      (export Color)
      (export mk)
      (export rank)))
  (input (do (import "lib" (Color mk rank)) (def (main) (rank (eval (quote (mk))))) (export main)))
  (output (: 2 Int64)))

; The forge fix is COMPLETE across two further axes, pinned so a regression can't reopen a sibling hole:
; (a) a private ctor buried DEEP in an eval-reconstructed compound is gated too (the parent-walk reaches the
; consumer file however deep the ctor ref sits), and (b) the DESTRUCTURE side — matching a private ctor from
; outside — is withheld (unforgeability is both "can't build" AND "can't take apart" a value except through
; the module's doors). Both were verified sound when the eval-forge fix landed (`Db::visibility_file_of`).
(case
  "eval does not forge a private constructor NESTED in a compound"
  (doc
    "The deep-nesting companion of the eval-forge pin: `(eval (quote (id (Color.Green))))` — the
           private `Color.Green` sits INSIDE a call to the exported `id`, not at the top of the
           reconstructed form. It still rejects CDZ0214: `Db::visibility_file_of`'s parent-walk reaches the
           consumer file however deeply the reconstructed ctor reference is nested, so the withheld-ctor gate
           fires on it exactly as at the top level. Pins that the fix is not depth-limited (a shallow guard
           would forge here).")
  (module "lib"
    (do
      (type Color (Red) (Green) (Blue))
      (def (mk) Color.Green)
      (def (id c) c)
      (export Color)
      (export mk)
      (export id)))
  (input
    (do (import "lib" (Color mk id)) (def (main) (eval (quote (id (Color.Green))))) (export main)))
  (error CDZ0214))

(case
  "a private constructor cannot be MATCHED from outside its module either"
  (doc
    "The destructure side of unforgeability: `(match (mk) ((Color.Green) 1) (_ 0))` names the private
           `Color.Green` in a PATTERN from outside `lib` — withheld CDZ0214, exactly as constructing it is. A
           value of an abstract type is neither built NOR taken apart through a private constructor outside
           the module (both directions gated by the same visibility check) — obtained + inspected only
           through the exported doors (`mk`/`rank`). Pins that opacity guards match, not just construction.")
  (module "lib"
    (do (type Color (Red) (Green) (Blue)) (def (mk) Color.Green) (export Color) (export mk)))
  (input
    (do (import "lib" (Color mk)) (def (main) (match (mk) ((Color.Green) 1) (_ 0))) (export main)))
  (error CDZ0214))

; The eval-forge fix also respects the PARTIAL point on the abstract↔concrete axis: a module may export
; SOME constructors (`(export Color.Green)`) but not others. Through eval, the eval-reconstructed ctor ref
; resolves per-constructor visibility at the consumer file (via `Db::visibility_file_of` + the qualified
; ctor gate), so the EXPORTED ctor works and a WITHHELD sibling is still CDZ0214 — the same partial surface
; hand-written code sees. Pins that the fix is not a coarse abstract-vs-concrete flag but the exact
; per-constructor gate, reached through eval.
(case
  "eval of a partially-exported type's WITHHELD constructor is rejected"
  (doc
    "`lib` exports only `Color.Green` (not `Red`/`Blue`). `(eval (quote (Color.Red)))` from the entry
           names a WITHHELD sibling constructor — rejected CDZ0214, exactly as a hand-written `(Color.Red)`
           would be. Pins that through eval the per-constructor visibility gate still distinguishes the
           exported ctor from the withheld ones (the fix is precise, not a blanket abstract flag).")
  (module "lib"
    (do (type Color (Red) (Green) (Blue)) (export Color.Green)))
  (input (do (import "lib" (Color)) (def (main) (eval (quote (Color.Red)))) (export main)))
  (error CDZ0214))

(case
  "eval of a partially-exported type's EXPORTED constructor works"
  (doc
    "The companion: the SAME `lib` exporting only `Color.Green` — `(eval (quote (Color.Green)))` names
           the EXPORTED ctor, so it folds to a `Color` value the match reads → 1 (no over-reject). Pins that
           the partial-export gate lets the exported ctor through via eval, exactly as hand-written code.")
  (module "lib"
    (do (type Color (Red) (Green) (Blue)) (export Color.Green)))
  (input
    (do
      (import "lib" (Color))
      (def (main) (match (eval (quote (Color.Green))) ((Color.Green) 1) (_ 0)))
      (export main)))
  (output (: 1 Int64)))

(case
  "an abstract type is used through the module's exported constructor and accessor"
  (doc
    "The companion of the reject above: the SAME abstract `lib` (handle `Color` + `mk` + `rank`, no
           constructor exported) used CORRECTLY. The entry never names a `Color` constructor — it obtains a
           value through the exported smart constructor `mk` and inspects it through the exported `rank`,
           the only doors the module opened. `(rank (mk))` → `Green` = 2. Pins that an abstract type is
           fully usable through its module's exported functions while its representation stays private —
           the value crosses the link as its underlying structural value (the nominal tag is compile-time
           only), so opacity costs nothing at runtime.")
  (module "lib"
    (do
      (type Color (Red) (Green) (Blue))
      (def (mk) Color.Green)
      (def (rank (: c Color)) (match c ((Color.Red) 1) ((Color.Green) 2) ((Color.Blue) 3)))
      (export Color)
      (export mk)
      (export rank)))
  (input (do (import "lib" (Color mk rank)) (def (main) (rank (mk))) (export main)))
  (output (: 2 Int64)))

(case
  "a specific constructor export exposes one variant and keeps the rest private"
  (doc
    "Between fully-abstract and fully-concrete: `lib` exports the handle `Color` plus ONE constructor
           `(. Color Green)`, keeping `Red`/`Blue` private. The entry may construct `(Color.Green)` (the
           exported constructor) — `rank` reads it → 2 — but constructing `(Color.Red)` would be CDZ0214.
           Pins that constructor visibility is per-constructor, not all-or-nothing: `(export (. Color G))`
           publishes exactly the named constructor, the partial point on the abstract↔concrete axis.")
  (module "lib"
    (do
      (type Color (Red) (Green) (Blue))
      (def (rank (: c Color)) (match c ((Color.Red) 1) ((Color.Green) 2) ((Color.Blue) 3)))
      (export Color.Green)
      (export rank)))
  (input (do (import "lib" (Color rank)) (def (main) (rank (Color.Green))) (export main)))
  (output (: 2 Int64)))

(case
  "a built-in comparison on an abstract type's value is rejected outside its module"
  (doc
    "`lib` exports the HANDLE `Color` (abstract) + a smart constructor `mk`. The entry may name
           `Color` and obtain values via `mk`, but comparing two of them with the built-in `=` observes
           the equality of `Color`'s PRIVATE representation, which the handle-only export withheld — so it
           is rejected CDZ0202 (the nominal-boundary code). A built-in structural comparison is not one of
           the operations a handle-only export publishes; a module that wants its abstract type compared
           exports a comparison FUNCTION (`(def (eq (: x Color) (: y Color)) …)`), the ML discipline —
           the representation stays hidden and only the module's published operations are available.
           Within the declaring module (or a concrete `Color.*` importer) `=` on `Color` is unaffected.")
  (module "lib"
    (do (type Color (Red) (Green) (Blue)) (def (mk) Color.Green) (export Color) (export mk)))
  (input (do (import "lib" (Color mk)) (def (main) (= (mk) (mk))) (export main)))
  (error CDZ0202))

(case
  "an abstract-typed value used as a CHAMP map key is rejected (opacity — the key path invokes the forbidden structural comparison)"
  (doc
    "The INDIRECT route to the same observation the direct-`=` case above rejects: a CHAMP `Map`/`Set`
           keyed by an abstract-typed value invokes a built-in STRUCTURAL comparison on that key at
           insert/lookup (champ_eq / value-eq over the key spine), which observes the abstract type's private
           representation through equality — exactly what type-system.md #An Abstract Type's Representation Is
           Not Observable Across Its Boundary forbids (a MUST). So `(Map.insert Map.empty (mk k) 42)` outside
           the declaring module is rejected CDZ0202 like `(= (mk k) (mk k))`, because the rule is about the
           OBSERVATION, not the surface syntax — routing the comparison through a map key does not escape it
           (concierge ruling, ask-17967). Values stay legal to HOLD (as a key's paired value, a payload, …);
           only the key-EQUALITY-observation rejects. A module that wants its abstract type used as a key
           publishes a comparison operation, the ML discipline.")
  (module "temp"
    (do (type Temp (T Int64)) (def (mk (: c Int64)) (T (* c 10))) (export Temp) (export mk)))
  (input
    (do
      (import "temp" (Temp mk))
      (def
        (main (: k Int64))
        (match (Map.lookup (Map.insert Map.empty (mk k) 42) (mk k)) ((Some v) v) ((None _u) -1)))
      (export main)))
  (call main (: 5 Int64))
  (error CDZ0202))

(case
  "a Map keyed by a COMPOUND containing an abstract type is rejected (opacity — the key path compares the abstract leaf)"
  (doc
    "The compound extension of the bare-abstract-key case above: a Map/Set key need not BE the
           abstract value directly — a COMPOUND that CONTAINS one, `(tuple (mk k) 1)` with an abstract
           `Temp` leaf, still has its key spine compared by champ_eq/value-eq at insert/lookup, which walks
           into the `Temp` leaf and observes its private representation through equality. So it is rejected
           CDZ0202 like the bare key (v-inference f23646b30), because the opacity MUST is about the
           observation reaching the abstract leaf, not the surface key shape. A key with no abstract leaf
           is unaffected.")
  (module "temp"
    (do (type Temp (T Int64)) (def (mk (: c Int64)) (T (* c 10))) (export Temp) (export mk)))
  (input
    (do
      (import "temp" (Temp mk))
      (def
        (main (: k Int64))
        (match
          (Map.lookup (Map.insert Map.empty #tuple((mk k) 1) 42) #tuple((mk k) 1))
          ((Some v) v)
          ((None _u) -1)))
      (export main)))
  (call main (: 5 Int64))
  (error CDZ0202))

(case
  "a built-in comparison on a COMPOUND containing an abstract type is rejected (opacity walks the compound)"
  (doc
    "The compound extension of the bare direct-`=` reject: `(= (tuple (mk k) 1) (tuple (mk k) 1))`
           with an abstract `Temp` leaf — the built-in structural comparison walks INTO the compound and
           observes the `Temp` representation through equality, so it is rejected CDZ0202 like the bare
           `(= (mk k) (mk k))` (v-inference 2f2be099c). The observation reaching the abstract leaf is what the
           opacity MUST forbid, regardless of the surrounding tuple.")
  (module "temp"
    (do (type Temp (T Int64)) (def (mk (: c Int64)) (T (* c 10))) (export Temp) (export mk)))
  (input
    (do
      (import "temp" (Temp mk))
      (def (main (: k Int64)) (if (= #tuple((mk k) 1) #tuple((mk k) 1)) 1 0))
      (export main)))
  (call main (: 5 Int64))
  (error CDZ0202))

(case
  "a built-in comparison on a LIST of an abstract type is rejected (opacity walks the list spine)"
  (doc
    "The list arm of the compound direct-`=` reject: `(= #list((mk k)) #list((mk k)))` with an abstract
           `Temp` element — the built-in structural comparison walks the list spine and observes the `Temp`
           representation through equality, so it rejects CDZ0202 like the tuple form above. The opacity walk
           (`key_ty_contains_abstract_at`) covers a `Ty::List` element arm, not just tuple.")
  (module "temp"
    (do (type Temp (T Int64)) (def (mk (: c Int64)) (T (* c 10))) (export Temp) (export mk)))
  (input
    (do
      (import "temp" (Temp mk))
      (def (main (: k Int64)) (if (= #list((mk k)) #list((mk k))) 1 0))
      (export main)))
  (call main (: 5 Int64))
  (error CDZ0202))

(case
  "a built-in comparison on two MAPS with abstract VALUES is rejected (direct-`=` walks the value spine)"
  (doc
    "The value-spine complement of the map-KEY reject and of the hold-legal case below: HOLDING an
           abstract value as a map value under a concrete key is legal (insert/lookup compares only the key,
           never the value spine), but a direct `(= m1 m2)` on two whole maps walks BOTH keys AND values via
           champ_eq, observing the abstract VALUE's private representation — so it rejects CDZ0202.
           `key_ty_contains_abstract_at` walks the `Ty::Map` value arm and the direct-`=` site passes the
           whole operand type. Pins that HOLDING is legal but COMPARING the whole map is not.")
  (module "temp"
    (do (type Temp (T Int64)) (def (mk (: c Int64)) (T (* c 10))) (export Temp) (export mk)))
  (input
    (do
      (import "temp" (Temp mk))
      (def
        (main (: k Int64))
        (if (= (Map.insert Map.empty k (mk k)) (Map.insert Map.empty k (mk k))) 1 0))
      (export main)))
  (call main (: 5 Int64))
  (error CDZ0202))

(case
  "a built-in comparison on two RECORDS with an abstract FIELD is rejected (opacity walks the record spine)"
  (doc
    "The record arm of the compound direct-`=` reject: `(= #record((= t (mk k))) …)` with an abstract
           `Temp` field — built-in record comparison walks the field spine and observes the `Temp`
           representation through equality, so it rejects CDZ0202 like the tuple/list forms.
           `key_ty_contains_abstract_at` recurses into `Ty::Record` fields.")
  (module "temp"
    (do (type Temp (T Int64)) (def (mk (: c Int64)) (T (* c 10))) (export Temp) (export mk)))
  (input
    (do
      (import "temp" (Temp mk))
      (def (main (: k Int64)) (if (= #record((= t (mk k))) #record((= t (mk k)))) 1 0))
      (export main)))
  (call main (: 5 Int64))
  (error CDZ0202))

(case
  "a built-in comparison on a concrete-only compound stays legal (the opacity recursion finds no abstract leaf)"
  (doc
    "The negative control of the compound direct-`=` sweep: with the SAME abstract `temp` module in
           scope, comparing two CONCRETE-only tuples `(= #tuple(k 1) #tuple(k 1))` stays legal — the opacity
           recursion walks the compound, finds no abstract leaf, and does not over-reject. Guards the CDZ0202
           compound-walk from creeping onto concrete-only compounds. Returns 1 (the tuples are equal).")
  (module "temp"
    (do (type Temp (T Int64)) (def (mk (: c Int64)) (T (* c 10))) (export Temp) (export mk)))
  (input
    (do
      (import "temp" (Temp mk))
      (def (main (: k Int64)) (if (= #tuple(k 1) #tuple(k 1)) 1 0))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64)))

(case
  "a built-in comparison on a concrete-only RECORD stays legal (no over-reject through the record recursion)"
  (doc
    "The record arm of the concrete-only control: `(= #record((= t k)) #record((= t k)))` with a
           concrete `Int64` field stays legal — the record recursion finds no abstract leaf. Returns 1.")
  (module "temp"
    (do (type Temp (T Int64)) (def (mk (: c Int64)) (T (* c 10))) (export Temp) (export mk)))
  (input
    (do
      (import "temp" (Temp mk))
      (def (main (: k Int64)) (if (= #record((= t k)) #record((= t k))) 1 0))
      (export main)))
  (call main (: 5 Int64))
  (output (: 1 Int64)))

(case
  "a Set/Map lookup over an abstract-element collection reached via a param is rejected (read-side opacity)"
  (doc
    "The read-side completion of the opacity sweep: the collection need not be LOCALLY constructed —
           a `(Set Temp)` reached through a fn PARAM (or import), then `Set.contains`/`Map.lookup`/`Set.union`
           etc., still compares its abstract elements by the built-in structural comparison at membership/
           lookup time, so it rejects CDZ0202 (v-inference 23fb89ea4). `(def (has (: s (Set Temp)) (: x Temp))
           (Set.contains s x))` — the membership probe observes the `Temp` representation. Completes the
           abstract-observation sweep across all four routes: construction (key), compound-containing key,
           direct-`=`, and read-side lookup.")
  (module "temp"
    (do (type Temp (T Int64)) (def (mk (: c Int64)) (T (* c 10))) (export Temp) (export mk)))
  (input
    (do
      (import "temp" (Temp mk))
      (def (has (: s (Set Temp)) (: x Temp)) (Set.contains s x))
      (def (main (: k Int64)) (if (has (Set.empty) (mk k)) 1 0))
      (export main)))
  (call main (: 5 Int64))
  (error CDZ0202))

(case
  "a Set.union over abstract-element Set params is rejected (set-algebra compares stored elements)"
  (doc
    "The set-ALGEBRA arm of the read-side opacity: `Set.union` over two `(Set Temp)` params compares
           their stored abstract elements by the built-in structural comparison to dedup the union, observing
           `Temp`'s private representation — so it rejects CDZ0202 like the `Set.contains` membership probe
           above (the read-side case's doc lists `Set.union` among the routes; this pins it directly). No
           local construction — the sets arrive as params.")
  (module "temp"
    (do (type Temp (T Int64)) (def (mk (: c Int64)) (T (* c 10))) (export Temp) (export mk)))
  (input
    (do
      (import "temp" (Temp mk))
      (def (u (: a (Set Temp)) (: b (Set Temp))) (Set.len (Set.union a b)))
      (def (main) (u #set() #set()))
      (export main)))
  (call main)
  (error CDZ0202))

(case
  "a Set.contains over a concrete Int64-keyed Set param stays legal (the read-side gate does not over-reject a concrete element)"
  (doc
    "The negative control of the read-side opacity: a `(Set Int64)` reached through a param, then
           `Set.contains`, stays legal — the element type is concrete, so membership comparison observes no
           private representation. Guards the CDZ0202 read-side gate from over-rejecting a concrete-keyed
           collection. `Set.contains` over an empty set is false → 0.")
  (input
    (do
      (def (has (: s (Set Int64)) (: x Int64)) (Set.contains s x))
      (def (main (: k Int64)) (if (has #set() k) 1 0))
      (export main)))
  (call main (: 5 Int64))
  (output (: 0 Int64)))

(case
  "a Map keyed by a DEEPLY-nested compound containing an abstract type is rejected (opacity recurses to any depth)"
  (doc
    "The depth guard for the compound-key reject: the recursion `key_ty_contains_abstract_at`
           (v-inference f23646b30) walks a compound key to ANY depth, not just one structural level — a
           `(Tuple (Tuple Temp Int64) Int64)` with the abstract `Temp` leaf TWO tuples deep still has its
           whole key spine compared by champ_eq/value-eq at insert/lookup, reaching the `Temp` leaf and
           observing its private representation. So it rejects CDZ0202 exactly as the one-level tuple key
           does. Pins that the opacity check does not stop at the outermost compound — a future edit that
           made the walk shallow (only the top-level elems) would flip this to a SILENT compile+observe of
           the abstract leaf, a soundness hole this case would catch.")
  (module "temp"
    (do (type Temp (T Int64)) (def (mk (: c Int64)) (T (* c 10))) (export Temp) (export mk)))
  (input
    (do
      (import "temp" (Temp mk))
      (def
        (main (: k Int64))
        (match
          (Map.lookup
            (Map.insert Map.empty #tuple(#tuple((mk k) 1) 2) 9)
            #tuple(#tuple((mk k) 1) 2))
          ((Some v) v)
          ((None _u) -1)))
      (export main)))
  (call main (: 5 Int64))
  (error CDZ0202))

(case
  "an abstract-typed value held as a Map VALUE under a concrete key is legal (value-holding never triggers the opacity reject)"
  (doc
    "The NEGATIVE boundary of the abstract-opacity sweep: opacity rejects only the KEY-comparison
           observation, NOT holding an abstract value. A Map keyed by a CONCRETE `Int64` whose VALUE is an
           abstract `Color` obtained through the module's smart constructor `mk` is LEGAL — the value spine
           is never compared (only the concrete key is), so the private representation is never observed;
           the held value is read back through the exported accessor `rank` → 2. Pins the exact scope of
           the CDZ0202 abstract-key/element check: an over-reach that rejected an abstract type in ANY
           collection position (value, payload, paired value — not just a comparable key/element) would
           break this legitimate hold-don't-compare use, so this positive case guards the reject from
           creeping into value positions. The complement of the CHAMP-key reject above.")
  (module "lib"
    (do
      (type Color (Red) (Green) (Blue))
      (def (mk) Color.Green)
      (def (rank (: c Color)) (match c ((Color.Red) 1) ((Color.Green) 2) ((Color.Blue) 3)))
      (export Color)
      (export mk)
      (export rank)))
  (input
    (do
      (import "lib" (Color mk rank))
      (def
        (main (: k Int64))
        (match (Map.lookup (Map.insert Map.empty k (mk)) k) ((Some c) (rank c)) ((None _u) -1)))
      (export main)))
  (call main (: 5 Int64))
  (output (: 2 Int64))
  (live-objects 0))

(case
  "eval of a fully-concrete imported constructor is legal from outside"
  (doc
    "`(eval (quote (P.Mk 7)))` where lib2 exports `(. P *)` — the CONCRETE complement of the
           no-forge pins above: a fully-exported ctor is reachable through eval's call-site
           visibility exactly as a direct `(P.Mk 7)` is, and the reconstructed value matches → 7.
           Pins the visibility gate's other edge (an over-reaching gate that withheld every
           eval-reconstructed ctor — not just private ones — breaks the legitimate metaprogramming
           path).")
  (module "lib2"
    (do (type P (Mk Int64)) (export P.*)))
  (input
    (do
      (import "lib2" (P))
      (def (main (: d Int64)) (match (eval (quote (P.Mk 7))) ((P.Mk v) v)))
      (export main)))
  (call main (: 0 Int64))
  (output (: 7 Int64)))

(case
  "a single-variant abstract type's constructor match outside its module is rejected CDZ0214 (withheld-ctor)"
  (doc
    "A withheld-constructor MATCH outside its module is rejected with CDZ0214 (the withheld-constructor
           code) — exactly as CONSTRUCTION is and as a MULTI-variant match is, per modules-and-namespaces.md
           §A Type's Handle And Its Constructors Are Independently Visible. `lib` exports the abstract handle
           `C` + smart ctor `mk` but NOT `C`'s variant ctor `A`; the entry matching `C.A` outside is rejected
           CDZ0214. A single-variant sum newtype-ERASES to `Ty::Nominal`, so its match reaches the nominal
           wrong-ctor check — which used to report the GENERIC CDZ0203 'not a variant of the matched type'
           instead of the actionable withheld-ctor CDZ0214 (v-verification: the exact HOL-kernel `Thm`/`Term`
           newtype shape). The nominal branch now propagates the head's coded withheld poison FIRST (the
           newtype twin of the boxed-sum path), so the message says 'this constructor is withheld, use the
           exported accessor'. Soundness was always intact (the match is rejected either way — abstract
           opacity holds); this pins the CODE. A genuine other-type ctor (no withheld poison) still gives CDZ0203.")
  (module "lib"
    (do (type C (A Int64)) (def (mk) (C.A 5)) (export C) (export mk)))
  (input (do (import "lib" (C mk)) (def (main) (match (mk) ((C.A n) n))) (export main)))
  (error CDZ0214))

; --- BARE-pattern / eval / guard-nested withheld-ctor rejects (soundness; v-inference 38c12a630) ------
(case
  "a BARE pattern on a withheld constructor is rejected like the qualified spelling (encapsulation soundness)"
  (doc
    "The BARE-pattern twin of the qualified-reject pin above: matching a withheld constructor `T`
           through the UNQUALIFIED pattern `((T v) …)` MUST reject CDZ0214 exactly as the qualified
           `((Temp.T v) …)` does. Before the fix (v-inference 38c12a630) the bare-pattern resolver looked up
           the ctor in the scrutinee TYPE's variant set WITHOUT the per-name visibility gate the qualified
           selector applies, so `(match (mk k) ((T v) v) …)` COMPILED and read the private smart-ctor payload
           (50) — an encapsulation SOUNDNESS hole the ADT/smart-constructor discipline (and the verification
           kernel's Thm/Term opacity) relies on. The fix gates the bare pattern head at the shared match
           lowering (`lower::pattern_constraints`), so it rejects like the qualified form.")
  (input
    (do
      (import "temp" (Temp mk))
      (def (main (: k Int64)) (match (mk k) ((T v) v) (_ -1)))
      (export main)))
  (module "temp"
    (do (type Temp (T Int64)) (def (mk (: c Int64)) (T (* c 10))) (export Temp) (export mk)))
  (call main (: 5 Int64))
  (error CDZ0214))

(case
  "an eval-reconstructed BARE pattern on a withheld constructor is also rejected (encapsulation soundness, metaprogramming route)"
  (doc
    "The metaprogramming route to the same hole: a bare withheld-ctor pattern reconstructed through
           `(eval (quasiquote (match … ((T v) v) …)))` MUST also reject CDZ0214 — the eval path shares the
           match lowering, so the `lower::pattern_constraints` visibility gate closes it too. Guards against
           a quasiquote splice smuggling in the bare pattern that the direct resolver now rejects.")
  (input
    (do
      (import "temp" (Temp mk))
      (def (main (: k Int64)) (eval (quasiquote (match (unquote (mk k)) ((T v) v) (_ -1)))))
      (export main)))
  (module "temp"
    (do (type Temp (T Int64)) (def (mk (: c Int64)) (T (* c 10))) (export Temp) (export mk)))
  (call main (: 5 Int64))
  (error CDZ0214))

(case
  "a guard-nested BARE pattern on a withheld constructor is also rejected (encapsulation soundness, guard-desugar route)"
  (doc
    "The guard-desugar route: a bare withheld-ctor pattern nested inside a guard condition
           `((guard w (match w ((T v) (> v 20)) …)) …)` MUST also reject CDZ0214. A guard desugars to a
           nested match through the same lowering, so the shared `lower::pattern_constraints` gate covers it
           — one resolver choke-point closes all three routes (direct, eval, guard-nested).")
  (input
    (do
      (import "temp" (Temp mk))
      (def
        (main (: k Int64))
        (match (mk k) ((guard w (match w ((T v) (> v 20)) (_ false))) 1) (_ -1)))
      (export main)))
  (module "temp"
    (do (type Temp (T Int64)) (def (mk (: c Int64)) (T (* c 10))) (export Temp) (export mk)))
  (call main (: 5 Int64))
  (error CDZ0214))

; --- TRANSITIVE re-export across a module chain (entry <- mid <- base) --------------------------------
; A module may RE-EXPORT a binding it imported from another module: `mid` imports `f` from `base` and lists
; `f` in its own export clause, so an entry importing from `mid` reaches `base`'s `f` transitively — the
; general-module analogue of the verification kernel's re-export chain (Inc-18). Encapsulation must hold
; across the chain: a base member NOT re-exported by `mid` stays unreachable from the entry (the explicit-
; visibility rule composes transitively). These are TODO on the rust backend like every multi-module case
; (a known rust gap — the wasm path pins the linking semantics).
(case
  "an effect declared in a MODULE is performed by its helper and handled by the IMPORTER"
  (doc
    "The cross-module effect lifecycle: the `logging` module DECLARES `Log`, exports it alongside a
           helper `emit-twice` whose body PERFORMS it twice; the importer installs the HANDLER around a
           call to the imported helper. The two performs cross the module boundary to discharge at the
           importer's handle — the first notes `n` (3 → 30, state 0→1), the second notes `n+1` (4 → 40)
           → 70. Pins that an effect is an exportable module member whose contract travels with the import
           (the declaration is routing-agnostic — the importer decides the handler), and that a module
           helper's performs home correctly against a handler the module never sees.")
  (module "logging"
    (do
      (effect Log (op note (-> Int64 Int64)))
      (def (emit-twice (: n Int64)) (+ (Log.note n) (Log.note (+ n 1))))
      (export Log)
      (export emit-twice)))
  (input
    (do
      (import "logging" (Log emit-twice))
      (def
        (main (: n Int64))
        (handle Log 0 ((note (v) s (resume (* v 10) (+ s 1)))) (emit-twice n)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 70 Int64)))

(case
  "an imported helper's performs discharge at a handler whose STATE is the importer's heap Map"
  (doc
    "The cross-module effect pin above threads SCALAR state; here the handle is seeded with
           the importer's MAP — heap state the defining module never sees. The imported helper
           performs twice; each arm Map.lookups the threaded state and re-threads it, so heap state
           must survive perform/resume round-trips ACROSS the module boundary. Miss face -1·10+10=0.")
  (input
    (do
      (import "probe" (Look sum2))
      (def
        (main (: b Int64))
        (do
          (def m #map((= 1 10) (= 2 20)))
          (handle
            Look
            m
            ((look (k) st (resume (match (Map.lookup st k) ((Some v) v) ((None _u) -1)) st)))
            (sum2 1 b))))
      (export main)))
  (module "probe"
    (do
      (effect Look (op look (-> Int64 Int64)))
      (def (sum2 (: a Int64) (: b Int64)) (+ (Look.look a) (* (Look.look b) 10)))
      (export Look)
      (export sum2)))
  (call main (: 2 Int64))
  (output (: 210 Int64))
  (call main (: 9 Int64))
  (output (: 0 Int64)))

(case
  "a transitive re-export reaches the base module's function through the middle module"
  (doc
    "`mid` imports `f` from `base` and re-exports it (`f` in mid's export clause); the entry imports
           `f` from `mid` and calls it. The re-exported binding resolves through the chain to base's `f`:
           `(f 4)` = 4*10 = 40. Pins that a module can re-export an imported binding and an importer reaches
           the original definition transitively (entry <- mid <- base).")
  (module "base"
    (do (def (f (: n Int64)) (* n 10)) (export f)))
  (module "mid"
    (do (import "base" (f)) (export f)))
  (input (do (import "mid" (f)) (def (main) (f 4)) (export main)))
  (output (: 40 Int64)))

(case
  "a base member not re-exported by the middle module is unreachable from the entry"
  (doc
    "Encapsulation composes across the chain: `base` exports only `pub`, keeping `secret` private, and
           `mid` re-exports only `pub`. The entry importing `secret` from `mid` names a binding neither base
           exported nor mid re-exported — rejected CDZ0201. Pins that the explicit-visibility rule holds
           transitively: a re-export cannot widen access to a member the origin kept private, and an
           importer cannot reach past mid's export clause to base's private members.")
  (module "base"
    (do (def (secret) 99) (def (pub (: n Int64)) (+ n 1)) (export pub)))
  (module "mid"
    (do (import "base" (pub)) (export pub)))
  (input (do (import "mid" (secret)) (def (main) (secret)) (export main)))
  (error CDZ0201))

(case
  "a middle module re-exports a base function and also uses it in its own exported function"
  (doc
    "`mid` both RE-EXPORTS base's `f` AND defines its own `g` that calls `f`; the entry imports both.
           `(f 2)` = 20 (reached transitively) and `(g 3)` = `(f 3)+1` = 31, summing to 51. Pins that a
           re-exported binding and a mid-defined binding that consumes it coexist — the re-export does not
           shadow or duplicate the binding mid uses internally, and both cross the boundary to the entry.")
  (module "base"
    (do (def (f (: n Int64)) (* n 10)) (export f)))
  (module "mid"
    (do (import "base" (f)) (def (g (: n Int64)) (+ (f n) 1)) (export f) (export g)))
  (input (do (import "mid" (f g)) (def (main) (+ (f 2) (g 3))) (export main)))
  (output (: 51 Int64)))

; A file-top `(pragma default-fraction …)` in the ENTRY must NOT leak across the import boundary into an
; imported module's literals. The pragma is MODULE-scoped (numeric-model.md §"…WITHIN THAT MODULE"; a file
; is a module) — it grounds the DECLARING file's unconstrained literals, not an imported file's. `lib`
; declares NO pragma and writes `(Rational.of 1 2)` (bare `1`/`2` are ordinary Int64 arguments to
; `Rational.of`, exactly as written); the entry declares `(pragma default-fraction Rational)` and imports
; `lib`. Regression guard: a bug briefly grounded the entry pragma over the WHOLE linked arena (every file's
; top-level defs), so `lib`'s `Rational.of` arguments became `Rational` and it failed CDZ0203 — the pragma
; leaked into the imported module. Here `lib.half` stays well-formed (its `1`/`2` are Int64) AND the entry's
; own `(/ 1 2)` grounds to `Rational` (the pragma DOES apply in the declaring file): the two `1/2` values
; are equal, so `main` returns 1.
(case
  "a file-top default-fraction pragma does not leak into an imported module's literals"
  (doc
    "The entry declares `(pragma default-fraction Rational)` and imports `half` from `lib`, which has
           NO pragma. `lib`'s `(Rational.of 1 2)` must keep its bare `1`/`2` as ordinary Int64 arguments —
           the entry's pragma is module-scoped and MUST NOT ground `lib`'s literals (a regression briefly
           leaked it across the import boundary, failing `lib`'s `Rational.of` CDZ0203). Meanwhile the
           entry's OWN `(/ 1 2)` DOES ground to `Rational` (the pragma applies in the declaring file), so it
           equals `lib.half` (both `1/2`); `main`'s result is `(: 1 Int64)` (explicitly annotated so the
           pragma does not ground the answer, keeping the pinned value unambiguous). Pins module-scoped
           pragma isolation across an import.")
  (module "lib"
    (do (def (half) (Rational.of 1 2)) (export half)))
  (input
    (do
      (pragma default-fraction Rational)
      (import "lib" (half))
      (def (main) (if (= (/ 1 2) (half)) (: 1 Int64) (: 0 Int64)))
      (export main)))
  (output (: 1 Int64)))

(case
  "a file-level (module-doc) header before an import is inert and round-trips"
  (doc
    "A `(module-doc \"…\")` is the file/module-level doc-comment node the ML reader emits for a `///`
           header before a NON-documentable form — the surface a file header round-trips as, rather than
           being downgraded to a `//` comment. (A `///` before a `def`/`type`/`effect`/`module` attaches
           INSIDE it as a `(doc …)`; only before a non-documentable form — here an `import` — does it become
           a top-level `(module-doc)`.) It DECLARES nothing and is inert at every stage (no def/export, no
           runtime effect), so the program compiles and runs exactly as without it: the header precedes the
           `import` of `half`, and `main` still returns 1. Pins that a top-level `(module-doc)` is TOLERATED
           (not the CDZ0201 an unmodeled top-level form gets) and inert — and, via corpus_roundtrip, that a
           `///` file header survives the ML surface round-trip (as `(module-doc)`, not a downgraded `//`).")
  (module "lib"
    (do (def (half) (: 1 Int64)) (export half)))
  (input
    (do
      (module-doc "The entry module — documents the file, defines main.")
      (import "lib" (half))
      (def (main) (half))
      (export main)))
  (output (: 1 Int64)))

(case
  "a heap Map crosses the module boundary whole and the importer's extension stays local"
  (doc
    "The accessor pin reads THROUGH the module (:1097); here the exported `base` returns the
           module's private Map ITSELF and the IMPORTER extends it — `Map.insert m 3 k` — while the
           module's own `probe` must keep seeing the UNCHANGED original (persistence across the
           component boundary): len 3 (300) + probe(3)=0 on the module side (0) + the importer's
           lookup reads its own k (307 at k=7, 300 at k=0). A boundary crossing that handed the
           importer a shared-mutable handle (or re-materialized the map per probe call from the
           extended value) flips the middle digit.")
  (input
    (do
      (import "table" (base probe))
      (def
        (main (: k Int64))
        (do
          (def m (base))
          (def m2 (Map.insert m 3 k))
          (+
            (* 100 (Map.len m2))
            (+ (* 10 (probe 3)) (match (Map.lookup m2 3) ((Some v) v) ((None _u) -1))))))
      (export main)))
  (module "table"
    (do
      (def tbl #map((= 1 10) (= 2 20)))
      (def (base) tbl)
      (def (probe (: k Int64)) (match (Map.lookup tbl k) ((Some v) v) ((None _u) 0)))
      (export base probe)))
  (call main (: 7 Int64))
  (output (: 307 Int64))
  (call main (: 0 Int64))
  (output (: 300 Int64)))

(case
  "a module factory's escaping closure carries a private HEAP value in its env"
  (doc
    "The heap-env variant of the closure-factory pins (private-FN capture and the performing
           factory are pinned above): `mk`'s returned closure captures the module-private MAP handle
           plus the caller's base — the env slot crossing the boundary holds a CHAMP handle, not
           private code or a scalar. Applied twice by the importer (f(5)=115, f(1)=111 → 1261 at k=5;
           1211 at k=0 — base rides, map read repeats). An env crossing that flattened the map to its
           looked-up scalar at factory time would still answer the happy path — the k=0 row's REPEATED
           read through the SAME env slot is what a snapshot-at-mk misses if the private table were
           later distinguishable; the pin's value is fixing the env-slot REP for heap captures across
           the component boundary.")
  (input
    (do
      (import "counter" (mk))
      (def (main (: k Int64)) (do (def f (mk 10)) (+ (* 10 (f k)) (f 1))))
      (export main)))
  (module "counter"
    (do
      (def secret #map((= 1 100)))
      (def
        (mk (: base Int64))
        (fn ((: x Int64)) (+ base (+ x (match (Map.lookup secret 1) ((Some v) v) ((None _u) 0))))))
      (export mk)))
  (call main (: 5 Int64))
  (output (: 1261 Int64))
  (call main (: 0 Int64))
  (output (: 1211 Int64)))

(case
  "a perform crosses TWO module boundaries to the entry's handler through a re-export chain"
  (doc
    "Composes the cross-module effect pin (:1495, one boundary) with the transitive re-export
           chain (:1543, plain fns): `base` declares Log and performs it in `work`; `mid` re-exports
           BOTH the effect and the helper without touching them; the ENTRY installs the handler. The
           perform homes across entry <- mid <- base — two boundaries, the middle one a pure
           pass-through that must forward the effect's identity (a mid that re-declared or re-keyed
           Log would orphan base's performs). work(k) = note(k) + note(k+1) with the arm ×10 and
           state stepping: k=3 → 30+40 = 70; k=0 → 0+10 = 10.")
  (module "base"
    (do
      (effect Log (op note (-> Int64 Int64)))
      (def (work (: n Int64)) (+ (Log.note n) (Log.note (+ n 1))))
      (export Log)
      (export work)))
  (module "mid"
    (do (import "base" (Log work)) (export Log) (export work)))
  (input
    (do
      (import "mid" (Log work))
      (def (main (: k Int64)) (handle Log 0 ((note (v) s (resume (* v 10) (+ s 1)))) (work k)))
      (export main)))
  (call main (: 3 Int64))
  (output (: 70 Int64))
  (call main (: 0 Int64))
  (output (: 10 Int64)))

(case
  "an unannotated helper re-exported through a chain instantiates at TWO types from the entry"
  (doc
    "The generic composition of the re-export chain: base's `dup` has NO annotations (`(tuple x
           x)`), mid re-exports it untouched, and the ENTRY instantiates it at Int64 AND String from
           its own call sites — specialization resolution must chase the binding through two hops and
           still specialize per ENTRY-side type (the one-boundary two-type pin is :1116-area; the
           chain adds the pass-through middle that must not pin the type). p=(k,k) summed ×100 +
           byte-len of q's \"ab\" slot: 602 at k=3, 2 at k=0. A resolution that froze dup at its
           FIRST instantiation fails the second call's type-check or answers with the wrong rep.")
  (input
    (do
      (import "mid" (dup))
      (def
        (main (: k Int64))
        (do
          (def p (dup k))
          (def q (dup "ab"))
          (+ (* 100 (+ (. p 0) (. p 1))) (String.byte-len (. q 0)))))
      (export main)))
  (module "base"
    (do (def (dup x) #tuple(x x)) (export dup)))
  (module "mid"
    (do (import "base" (dup)) (export dup)))
  (call main (: 3 Int64))
  (output (: 602 Int64))
  (call main (: 0 Int64))
  (output (: 2 Int64)))

; --- The DIAMOND dependency: type unification + effect identity through both arms. ---
(case
  "DIAMOND: two middles import one base; the entry composes both and base's abstract type unifies"
  (doc
    "The module chains above are LINEAR (entry<-mid<-base); a DIAMOND has two middles importing ONE base with the entry composing both arms: base's nominal Ctr flows through left.bump and right.dbl and must unify as the SAME type — a per-import-path fresh nominal would reject dbl's argument. read(dbl(bump(mk 5))) = 12.")
  (module "base"
    (do
      (type Ctr (Mk Int64))
      (def (mk (: n Int64)) (Ctr.Mk n))
      (def (read (: c Ctr)) (match c ((Ctr.Mk v) v)))
      (export Ctr)
      (export mk)
      (export read)))
  (module "left"
    (do (import "base" (Ctr mk read)) (def (bump (: c Ctr)) (mk (+ (read c) 1))) (export bump)))
  (module "right"
    (do (import "base" (Ctr mk read)) (def (dbl (: c Ctr)) (mk (* (read c) 2))) (export dbl)))
  (input
    (do
      (import "base" (mk read))
      (import "left" (bump))
      (import "right" (dbl))
      (def (main (: n Int64)) (read (dbl (bump (mk n)))))
      (export main)))
  (call main (: 5 Int64))
  (output (: 12 Int64)))

(case
  "DIAMOND effect: two middles each perform base's ONE effect; the entry's single handler serves both"
  (doc
    "The effect face of the diamond: base declares Tick, left and right EACH perform it, and the entry's ONE handler serves both arms with threaded state (3+130=133) — a per-path re-keyed effect identity would orphan one arm's perform; the stepping state proves both route through the same frame in order.")
  (module "base"
    (do (effect Tick (op t (-> Int64 Int64))) (export Tick)))
  (module "left"
    (do (import "base" (Tick)) (def (lwork (: n Int64)) (Tick.t n)) (export lwork)))
  (module "right"
    (do (import "base" (Tick)) (def (rwork (: n Int64)) (Tick.t (* n 10))) (export rwork)))
  (input
    (do
      (import "base" (Tick))
      (import "left" (lwork))
      (import "right" (rwork))
      (def
        (main (: k Int64))
        (handle Tick 0 ((t (v) s (resume (+ v s) (+ s 100)))) (+ (lwork k) (rwork k))))
      (export main)))
  (call main (: 3 Int64))
  (output (: 133 Int64)))

; --- Abstract-type opacity through collections and comparability; the concrete-export perimeter. ---
(case
  "abstract-type values live in an importer's map and round-trip through module ops"
  (doc
    "The abstract-data-type discipline × collections: `Temp` is exported as a BARE handle
           (constructors withheld) — the importer can hold its values (here as MAP VALUES, through
           insert/lookup/Option) and pass them back to the module's `celsius`, but never construct
           or match them (the smart-constructor invariant — mk scales ·10 — is un-forgeable). The
           opaque value crosses CHAMP storage and the Option projection with its rep intact (25 at
           k=25, 0 at k=0). A map that required matching its value type (or an import that lost the
           nominal frame) breaks the hold-don't-open contract.")
  (input
    (do
      (import "temp" (Temp mk celsius))
      (def
        (main (: k Int64))
        (do
          (def m (Map.insert Map.empty 1 (mk k)))
          (match (Map.lookup m 1) ((Some t) (celsius t)) ((None _u) -1))))
      (export main)))
  (module "temp"
    (do
      (type Temp (T Int64))
      (def (mk (: c Int64)) (T (* c 10)))
      (def (celsius (: t Temp)) (match t ((T v) (/ v 10))))
      (export Temp)
      (export mk)
      (export celsius)))
  (call main (: 25 Int64))
  (output (: 25 Int64))
  (call main (: 0 Int64))
  (output (: 0 Int64)))

(case
  "equality of ABSTRACT-type values is rejected — opacity covers comparability"
  (doc
    "The eq face of the abstract-type discipline: an importer holding two bare-handle `Temp`
           values may NOT `=` them — rejected CDZ0202 (equality needs the type's content visible;
           an abstract handle withholds it, since content-eq would leak the smart constructor's
           internals bit-by-bit through equality probes). The importer compares only what the
           module exposes (an exported eq/accessor). Completes the hold-don't-open contract:
           construction CDZ0214, matching CDZ0214 (gate pending for bare), equality CDZ0202,
           holding/passing legal (the map pin).")
  (input
    (do
      (import "temp" (Temp mk))
      (def
        (main (: k Int64))
        (+ (* 10 (if (= (mk k) (mk k)) 1 0)) (if (= (mk k) (mk (+ k 1))) 1 0)))
      (export main)))
  (module "temp"
    (do (type Temp (T Int64)) (def (mk (: c Int64)) (T (* c 10))) (export Temp) (export mk)))
  (error CDZ0202))

(case
  "bare patterns on a CONCRETELY-exported type stay legal for importers"
  (doc
    "The working perimeter of the bare-pattern withheld-ctor gate (the abstract-type bypass is
           the held soundness finding): `Shape` exports its constructors CONCRETELY (`(. Shape *)`),
           so the importer's BARE `((Circle r) ...)` match is fully legal — as is the module's own
           internal bare match in `area` (150 + 16 → 166 at k=5; 16 at k=0 — Circle 0 → 0 → 0·10=0 + 16
           = 16). Boxes the gate fix: it must key on per-name
           VISIBILITY (withheld vs exported), not on bare-vs-qualified spelling — over-gating bare
           patterns on concrete types would break every ordinary importer match.")
  (input
    (do
      (import "shapes" (Shape area))
      (def
        (main (: k Int64))
        (+
          (* 10 (match (Shape.Circle k) ((Circle r) (* r 3)) ((Square s) (* s s)) (_ -1)))
          (area (Shape.Square 4))))
      (export main)))
  (module "shapes"
    (do
      (type Shape (Circle Int64) (Square Int64))
      (def (area (: s Shape)) (match s ((Circle r) (* r 3)) ((Square s2) (* s2 s2))))
      (export Shape)
      (export Shape.*)
      (export area)))
  (call main (: 5 Int64))
  (output (: 166 Int64))
  (call main (: 0 Int64))
  (output (: 16 Int64)))

; --- The cross-module api-error flow (concrete export spelling). ---
(case
  "a CONCRETELY-exported error sum flows back through two call layers and dispatches at the entry"
  (doc
    "The cross-module face of the api-error idiom: the sum is declared in a base module with the CONCRETE (export (. IoErr *)) spelling (a bare-handle export withholds the constructors — CDZ0214 — per the opacity rules), the Result flows through a middle module's fetch, and the entry dispatches all four arms through nested patterns.")
  (module "errs"
    (do (type IoErr (NotFound String) (Denied Int64) (Timeout)) (export IoErr.*)))
  (module "store"
    (do
      (import "errs" (IoErr))
      (def
        (fetch (: id Int64))
        (:
          (if
            (= id 1)
            (Result.Ok "data")
            (if
              (= id 2)
              (Result.Err (IoErr.NotFound "key2"))
              (if (= id 3) (Result.Err (IoErr.Denied 403)) (Result.Err (IoErr.Timeout unit)))))
          (Result String IoErr)))
      (export fetch)))
  (input
    (do
      (import "errs" (IoErr))
      (import "store" (fetch))
      (def
        (code (: id Int64))
        (match
          (fetch id)
          ((Result.Ok s) (String.byte-len s))
          ((Result.Err (IoErr.NotFound key)) (+ 100 (String.byte-len key)))
          ((Result.Err (IoErr.Denied c)) (+ 1000 c))
          ((Result.Err (IoErr.Timeout _u)) -1)))
      (def (main (: id Int64)) (code id))
      (export main)))
  (call main (: 1 Int64))
  (output (: 4 Int64))
  (call main (: 2 Int64))
  (output (: 104 Int64))
  (call main (: 3 Int64))
  (output (: 1403 Int64))
  (call main (: 9 Int64))
  (output (: -1 Int64))
  (live-objects 0))

; --- Import reflection: the reserved `__ast__` name -------------------------------------------------
; A module implicitly exports the reserved name `__ast__`, which reflects that module's canonical AST as
; a compile-time `Ast` value (DESIGN-compiler-primitives.md §3a — import reflection). It is imported in
; the ordinary name-list form alongside a module's real items, and binds to the SAME `Ast` value a
; `quote` of the module body would produce. A contract-agnostic compiler primitive: userspace walks the
; reflected AST (with `Ast.encode`, a transform, `Blake3.of`) to build a content-address / contract-id;
; the compiler models only "syntax", never what the reflected program means.
(case
  "import { __ast__ } binds the sibling module's canonical AST as a compound Ast value"
  (doc
    "`import \"lib\" (__ast__)` binds `__ast__` to the reflected canonical AST of module `lib` — the
           SAME `Ast` value a `quote` of `lib`'s module body denotes. A module body is a `(do …)`, which
           reflects to an `Ast.List`, so `__ast__` matches the `Ast.List` variant here. Reflection reuses
           the linker's already-loaded module document + the structural reifier, so it costs nothing at run
           time (the bound value is a compile-time constant). The exact byte-for-byte faithfulness against
           a `quote` of the module body is pinned by the `rcdzc` unit test
           `import_ast_reflection_binds_the_module_ast` (a `quote` of a `(do …)` block does not round-trip
           the ML surface, so it cannot ride a corpus case).")
  (module "lib"
    (do (def (answer) 42) (export answer)))
  (input
    (do
      (import "lib" (__ast__))
      (def (main) (match __ast__ ((Ast.List _) true) (_ false)))
      (export main)))
  (output (: true Bool)))

(case
  "a reflected module AST is a well-formed Ast value that round-trips through encode/decode"
  (doc
    "The reflected `__ast__` is an ordinary `Ast` value: `Ast.encode` serializes it to canonical
           bytes and `Ast.decode` reads them back to an equal tree (value-interchange.md — the encoding is
           a bijection). Pins that reflection produces a genuine `Ast` value, not a bespoke handle.")
  (module "lib"
    (do (def (answer) 42) (export answer)))
  (input
    (do
      (import "lib" (__ast__))
      (def
        (main)
        (match
          (Ast.decode (Ast.encode __ast__))
          ((Ok a) (= (Ast.encode a) (Ast.encode __ast__)))
          ((Err _) false)))
      (export main)))
  (output (: true Bool)))

(case
  "import { __ast__ } reflects a module containing char and symbol literals (reflection is total)"
  (doc "\a")
  (module "lib"
    (do (def (c) #\a) (def (tag) #"tag") (export c) (export tag)))
  (input
    (do
      (import "lib" (__ast__))
      (def (main) (match __ast__ ((Ast.List _) true) (_ false)))
      (export main)))
  (output (: true Bool)))

(case
  "a module reflects itself via Ast.module and exports a compile-time content-address a caller imports"
  (doc
    "SELF-REFLECTION (the P4 contract-id mechanism): module `c` uses the `Ast.module` intrinsic to
           reflect its OWN module AST, hashes it to a 32-byte digest, and exports that as a compile-time
           CONSTANT `cid`. A caller imports the constant directly — no per-caller AST transform, and no
           self-import. Pins that a module can self-reflect (Ast.module) and export a content-address
           constant (the compiler-side of userspace contract-id construction).
           The imported `cid` binds to its DEFINING module `c`, NOT the importer — a VALUE check, not a
           length one: the importer computes its OWN reflection digest `(Blake3.of (Ast.encode Ast.module))`
           and asserts it DIFFERS from the imported `cid` (they are different modules, so different digests).
           A use-site misreflection (`Ast.module` late-binding to the importer, the bug this pins) would make
           them EQUAL — a length check (`Bytes.len cid == 32`) could NOT catch it, since both digests are 32
           bytes; that weak check masked this miscompile. `Ast.module` reflects the file of the ACCESS SITE
           occurrence (the `(. Ast module)` node), so `c`'s exported reflection stays bound to `c`.")
  (module "c"
    (do (def (cid) (Blake3.of (Ast.encode Ast.module))) (export cid)))
  (input
    (do
      (import "c" (cid))
      (def (main) (if (= cid (Blake3.of (Ast.encode Ast.module))) 1 0))
      (export main)))
  (output (: 0 Int64)))

(case
  "an imported def whose body is Ast.module reflects its DEFINING module, not the importer"
  (doc
    "The BARE-reflection twin of the digest case above (no hash in between). A library `lib` exports a
           value def `m` whose body IS `Ast.module` — the reflection of `lib`. The importer references the
           imported `m` and compares it to its OWN `Ast.module` (the importer's module). They are different
           modules, so the `Ast` values differ: `(= m Ast.module)` is FALSE → 0. If `Ast.module` late-bound
           to the USE site (the bug), the imported `m` would reflect the IMPORTER and the two would be EQUAL
           → 1. Pins that a value-def reference folds the def body in place at its DEFINING occurrence, so the
           reflection binds to `lib` regardless of who imports `m` (the `Ast.module` fold keys on the access
           site's file, not the shared built-in reflect op-record).")
  (module "lib"
    (do (def m Ast.module) (export m)))
  (input (do (import "lib" (m)) (def (main) (if (= m Ast.module) 1 0)) (export main)))
  (output (: 0 Int64)))

(case
  "an imported nullary accessor projecting a field off a recursive self-reflected record descriptor folds"
  (doc
    "The operator's record-API cross-module path: a module `c` reflects ITSELF, builds a full descriptor
           record via a RECURSIVE comment-tolerant transform (`contract` collects the module's `type` forms,
           peeling comments), and exports a nullary accessor `cid` that projects the descriptor's `id` field
           (the `0x01 ++ blake3(canonical decl)` content-address). A CONSUMER imports `cid` and calls it. This
           folds entirely at compile time: the general const-evaluator evaluates the recursive `contract` over
           `Ast.module` to a constant record, projects `.id`, and delegates the `Ast.encode`/`Blake3.of` folds
           to `core_of` — so the imported `cid()` is a compile-time constant the consumer reads (a 32-byte
           digest here), NOT a runtime record build (which would fail to emit, its `Ast`-typed fields having no
           runtime rep). Pins that a self-reflected contract DESCRIPTOR — not just a bare Bytes id — crosses
           modules by folding. (The accessor is named `cid`, distinct from the `id` FIELD: a same-name
           accessor + field is a separate resolution matter.)")
  (module "c"
    (do
      (def
        (child (const (: form Ast)) (: i Int64))
        (match
          form
          ((Ast.List es) (match (List.at es i) ((Option.Some v) v) ((Option.None) (Ast.Name "?"))))
          (_ (Ast.Name "?"))))
      (def (name-of (const (: form Ast))) (match form ((Ast.Name n) n) (_ "")))
      (def (head-name (const (: form Ast))) (name-of (child form 0)))
      (def (peel (const (: x Ast))) (if (= (head-name x) "comment") (peel (child x 2)) x))
      (def
        (collect (const (: xs (List Ast))))
        (match
          xs
          (#list() (: #list() (List Ast)))
          (#list(h (.. t))
            (let
              ((g (peel h)) (tail (collect t)))
              (if (= (head-name g) "type") (List.prepend tail g) tail)))))
      (def
        (contract (const (: mm Ast)))
        (match
          mm
          ((Ast.List forms)
            #record((=
                id
                (Blake3.of
                  (Ast.encode (Ast.List (List.prepend (collect forms) (Ast.Name "types"))))))
              (= nm "")))
          (_ #record((= id b"") (= nm "")))))
      (def (cid) (. (contract Ast.module) id))
      (export cid)))
  (input (do (import "c" (cid)) (def (main) (Bytes.len (cid))) (export main)))
  (output (: 32 Int64)))

; -- whole-module ALIAS import projects a member fn (breaker batch 397b; the #3656 surface; effects stay name-resolved per the standing ruling) --
(case
  "mal1 a whole-module ALIAS import projects a member fn"
  (module "lib"
    (do (def (dbl (: x Int64)) (* x 2)) (export dbl)))
  (input (do (import "lib" m) (def (main (: n Int64)) (m.dbl n)) (export main)))
  (call main (: 21 Int64))
  (output (: 42 Int64)))

; -- breaker batch 409 (2026-08-26): PER-NAME import rename `(as orig alias)` semantic faces
; (#3719 resolve/link, same-hour probe; complements #3723's ML-surface rust tests with RUNNING corpus
; pins). imr1 basic bind+run, imr2 the descriptor-collision disambiguation (the section-8 dispatcher
; shape), imr3 one export under TWO aliases, imr4 mixed plain+rename list, imr5 the ORIGINAL name is
; unbound after rename (clean CDZ0101), imr6 alias collision rejects CDZ0201, imr7 renamed fn applies.
(case
  "imr1 a per-name import rename binds the export under the alias and runs"
  (module "lib"
    (do (def (descriptor) 30) (export descriptor)))
  (input (do (import "lib" ((as descriptor foo))) (def (main) (foo)) (export main)))
  (output (: 30 Int64)))

(case
  "imr2 per-name renames disambiguate a uniform export name from two modules"
  (module "liba"
    (do (def (descriptor) 30) (export descriptor)))
  (module "libb"
    (do (def (descriptor) 12) (export descriptor)))
  (input
    (do
      (import "liba" ((as descriptor a-desc)))
      (import "libb" ((as descriptor b-desc)))
      (def (main) (+ (a-desc) (b-desc)))
      (export main)))
  (output (: 42 Int64)))

(case
  "imr3 one export imported under TWO aliases — both bind to the same def"
  (module "lib"
    (do (def (descriptor (: x Int64)) (* x 3)) (export descriptor)))
  (input
    (do
      (import "lib" ((as descriptor tri) (as descriptor thrice)))
      (def (main) (+ (tri 5) (thrice 9)))
      (export main)))
  (output (: 42 Int64)))

(case
  "imr4 a MIXED list — plain import and rename side by side"
  (module "lib"
    (do (def (base) 40) (def (bump (: x Int64)) (+ x 1)) (export base) (export bump)))
  (input (do (import "lib" (base (as bump inc))) (def (main) (inc (base))) (export main)))
  (output (: 41 Int64)))

(case
  "imr5 the ORIGINAL name is NOT bound after a rename (unbound reject)"
  (module "lib"
    (do (def (descriptor) 30) (export descriptor)))
  (input (do (import "lib" ((as descriptor foo))) (def (main) (descriptor)) (export main)))
  (error CDZ0101))

(case
  "imr6 an alias colliding with another import is a colliding-import reject"
  (module "liba"
    (do (def (descriptor) 30) (export descriptor)))
  (module "libb"
    (do (def (other) 12) (export other)))
  (input
    (do
      (import "liba" ((as descriptor thing)))
      (import "libb" ((as other thing)))
      (def (main) (thing))
      (export main)))
  (error CDZ0201))

(case
  "imr7 a renamed FUNCTION import applies through the alias"
  (module "lib"
    (do (def (descriptor (: x Int64) (: y Int64)) (- x y)) (export descriptor)))
  (input (do (import "lib" ((as descriptor sub))) (def (main) (sub 50 8)) (export main)))
  (output (: 42 Int64)))

(case
  "a duplicate sum variant declaration carries a delete fix"
  (doc
    "A repeated sum VARIANT (payload form) in a type declaration is the same fixed-name-set collision as a
        duplicate def/export/type (CDZ0201), carrying a DELETE fix on the redundant variant. From rcdzc
        a_duplicate_sum_variant_op_and_map_key_each_carry_a_delete_fix.")
  (input (do (type C (Mk Int64) (Mk Int64) (Other)) (def (main) 0) (export main)))
  (error CDZ0201 (fix (kind delete))))

(case
  "a duplicate effect operation declaration carries a delete fix"
  (doc
    "A repeated effect OPERATION is a fixed-name-set collision (CDZ0201) with a DELETE fix on the redundant
        op. From rcdzc a_duplicate_sum_variant_op_and_map_key_each_carry_a_delete_fix.")
  (input (do (effect E (op a (-> Int64 Unit)) (op a (-> Int64 Unit))) (def (main) 5) (export main)))
  (error CDZ0201 (fix (kind delete))))

(case
  "performing a duplicate-operation effect reports one declaration-site error, not a leaked record-field consequent"
  (doc
    "PERFORMING a dup-op effect projects its synth record, which used to re-report the same duplicate as a
        misleading 'record names field more than once' (leaking the internal record). Only the declaration-site
        op reject remains → (no-other-errors) pins that no record-field consequent accompanies it. (migrated
        from rcdzc a_duplicate_effect_operation_is_rejected.)")
  (input
    (do
      (effect E (op get (-> Unit Int64)) (op get (-> Unit Bool)))
      (def (main) (E.get))
      (export main)))
  (error CDZ0201)
  (no-other-errors))

; ── bare zero-operand declaration keyword forms declare NOTHING → rejected, naming the form (migrated from
; rcdzc a_bare_declaration_keyword_form_declares_nothing_is_rejected) ──
; A bare `(def)` / `(type)` / `(effect)` has no name and no body/variants/ops — it declares nothing. It used
; to be SILENTLY ACCEPTED (it registers no Def/TypeDecl/EffectDecl, so the per-declaration walks never see it
; and `unknown_top_forms` skips it — its head IS a known keyword). Now each is CDZ0201 naming the bare form.
(case
  "a bare def declaration keyword form declares nothing and is rejected"
  (input (do (def) (def (main) 0) (export main)))
  (error CDZ0201 (message "declares nothing") (message "`(def)`")))

(case
  "a bare type declaration keyword form declares nothing and is rejected"
  (input (do (type) (def (main) 0) (export main)))
  (error CDZ0201 (message "declares nothing") (message "`(type)`")))

(case
  "a bare effect declaration keyword form declares nothing and is rejected"
  (input (do (effect) (def (main) 0) (export main)))
  (error CDZ0201 (message "declares nothing") (message "`(effect)`")))

; A definition is `(def <name> <value>)` / `(def (<name> <param>…) <body>)` — exactly ONE body and a real
; name (distinct from the bare `(def)` "declares nothing" cases above, which have no signature at all). A
; no-body def `(def (main))` formerly surfaced only at emit (a check≡compile gap); a too-many-body `(def
; (main) 1 2)` was silently accepted (the trailing form dropped — a silent miscompile); a nameless `(def ()
; …)` / `(def (5 x) …)` registered an empty unreachable name. All are now CDZ0201 (the too-many carries a
; delete-the-surplus fix). A QUOTED `(def …)` is inert data and is NOT flagged. (migrated from rcdzc
; a_definition_with_the_wrong_body_count_is_cdz0201.)
(case
  "a definition with a signature but no body is rejected"
  (input (do (def (main)) (export main)))
  (error CDZ0201 (message "has no body")))

(case
  "a value definition with no body is rejected"
  (input (do (def x) (def (main) 1) (export main)))
  (error CDZ0201 (message "has no body")))

(case
  "a definition with more than one body is rejected with a delete-the-surplus fix"
  (input (do (def (main) 1 2) (export main)))
  (error CDZ0201 (message "more than one body") (fix (kind delete))))

(case
  "a definition with an empty signature has no name and is rejected"
  (input (do (def () 1) (def (main) 0) (export main)))
  (error CDZ0201 (message "has no name")))

(case
  "a definition with a non-name signature head has no name and is rejected"
  (input (do (def (5 x) 1) (def (main) 0) (export main)))
  (error CDZ0201 (message "has no name")))

(case
  "a quoted def form is inert data and is not flagged as a malformed declaration"
  (input (do (def (main) (do (quote (def foo)) 0)) (export main)))
  (call main)
  (output (: 0 Int64)))

; Further def-signature well-formedness: a module is a record of its defs (a FIXED field set), so defining
; the same name twice is CDZ0201 "defined more than once" (with a delete-the-redundant-def fix), not an
; implicit first/last-wins. A parameter list is a linear BINDER position: a repeated param name is CDZ0102
; (nonlinear). (migrated from rcdzc a_duplicate_definition_is_rejected /
; a_duplicate_parameter_name_is_rejected_as_nonlinear. The literal-parameter reject
; `(def (f 5) …)` stays a rcdzc test: a bare literal in a param position is not ML-surface-parseable, so it
; has no corpus round-trippable form.)
(case
  "a duplicate definition is rejected with a delete-the-redundant-def fix"
  (input (do (def (f) 1) (def (f) 2) (def (main) (f)) (export main)))
  (error CDZ0201 (message "defined more than once") (fix (kind delete))))

(case
  "a duplicate parameter name is rejected as a nonlinear binder with a rename fix"
  (doc
    "CDZ0102 carries the mechanical repair: RENAME the repeated binder to a fresh non-colliding name
          (`x` → `x2`), making the parameter list linear. Heuristic (unverified — the rename clears the hard
          error but the fresh binder is then unused until the author wires it up). Enhanced from rcdzc
          a_non_linear_parameter_carries_a_rename_fix_avoiding_collisions.")
  (input (do (def (f x x) x) (def (main) (f 1 2)) (export main)))
  (error CDZ0102 (fix (kind replace) (replacement "x2") (unverified))))

(case
  "a nonlinear-parameter rename fix dodges an existing later binder name"
  (doc
    "The fresh name avoids EVERY param name (earlier AND later), not just a `+1` suffix: with a later
          `x2` already present, the duplicate `x` renames to `x3`, not the already-taken `x2`. Pins the
          collision-avoidance half of the rename heuristic. From rcdzc
          a_non_linear_parameter_carries_a_rename_fix_avoiding_collisions.")
  (input (do (def (f x x x2) x) (def (main) (f 1 2 3)) (export main)))
  (error CDZ0102 (fix (kind replace) (replacement "x3") (unverified))))

; An export clause names a DEFINITION: `(export <name>)` / `(export <name>…)`. An argument that is not a
; bare name — `(export (g x))` / `(export 5)` / `(export)` / a non-name element of a multi-name export — was
; SILENTLY DROPPED (the scan only recorded an Export when the argument `as_name`s), so the program compiled
; as if the export were never written. Now rejected CDZ0201 at the chokepoint ("an export names a
; definition"); a compound whose HEAD is a name recovers the intent with a replace-with-`g` fix, while a
; non-recoverable argument gets the message alone. A CONSTRUCTOR-export `(export (. T A))` / `(export (. T
; *))` (the opaque-types surface) is well-formed and NOT flagged; a malformed ctor-export `(. T)` / `(. T A
; B)` gets the constructor-export-specific message. (migrated from rcdzc
; a_malformed_export_clause_is_rejected_not_silently_dropped.)
(case
  "a compound export clause whose head is a name is rejected with a recover-the-name fix"
  (input (do (def (g) 1) (export (g x))))
  (error CDZ0201 (message "an export names a definition") (fix (kind replace) (replacement "g"))))

(case
  "a non-name export argument is rejected"
  (input (do (def (g) 1) (export 5)))
  (error CDZ0201 (message "an export names a definition")))

(case
  "an empty export clause is rejected"
  (input (do (def (g) 1) (export)))
  (error CDZ0201 (message "an export names a definition")))

(case
  "a non-name element of a multi-name export is rejected, not silently dropped"
  (input (do (def (a) 1) (export a 5)))
  (error CDZ0201 (message "an export names a definition")))

(case
  "a constructor-export of a named variant is well-formed and not flagged"
  (input (do (type T (A) (B)) (export T.A) (def (main) 1) (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a constructor-export of all variants (. T *) is well-formed and not flagged"
  (input (do (type T (A) (B)) (export T.*) (def (main) 1) (export main)))
  (call main)
  (output (: 1 Int64)))

(case
  "a malformed constructor-export with no ctor segment gets the constructor-export message"
  (input (do (type T (A) (B)) (export (. T)) (def (main) 1) (export main)))
  (error CDZ0201 (message "a constructor-export is")))

(case
  "a malformed constructor-export with too many segments gets the constructor-export message"
  (input (do (type T (A) (B)) (export (. T A B)) (def (main) 1) (export main)))
  (error CDZ0201 (message "a constructor-export is")))

(case
  "a constructor-export whose TYPE head is a near-miss suggests the declared type with a rename fix"
  (doc
    "`(export Colr.*)` names `Colr` where `(type Color …)` is declared — a near-miss of the
          declared sum. The type head names no sum type (CDZ0201, expected `to be a sum type`), and it
          suggests the declared type + carries a rename fix. `Colr.*` reads as ONE dotted atom (the `*` is a
          reserved, non-identifier final member segment the reader keeps unsplit — unlike a named ctor
          `T.A`, which desugars to a `(. T A)` list), so the rename replaces the WHOLE atom `Colr.*` ->
          `Color.*`. The type-name twin of the constructor-NAME did-you-mean. (Migrated from rcdzc
          a_ctor_export_with_a_mistyped_type_name_suggests_the_declared_type.)")
  (input (do (type Color (R) (G)) (export Colr.*) (def (main) 5) (export main)))
  (error
    CDZ0201
    (message "to be a sum type")
    (message "did you mean `Color`?")
    ; `Colr.*` reads as ONE dotted atom (`*` is a reserved non-identifier member segment the reader keeps
    ; unsplit), so the rename fix replaces the WHOLE atom — the replacement carries the `.*` tail
    ; (`Colr.*` -> `Color.*`), not the bare type name (which would drop the wildcard).
    (fix (kind replace) (replacement "Color.*"))))

; The did-you-mean fires ONLY on a plausible NEAR-MISS of a declared sum. A VALUE-named export head
; (`(export (. helper *))` where `helper` is a def, not a type) and a FAR-MISS undeclared name
; (`(export (. zzzzz *))`, no declared type near it) both still reject "to be a sum type" (CDZ0201) but carry
; NO spurious type suggestion — the `(not "did you mean")` message-absence pins that. (Migrated from rcdzc
; a_ctor_export_type_head_gets_no_spurious_did_you_mean.)
(case
  "a constructor-export whose head names a VALUE (not a sum type) gets no spurious did-you-mean"
  (input (do (def (helper) 5) (export helper.*) (def (main) 5) (export main)))
  (error CDZ0201 (message "to be a sum type") (not "did you mean")))

(case
  "a constructor-export whose head is a FAR-MISS undeclared name gets no spurious did-you-mean"
  (input (do (type Color (R)) (export zzzzz.*) (def (main) 5) (export main)))
  (error CDZ0201 (message "to be a sum type") (not "did you mean")))

; The CONSTRUCTOR-NAME near-miss (the twin of the TYPE-name near-miss above): the type head IS a declared
; sum, but the named variant is a near-miss of one of its constructors. `(export (. T Alph))` for
; `(type T (Alpha) (Beta))` names no constructor of `T` (CDZ0201), suggests the near variant, and carries a
; heuristic rename fix on the constructor occurrence (`Alph` -> `Alpha`). The two category-word faces pin how
; a NON-sum head is described: a VALUE definition head says "a value definition", an UNDECLARED head says
; "not a declared type" — both distinct from a near-miss sum (no did-you-mean). (Migrated from rcdzc
; a_constructor_export_is_semantically_validated.)
(case
  "a constructor-export naming a near-miss variant suggests the constructor with a rename fix"
  (input (do (type T (Alpha) (Beta)) (export T.Alph) (def (main) 1) (export main)))
  (error
    CDZ0201
    (message "is not a constructor of the sum type `T`")
    (message "did you mean `Alpha`?")
    (fix (kind replace) (replacement "Alpha") (unverified))))

(case
  "a constructor-export whose head is a value definition names the value-definition category"
  (input (do (def foo 5) (export foo.A) (def (main) 1) (export main)))
  (error CDZ0201 (message "to be a sum type") (message "a value definition")))

(case
  "a constructor-export whose head is an undeclared name names the not-a-declared-type category"
  (input (do (export Undeclared.A) (def (main) 1) (export main)))
  (error CDZ0201 (message "to be a sum type") (message "not a declared type")))

; --- A mutual-recursion cycle CROSSING a module boundary must DECLINE, not ICE. ---
(case
  "a mutual-recursion cycle crossing a module boundary declines (not a compiler panic)"
  (doc
    "A module fn `lib.f` and a ROOT fn `g` form a mutual-recursion CYCLE through the module projection
           (`lib.f` → `g` → `lib.f`). Re-entering the cycle via the module member transiently loses the lambda
           head, which used to PANIC the compiler ('lambda_body implies a lambda head', an ICE that also
           crashed `cdz check` / the editor loop; breaker). It now DECLINES with a coded reject
           (decline-don't-crash): the cycle is a recursion the inliner can't reduce and the module member
           can't (yet) emit a stable `Core::Call` for. IDEALISTIC (should-work, routed to v-module-system): a
           cross-module mutual cycle SHOULD compile like a ROOT-level mutual cycle does (a real recursive
           `Core::Call` once the module member resolves to its top-level def index) — this decline is the
           coded-reject-for-now, flipping to a value when that resolution-side lowering lands. One-way
           module→root refs and root-level mutual cycles already compile (breaker's green controls).")
  (input
    (do
      (module lib
        (def (f (: k Int64)) (if (= k 0) 0 (g (- k 1))))

        (export f))
      (def (g (: k Int64)) (if (= k 0) 1 (lib.f (- k 1))))
      (def (main (: n Int64)) (lib.f n))
      (export main)))
  ; IDEALISTIC should-work (operator: corpus is the impl-independent spec; a not-yet-implemented DECLINE is
  ; a TODO `(output V)`, NEVER `(error CDZ0900 …)`): the cross-module cycle SHOULD compute like its
  ; root-level twin — f(even)=0, f(odd)=1 — auto-Passing when the specializer lowers cross-module cycles;
  ; today it DECLINES (CDZ0900), so this grades TODO (decline-don't-crash: it must not ICE, #7916/breaker).
  (call main (: 4 Int64))
  (output (: 0 Int64))
  (call main (: 5 Int64))
  (output (: 1 Int64)))

; Cross-module mutual recursion: a module fn and a ROOT fn (or two modules) in a recursion CYCLE
; through the projection. Idealistically the cycle lowers like a root-level mutual pair (which
; computes today); currently the specializer declines it (CDZ0900 "recursive function needs runtime
; specialization", #7916 — the coded floor that replaced an ICE at lower/compute.rs:1597, which also
; crashed `cdz check`). TODO — auto-flips when cross-module cycles lower. f(even)=0, f(odd)=1.
; (breaker ICE probe 2026-09-02.)
(case
  "a mutual-recursion cycle crossing a module boundary computes like its root-level twin (should-work; today the specializer declines)"
  (input
    (do
      (module lib
        (def (f (: k Int64)) (if (= k 0) 0 (g (- k 1))))

        (export f))
      (def (g (: k Int64)) (if (= k 0) 1 (lib.f (- k 1))))
      (def (main (: n Int64)) (lib.f n))
      (export main)))
  (call main (: 4 Int64))
  (output (: 0 Int64))
  (call main (: 5 Int64))
  (output (: 1 Int64)))

(case
  "two modules in a mutual-recursion cycle compute like a root-level pair (should-work; today the specializer declines)"
  (input
    (do
      (module a
        (def (f (: k Int64)) (if (= k 0) 0 (b.g (- k 1))))

        (export f))
      (module b
        (def (g (: k Int64)) (if (= k 0) 1 (a.f (- k 1))))

        (export g))
      (def (main (: n Int64)) (a.f n))
      (export main)))
  (call main (: 4 Int64))
  (output (: 0 Int64))
  (call main (: 5 Int64))
  (output (: 1 Int64)))
