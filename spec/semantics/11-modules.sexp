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

(case "a module declaration binds its name in the enclosing scope"
  (doc    "Witnesses core-semantics.md #A Module Binds Its Name In Its Enclosing Scope: the module
           declaration `(module m …)` puts `m` in scope for the following form of the (do …) block —
           no `let` is needed to name it. Together with #A Module Evaluates To A Record Of Its Exports
           (3rd sentence: an exported definition is reachable by member access), `(. m answer)` reaches
           the export and the outer parens apply the nullary export function.")
  (input  (do
            (module m
              (def (answer) 42))
            ((. m answer) unit)))
  (output (: 42 Int64)))

(case "a nullary module export applied to a non-unit argument is rejected"
  (doc    "A nullary export `(def (answer) 42)` is a `Unit -> Int64` function (core-semantics.md §A Nullary
           Function's Argument Type Is Unit), INVOKED by applying it to `unit`. Applying it to a non-unit
           argument — `((. m answer) 5)`, `5 : Int64` — is a type error and MUST be rejected CDZ0203
           (cannot unify Unit with Int64), exactly as a written `(def (f (: u Unit)) 42)` applied to `(f 5)`
           is. The module synthesizes the field as a lambda over one ignored param; that param is ANNOTATED
           `Unit`, not bare — a bare param would take a fresh type variable (typing the export `∀a. a ->
           Int64`) and SILENTLY ACCEPT the wrong argument, running the ill-typed program to 42.")
  (input  (do
            (def (main)
              (do (module m (def (answer) 42))
                  ((. m answer) 5))) (export main)))
  (error  CDZ0203))

(case "a nullary module export's non-unit argument does not swallow the argument's own fault"
  (doc    "The companion of the reject above: because the synthesized param is `Unit`-typed rather than a
           free type variable, a non-unit argument is not silently dropped — so its OWN fault surfaces too.
           `((. m answer) (/ 1 0))` is rejected rather than running to 42 with the `(/ 1 0)` discarded (a
           bare-param free variable would accept and drop it). The argument disagrees with Unit → CDZ0203.")
  (input  (do
            (def (main)
              (do (module m (def (answer) 42))
                  ((. m answer) (/ 1 0)))) (export main)))
  (error  CDZ0203))

(case "a module in a top-level do sequence type-checks its members"
  (doc    "A `(module …)` may sit as an ELEMENT of a top-level `(do …)` sequence root. Its members must be
           type-checked exactly as a bare top-level `(module …)`'s are: an ill-typed nullary member — here
           `(def (bad) (+ 1 2.0))`, a Float/Int mix — MUST be rejected CDZ0301, not silently accepted. This
           position was a type-check hole: the top-level scan registers only def/export/type/effect items
           (no `module` branch), and the nested-declaration walk SKIPS a top-level item as already-scanned,
           so a module here was registered by NEITHER path — its members escaped `collect_faults` while a
           bare `(module m …)` and a def-body-nested one were both checked. Now a top-level module item is
           registered via the shared module-gather, so its member bodies are type-checked wherever the
           module sits (core-semantics.md #A program that is not well-typed MUST be rejected). Also holds
           for a Bool/Int mix and an unbound name in the member.")
  (input  (do
            (module m (def (bad) (+ 1 2.0)))
            (def (main) 5)
            (export main)))
  (error  CDZ0301))

(case "a top-level module is named by a top-level def's body"
  (doc    "Witnesses core-semantics.md #A Module Binds Its Name In Its Enclosing Scope from a NEW position:
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
  (input  (do
            (module Temp (def (c-to-f c) (+ (/ (* c 9) 5) 32)) (export c-to-f))
            (def (main) ((. Temp c-to-f) 100))
            (export main)))
  (output (: 212 Int64)))

(case "each definition in a module registers a reachable export field"
  (doc    "Witnesses core-semantics.md #A Module Evaluates To A Record Of Its Exports (2nd sentence:
           each definition registers its name and value as a field of the module's record): a module
           with two definitions exposes each as a field of its record, so both are reachable by member
           access. Applying both exports and summing shows neither shadows nor displaces the other —
           the record carries a field per definition.")
  (input  (do
            (module m
              (def (one) 1)
              (def (two) 2))
            (+ ((. m one) unit) ((. m two) unit))))
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

(case "a module member not named by the export clause is private"
  (doc    "The module exports only `pub`, so `secret` is a private definition — absent from the module's
           export record. Projecting `(. m secret)` names a field the record does not carry, the
           closed-record CDZ0201 (modules-and-namespaces.md §Visibility Is Explicit: a definition not made
           visible MUST NOT be reachable outside the module). Before this, a module's record carried EVERY
           definition regardless of the export clause, so `(. m secret)` reached a private helper — an
           over-exposure the explicit-visibility rule forbids. The export clause now filters the record.")
  (input  (do
            (module m
              (def (pub x) (+ x 1))
              (def (secret x) (+ x 100))
              (export pub))
            (def (main) ((. m secret) 5))
            (export main)))
  (error  CDZ0201))

(case "a module member named by the export clause is reachable"
  (doc    "The visible companion of the private case: `pub` IS named by `(export pub)`, so it is a field
           of the module's record and `(. m pub)` reaches it — pub(5) = 6. Pins that filtering the record
           to the export clause does not withhold a NAMED export (only the unnamed `secret` is hidden).")
  (input  (do
            (module m
              (def (pub x) (+ x 1))
              (def (secret x) (+ x 100))
              (export pub))
            (def (main) ((. m pub) 5))
            (export main)))
  (output (: 6 Int64)))

(case "a private module member is still visible to a sibling"
  (doc    "Explicit visibility withholds a member's OUTWARD reachability, not its INTRA-module visibility:
           `helper` is not exported (so `(. m helper)` from outside would be CDZ0201), but `pub` — which IS
           exported — calls `helper` by name in its own body, exactly as any two module definitions are
           mutually visible (§A Module Function Calls A Sibling Export By Name). So the export clause hides
           `helper` from the record while `pub`'s body still reaches it: pub(3) = helper(3) + 1 = 7. Pins
           that the visibility filter touches only the export record (`modules::module_record`), not the
           sibling scope (`resolve::module_sibling_binds`), so a private helper stays internally callable.")
  (input  (do
            (module m
              (def (helper x) (* x 2))
              (def (pub x) (+ (helper x) 1))
              (export pub))
            (def (main) ((. m pub) 3))
            (export main)))
  (output (: 7 Int64)))

(case "a private sibling defined after its exported caller still resolves"
  (doc    "The DEFINITION-ORDER companion of the private-sibling case above: there `helper` precedes its
           caller; here the exported `pub` comes FIRST and forward-references the private `helper` defined
           after it. Sibling visibility is order-independent (every member sees every member), and the
           privacy filter must not interact with the forward-reference path: pub(21) = helper(21) = 42.
           A resolver that binds siblings in definition order — or one that consults the (filtered) export
           record for a not-yet-seen name — breaks exactly this shape.")
  (input  (do
            (module m
              (export pub)
              (def (pub (: x Int64)) (helper x))
              (def (helper (: x Int64)) (* x 2)))
            ((. m pub) 21)))
  (output (: 42 Int64)))

(case "a mutually-recursive pair fully named by the export clause resolves"
  (doc    "A mutual-recursion CYCLE inside a module with an export clause naming BOTH members:
           even↔odd, `((. m even) 4)` = 1 (4 is even; the cycle bottoms out through 4→3→2→1→0). The
           knot-tying for a mutually-recursive module group must survive the presence of an export
           clause — this pins the both-exported face (the export-everything default cycle already works;
           the one-private face is the open false-rejection filed as
           adv-private-module-member-in-mutual-recursion-false-reject).")
  (input  (do
            (module m
              (export even odd)
              (def (even (: n Int64)) (if (= n 0) 1 (odd (- n 1))))
              (def (odd (: n Int64)) (if (= n 0) 0 (even (- n 1)))))
            ((. m even) 4)))
  (output (: 1 Int64)))

(case "one module's export clause does not affect a same-named member of another module"
  (doc    "Privacy is PER-MODULE state: module `a` exports only `pub` (hiding its `helper`), while module
           `b` has NO export clause, so ITS `helper` keeps the export-everything default — `(. b helper)`
           reaches it (7 × 3 = 21) even though a same-named member of `a` is private. A privacy filter
           keyed by NAME rather than by (module, name) — e.g. a global hidden-names set — would let `a`'s
           clause shadow `b`'s member. Pins the filter's scope is the declaring module's record only.")
  (input  (do
            (module a
              (export pub)
              (def (helper (: x Int64)) (* x 2))
              (def (pub (: x Int64)) (helper x)))
            (module b
              (def (helper (: x Int64)) (* x 3)))
            ((. b helper) 7)))
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

(case "a module value definition registers a reachable export field"
  (doc    "The value-definition companion of the case above: `(def v 7)` is a value definition, not a
           function, so `(. m v)` projects the field directly (no `unit` application) and yields 7 —
           core-semantics.md #A Module Evaluates To A Record Of Its Exports (each definition registers its
           name and value as a field) with the glossary's Definition = 'a value, function, type'. A
           compiler that registers only function definitions drops `v`; `(. m v)` then traps at run time
           on a missing field of an emitted component — a decline-don't-miscompile violation, since the
           program is well-typed and its value is 7. A generation that does not yet register value
           definitions MUST decline rather than emit a component that traps.")
  (input  (do
            (module m
              (def v 7))
            (. m v)))
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

(case "a module with two definitions of the same name is rejected"
  (doc    "`(def (f) 1)` and `(def (f) 2)` both register the field `f` of the module's record — but a
           record has a FIXED SET of field names (core-semantics.md #A Record Has A Fixed Set Of Named
           Fields), so registering `f` twice is the same ill-formedness the record literal `(record (a 1)
           (a 2))` is rejected for (CDZ0201). The module MUST be rejected, not resolved by keeping the
           first definition and discarding the second (which yields `(f)` = 1) — an implicit first-wins
           precedence the fixed field set forbids, exactly as modules-and-namespaces.md #Importing forbids
           resolving two same-named imports by precedence. Pins that the duplicate-field check reaches a
           module's definitions, not only a record literal's fields (core-semantics.md #A Module Evaluates
           To A Record Of Its Exports: each definition registers its name as a field). A generation that
           does not yet detect a duplicate definition declines rather than silently choosing one.")
  (input  (do
            (def (f) 1)
            (def (f) 2)
            (def (main) (f)) (export main)))
  (error  CDZ0201))

(case "two sibling modules may each define a private helper of the same name"
  (doc    "The duplicate-definition check is PER-MODULE, not global across a linked package: a module's
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
    (do
      (def (foo (: x Int64)) (+ x 1))
      (def (bump (: x Int64)) (foo x))
      (export bump)))
  (input  (do
            (import "lib" (bump))
            (def (foo (: x Int64)) (* x 2))
            (def (main) (+ (foo 5) (bump 5)))
            (export main)))
  (output (: 16 Int64)))

; A duplicate EXPORT clause is the export-side analogue of the duplicate definition above: a module's
; exports are a record whose fields are the exported names (core-semantics.md #A Module Evaluates To A
; Record Of Its Exports), and a record has a fixed set of field names, so exporting the same name twice
; places two entries under one field — the same CDZ0201 ill-formedness. It MUST be rejected before
; emitting: two export entries of one name are forbidden by the component binary format, so emitting
; them produces a component that fails to parse — a decline-don't-miscompile violation.

(case "a duplicate export clause for the same name is rejected"
  (doc    "`(export a)` twice names the export `a` twice. A module's exports are a record with a fixed
           set of field names, so a repeated export is the CDZ0201 duplicate-field ill-formedness — the
           export analogue of the duplicate definition above and of `(record (a 1) (a 2))`. The compiler
           MUST reject it (CDZ0201), never emit a component with two export entries named `a` (which the
           component binary format forbids, so the emitted bytes fail to parse).")
  (input  (do (def (a) 42) (export a) (export a)))
  (error  CDZ0201))

(case "a duplicate export of the entry is rejected"
  (doc    "The `main` sibling: `(export main)` twice. Same CDZ0201 duplicate-export rejection — the
           defect is independent of the exported name, not special to the entry-selection path.")
  (input  (do (def (main) 42) (export main) (export main)))
  (error  CDZ0201))

; --- A non-kebab export name crosses under a normalized kebab-case extern name ------------------------
; A Cadenza identifier may contain uppercase letters (`fA`, `Foo`) or underscores (`my_func`) — all valid
; source names — but the component model requires an export's extern name to be KEBAB-CASE (lowercase
; words, hyphen-separated). Emitting a non-kebab name verbatim produces a component that fails to validate
; (an unloadable artifact). The compiler NORMALIZES a non-kebab export name to a valid kebab extern name
; (`fA` → `f-a`, `my_func` → `my-func`) — deterministically, so a caller still names the export by its
; source identifier and the runner resolves it through the same rule. Two DISTINCT source names that
; normalize to the SAME extern name is a collision the compiler rejects (CDZ0201), like a duplicate export.

(case "an export whose name is not kebab-case crosses under a normalized extern name"
  (doc    "`(def (fA (: x Int64)) (+ x 1))` with `(export fA)` — `fA` is a valid Cadenza identifier
           (uppercase identifiers are legal) but NOT a valid component extern name. Rather than emit an
           unloadable component (the old miscompile: `export name fA is not a valid extern name`), the
           compiler normalizes the extern name to kebab-case `f-a`; the export is invoked by its SOURCE
           name `fA`, which the runner resolves through the same normalization. `(fA 5)` = 6. Pins that a
           non-kebab export name produces a LOADABLE component, not a silently-invalid artifact.")
  (input  (do (def (fA (: x Int64)) (+ x 1)) (export fA)))
  (call   fA (: 5 Int64))
  (output (: 6 Int64)))

(case "an underscore export name crosses under a normalized extern name"
  (doc    "The underscore shape: `(def (my_func (: x Int64)) (+ x 1))` with `(export my_func)` normalizes
           to the kebab extern name `my-func`. `(my_func 5)` = 6. Confirms the normalization covers the
           underscore separator, not only camelCase — every non-kebab source name yields a loadable
           component.")
  (input  (do (def (my_func (: x Int64)) (+ x 1)) (export my_func)))
  (call   my_func (: 5 Int64))
  (output (: 6 Int64)))

(case "two exports normalizing to the same kebab extern name are rejected"
  (doc    "`(export fA)` and `(export f-a)` both normalize to the extern name `f-a` — a collision the
           component boundary cannot carry (two exports of one name). The compiler rejects it CDZ0201, the
           same duplicate-export ill-formedness as two identical export names, rather than silently
           merging or dropping one. Distinct from the duplicate-export cases above: here the SOURCE names
           differ (`fA` vs `f-a`) but their normalized extern names coincide.")
  (input  (do (def (fA (: x Int64)) (+ x 1)) (def (f-a (: y Int64)) (+ y 2)) (export fA) (export f-a)))
  (error  CDZ0201))

(case "a top-level value definition binds a name usable by the program's functions"
  (doc    "A definition is 'a value, function, type' (glossary), and each registers its name in the module
           (core-semantics.md #A Module Evaluates To A Record Of Its Exports). So a VALUE definition
           `(def answer 42)` at the program's top level MUST bind `answer` for the module's functions to
           reference, exactly as a function definition binds its name — `(def (main) answer)` yields 42. The
           nested-module value-def case earlier in this file (`(do (module m (def v 7)) (. m v))`) pins the
           same rule for a module in do-position; this pins it for the OUTER program module. A compiler that
           accepts only function definitions `(def (f …) …)` at top level rejects this well-typed program
           (\"def without a signature\") — but a value definition is an ordinary definition form, so it MUST
           bind here. (This is load-bearing for a Cadenza-authored compiler whose shared tables — e.g. an
           opcode record generated as `(def op (record …))` — are top-level value definitions.)")
  (input  (do
            (def answer 42)
            (def (main) answer) (export main)))
  (output (: 42 Int64)))

(case "a top-level value definition binds a record projected by the program's functions"
  (doc    "The record companion of the scalar value-def above: a top-level value definition may bind a
           RECORD, and a function projects its fields by member access (core-semantics.md #Member Access
           Projects A Record Field). `(def tbl (record (a 7) (b 8)))` binds `tbl`; `(. tbl b)` is 8. This is
           exactly the shape a Cadenza-authored compiler's generated opcode table takes — `(def op (record
           (i64-const 0x42) …))` — a top-level record value read by `(. op i64-const)`, so it is load-bearing
           for self-hosting. A compiler that accepts only function definitions at top level rejects this
           well-typed program (\"def without a signature\"); a value definition binding a record MUST bind
           here and project.")
  (input  (do
            (def tbl (record (a 7) (b 8)))
            (def (main) (. tbl b)) (export main)))
  (output (: 8 Int64)))

(case "a top-level value definition may reference a value defined later in the module"
  (doc    "A module's definitions form a mutually-visible scope, not a top-to-bottom sequence: a value
           definition may reference a name bound by a LATER definition (core-semantics.md #A Module
           Evaluates To A Record Of Its Exports — every top-level name is in scope in every definition's
           body). `(def b (+ a 4))` uses `a`, which is defined AFTER it as `(def a 3)`; the module resolves
           `a` = 3 regardless of order, so `b` = 7. Pins that value-definition resolution is order-independent
           (a compiler that resolved names strictly top-to-bottom would report `a` unbound in `b`'s body),
           the same forward visibility a function definition already enjoys.")
  (input  (do
            (def b (+ a 4))
            (def a 3)
            (def (main) b)
            (export main)))
  (output (: 7 Int64)))

(case "a value definition may carry a leading doc, like a function definition"
  (doc    "A `(doc …)` form immediately after the definition's name/signature documents it and is not part
           of the value; a FUNCTION definition already accepts one (`(def (f) (doc \"…\") body)`), and a
           VALUE definition MUST accept one symmetrically — a definition is 'a value, function, type'
           (glossary), so the doc affordance cannot depend on which. `(def answer (doc \"the answer\") 42)`
           binds `answer` = 42 with the doc ignored for the value. A compiler that reads a value def as
           exactly name+value rejects the doc'd form (\"value def without a single value expression\") while
           accepting the doc'd function form — an asymmetry a definition form must not have. Load-bearing
           for a Cadenza-authored compiler whose generated shared tables are documented value defs (e.g.
           `(def op (doc \"opcode bytes\") (record …))`).")
  (input  (do
            (def answer (doc "the answer") 42)
            (def (main) answer) (export main)))
  (output (: 42 Int64)))

(case "a line comment wrapping a top-level form does not hide it"
  (doc    "A leading `//` line comment on a top-level form reifies (by the reader) to `(comment \"<text>\"
           <form>)` wrapping the WHOLE form — the comment companion of the leading `(doc …)` above. The
           comment is SEMANTICALLY INERT (self-hosting-surface.md §the tree carries comments and
           documentation — the compiler sees through comments as it sees through docs), so the compiler must
           peel it to the wrapped form. `(comment \"note\" (def (f (: x Int64)) x))` defines `f`, and
           `(f 7)` = 7. A compiler that peels a leading `(doc …)` but NOT a `(comment …)` reads `comment` as
           an unknown top-level declaration head → the wrapped `def` is invisible ('unbound name comment' +
           `f` unbound). Load-bearing for a Cadenza-authored compiler whose own sources carry ordinary
           top-level `//` comments.")
  (input  (do
            (comment "note" (def (f (: x Int64)) x))
            (def (main) (f 7))
            (export main)))
  (output (: 7 Int64)))

(case "stacked line comments on a top-level form are all seen through"
  (doc    "Stacked `//` lines on one form NEST — `// a` then `// b` above `def f` is `(comment \"a\"
           (comment \"b\" (def …)))` — so the compiler must peel to the INNERMOST form, not just one layer.
           `f` still defines and `(f 7)` = 7. Pins that the comment peel follows the whole nested chain, the
           multi-line-comment shape a real source file's header block takes.")
  (input  (do
            (comment "a" (comment "b" (def (f (: x Int64)) x)))
            (def (main) (f 7))
            (export main)))
  (output (: 7 Int64)))

(case "a line comment wrapping a type declaration is seen through"
  (doc    "The comment peel is not `def`-specific — it must see through a comment wrapping ANY top-level
           form. `(comment \"the color\" (type C (R) (G)))` declares the type `C`; the program then
           constructs and matches its variants → 1 for `C.R`, 2 for `C.G`, selected by a runtime Bool. Pins
           that a leading `//` on a `type` declaration does not hide it (the type-decl companion of the
           def case above), so a commented type in a compiler's IR-sum module stays visible.")
  (input  (do
            (comment "the color" (type C (R) (G)))
            (def (main (: b Bool)) (match (if b (C.R) (C.G)) ((C.R) 1) ((C.G) 2)))
            (export main)))
  (call   main (: true Bool)) (output (: 1 Int64))
  (call   main (: false Bool)) (output (: 2 Int64)))

(case "a line comment wrapping the entry point is seen through"
  (doc    "The comment peel reaches the ENTRY too: `(comment \"run it\" (def (main …) …))` wraps the exported
           entry, which must still be found and run. `dbl` is defined plainly; the commented `main` doubles
           its argument → 10 for 5. Pins that a `//` on the entry point does not hide it from the export
           scan (a commented `main`/entry is the natural top of a source file), the entry companion of the
           def and type cases.")
  (input  (do
            (def (dbl (: x Int64)) (+ x x))
            (comment "run it" (def (main (: x Int64)) (dbl x)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 10 Int64)))

(case "a module function calls a sibling export by name"
  (doc    "Witnesses core-semantics.md #A Module Binds Its Name In Its Enclosing Scope (2nd sentence:
           module bindings resolve under the same lexical scope rules as any other binding) together
           with #A Module Evaluates To A Record Of Its Exports: a module's exported definitions are in
           scope in each other's bodies, exactly as top-level definitions are mutually visible. `f`
           calls its sibling `dbl` by name; f(3) = dbl(3) + 1 = 7. Intra-module references are the norm
           — a prelude or a group of compiler passes is a module whose functions call one another.")
  (input  (do
            (module lib
              (def (dbl x) (* x 2))
              (def (f x) (+ (dbl x) 1)))
            ((. lib f) 3)))
  (output (: 7 Int64)))

(case "a module function is recursive"
  (doc    "Witnesses core-semantics.md #A Module Evaluates To A Record Of Its Exports with a
           self-reference: an exported function is in scope in its own body, so it may recurse.
           `fac` calls itself; fac(5) = 120. A recursive export resolves by the same lexical scope a
           top-level recursive def does AND lowers the same way — the member is registered as a standalone
           emittable function, so the self-call is a runtime `Core::Call`, not an unbounded inline. A
           compiler that resolves the recursion but cannot emit a non-top-level recursive callee declines
           (a Todo); one that models it runs `fac` to 120.")
  (input  (do
            (module lib
              (def (fac n) (if (= n 0) 1 (* n (fac (- n 1))))))
            ((. lib fac) 5)))
  (output (: 120 Int64)))

(case "two module functions are mutually recursive"
  (doc    "Mutual recursion between two module members: `ev` calls `od`, `od` calls `ev` — neither reaches
           a normal form by inlining, so BOTH lower to standalone runtime functions calling each other
           (core-semantics.md #A Module Evaluates To A Record Of Its Exports: the members are mutually
           visible, so each names the other, and each is emittable). ev(10) is true → 1. Pins that the
           member-registration reaches an EACH-OTHER call group, not only a single self-recursive member.")
  (input  (do
            (module m
              (def (ev n) (if (= n 0) true (od (- n 1))))
              (def (od n) (if (= n 0) false (ev (- n 1)))))
            (if ((. m ev) 10) 1 0)))
  (output (: 1 Int64)))

(case "a recursive function in a nested module runs through the projection chain"
  (doc    "A recursive function in a NESTED module is reached AND lowered through the member-access chain:
           `(. (. outer inner) fac)` projects the inner module's `fac`, whose self-call lowers to a runtime
           `Core::Call` to the same registered member (its `Member`-headed call site reduces to the field
           lambda's body, the def identity the recursion emits against). fac(5) = 120. Composes the
           module-in-module nesting with the recursive-member lowering.")
  (input  (do
            (module outer
              (module inner
                (def (fac n) (if (= n 0) 1 (* n (fac (- n 1)))))))
            ((. (. outer inner) fac) 5)))
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

(case "a module nested in a module projects as a nested record field"
  (doc    "Witnesses core-semantics.md #A Module Evaluates To A Record Of Its Exports for a NESTED module:
           `(module inner (def v 7))` written as a member of `(module outer …)` registers `inner` as a
           field of the outer's record whose value is the inner module's OWN record, so `(. (. outer inner)
           v)` is two ordinary member projections (core-semantics.md #Member Access Projects A Record Field)
           and yields 7 — the nested-record analogue of a flat export, nothing privileged by name. A compiler
           that registers only `(def …)` members drops the nested module; `(. outer inner)` then names a
           missing field and TRAPS on the emitted component — a decline-don't-miscompile violation, so a
           generation that does not model nested modules declines rather than emitting a trapping projection.")
  (input  (do
            (module outer
              (module inner
                (def v 7)))
            (. (. outer inner) v)))
  (output (: 7 Int64)))

(case "a module may nest three deep"
  (doc    "Nesting is arbitrary-depth: `(module a (module b (module c (def v 42))))` reaches `v` through
           three member accesses `(. (. (. a b) c) v)`. Pins that a nested module is itself a record whose
           fields may be records recursively — no depth privilege, the same `synth_by_occ`-embed at each
           level (`modules::synthesize` builds inner-first so each enclosing module embeds an already-built
           record).")
  (input  (do
            (module a
              (module b
                (module c
                  (def v 42))))
            (. (. (. a b) c) v)))
  (output (: 42 Int64)))

(case "a nested module's function export is applied through the projection chain"
  (doc    "A nested module's FUNCTION export is reached AND applied through the member-access chain: the
           inner field value is the same `(fn (params) body)` lambda a flat export carries, so `((. (. outer
           inner) f) 21)` β-reduces by the ordinary application path — f(21) = 42. Pins that the nested
           record's fields carry lambdas identically to a top-level module's, not only bare values.")
  (input  (do
            (module outer
              (module inner
                (def (f x) (* x 2))))
            ((. (. outer inner) f) 21)))
  (output (: 42 Int64)))

(case "two adjacent modules declared inside a function body compose"
  (doc    "A function body is a `(do …)` sequence that may hold BODY-LOCAL module declarations: `main`'s
           body declares `Inc` then `Scale` then uses both — `Scale.g(Inc.f(4))` = 50. Pins that two
           ADJACENT nested modules in a def body both register (each its own nested record) and the trailing
           expression reaches them. The point beyond the value: the ML surface round-trip. The ML printer
           emits a non-final declaration-keyword statement PARENTHESIZED — `(module Inc { … }); (module Scale
           { … }); …` — because the reader's `;`-sequence otherwise BREAKS before a bare `module` keyword
           (treating it as the next top-level form), truncating the body after the first module. The parens
           make each a bracketed expression the reader collects into the body, so the printer emits ML the
           reader reads back to this same tree (the roundtrip harness exercises exactly that path). Without
           the wrapping the printer produced ML it then rejected — a printer/reader round-trip failure.")
  (input  (do
            (def (main)
              (do (module Inc (def (f x) (+ x 1)))
                  (module Scale (def (g x) (* x 10)))
                  ((. Scale g) ((. Inc f) 4))))
            (export main)))
  (call   main)
  (output (: 50 Int64)))

(case "an outer definition references a sibling nested module by bare name"
  (doc    "A module's members are mutually visible (core-semantics.md #A Module Evaluates To A Record Of
           Its Exports), and a nested module is a member — so an outer `(def …)` may reference the sibling
           nested module by BARE name. `f`'s body reads `(. inner dbl)`, resolving `inner` to the inner
           module's record via the same in-module sibling scope a bare def reference uses; f(21) = dbl(21) =
           42. Pins that the nested module participates in in-module scope as a member, not only as a
           qualified projection target.")
  (input  (do
            (module outer
              (module inner
                (def (dbl x) (* x 2)))
              (def (f y) ((. inner dbl) y)))
            ((. outer f) 21)))
  (output (: 42 Int64)))

(case "a module's delegated capability is reachable as metadata, not as an export"
  (doc    "Witnesses core-semantics.md #A Module Carries Its Manifest And Entry As Metadata:
           the capabilities are reached by the (meta …) key, distinct from the export
           namespace, so they never collide with an export. The module declares the routing-agnostic
           effect `log` and its entry `main` DELEGATES it to the host with `(host (log) …)`; the manifest
           is the union of the entry's delegations, so the capabilities metadata contains \"log\" (the
           delegation — not the declaration — is the grant, capabilities-and-effects.md #The Program
           Manifest Is The Union Of Its Entrypoints' Delegations).")
  (input  (do
            (module m
              (effect log (op emit (-> String Unit)))
              (def (main) (host (log) (log.emit "hi"))))
            (= (. m (meta capabilities)) (list "log"))))
  (output (: true Bool)))

(case "a delegated capability is not itself an export field"
  (doc    "Witnesses core-semantics.md #A Module Carries Its Manifest And Entry As Metadata (1st
           sentence): a delegated capability is carried as metadata SEPARATE from the exported fields,
           so it is not itself an export. The module's entry delegates `log` to the host but the module
           exports only `main`; a module IS a record of its exports, and `log` is not among them, so
           projecting it is a COMPILE-TIME type error (CDZ0201) — naming a field the record does not
           contain (core-semantics.md #Member Access Projects A Record Field), rejected before lowering
           rather than deferred to a runtime trap. The capability lives in `(meta capabilities)`
           (witnessed by the case above), not among the export fields, so `log` resolves to no export.")
  (input  (do
            (module m
              (effect log (op emit (-> String Unit)))
              (def (main) (host (log) (log.emit "hi"))))
            (. m log)))
  (error  CDZ0201))

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

(case "an unrecognized pragma key is rejected rather than ignored"
  (doc    "`(pragma frobnicate 3)` names a key the pinned registry does not define, so the module is
           REJECTED (CDZ0601, modules-and-namespaces.md #An Unrecognized Module Directive Is Rejected),
           not silently ignored. THE reason the channel is strict: a dropped meaning-changing directive
           would make one source mean two things on two toolchains. The general-mechanism companion of
           the numeric `default-integer` cases.")
  (input  (do
            (module m
              (pragma frobnicate 3)
              (def (answer) 42))
            ((. m answer) unit)))
  (error  CDZ0601))

(case "a recognized pragma with a malformed argument list is rejected"
  (doc    "`(pragma default-integer)` names a registered key but omits its one required argument, so it
           is rejected against the shape the key defines (CDZ0602, modules-and-namespaces.md #A Module
           Directive Is Drawn From A Fixed Set, 2nd sentence). Distinct from CDZ0601 (unknown key) and
           from CDZ0303 (a well-formed directive whose type argument fails the integer-domain predicate):
           here the directive is structurally malformed.")
  (input  (do
            (module m
              (pragma default-integer)
              (def (answer) 42))
            ((. m answer) unit)))
  (error  CDZ0602))

(case "an export and a like-named metadata key do not collide"
  (doc    "Witnesses core-semantics.md #A Module Carries Its Manifest And Entry As Metadata (2nd
           sentence): metadata is reached by a key distinct from every export name, so metadata access
           cannot collide with an export. This module's entry delegates `log` to the host AND the module
           exports a definition literally named `capabilities`. The export `(. m capabilities)` resolves
           to that definition (applied, it yields 7), while `(. m (meta capabilities))` resolves to the
           manifest — the same spelling in the two channels denotes two different things, which is the
           whole reason metadata lives behind (meta …).")
  (input  (do
            (module m
              (effect log (op emit (-> String Unit)))
              (def (capabilities) 7)
              (def (main) (host (log) (log.emit "hi"))))
            (if (= ((. m capabilities) unit) 7)
                (= (. m (meta capabilities)) (list "log"))
                false)))
  (output (: true Bool)))

; ── MULTI-FILE PACKAGE composition (modules-and-namespaces.md; DESIGN-package-linking.md) ──────────────
; Each case below carries one or more `(module "name" <prog>)` LIBRARY files; the `(input …)` is the
; ENTRY (named `main`). A library's public surface is its `(export …)` list; the entry (or another
; library) reaches it only through an explicit `(import "name" (names…))`.

(case "an imported name resolves to a sibling file's exported definition"
  (doc    "Witnesses modules-and-namespaces.md #Imports Are Explicit: a name defined in another module
           is brought into scope by an explicit import, and a call to it resolves across the file
           boundary into one linked component. `lib` exports `helper` (→ 40); `main` imports and calls
           it, adding 2.")
  (module "lib"
    (do (def (helper) 40) (export helper)))
  (input  (do
            (import "lib" (helper))
            (def (main) (+ (helper) 2))
            (export main)))
  (output (: 42 Int64)))

(case "an unimported sibling definition is not in scope"
  (doc    "Witnesses modules-and-namespaces.md #Imports Are Explicit (2nd sentence: an import introduces
           no names beyond those it names) + #Visibility Is Explicit: WITHOUT an `(import …)`, a sibling
           file's exported name is invisible — referencing it is an unbound-name rejection (CDZ0101),
           not an implicit cross-file resolution.")
  (module "lib"
    (do (def (helper) 40) (export helper)))
  (input  (do
            (def (main) (+ (helper) 2))
            (export main)))
  (error  CDZ0101))

(case "importing a name a module does not export is rejected"
  (doc    "Witnesses modules-and-namespaces.md #Visibility Is Explicit (2nd sentence: a definition not
           made visible is not importable): `lib` defines `helper` and exports only `other`, so
           importing `helper` is rejected — visibility is the export list, not mere definition.")
  (module "lib"
    (do (def (helper) 40) (def (other) 1) (export other)))
  (input  (do
            (import "lib" (helper))
            (def (main) (helper))
            (export main)))
  (error  CDZ0201))

(case "two definitions imported under the same name are rejected"
  (doc    "Witnesses modules-and-namespaces.md #Colliding Imported Names Are Rejected: importing two
           definitions under the same local name into one scope is a compile-time error (CDZ0201),
           never resolved by an implicit precedence.")
  (module "a"
    (do (def (x) 1) (export x)))
  (module "b"
    (do (def (x) 2) (export x)))
  (input  (do
            (import "a" (x))
            (import "b" (x))
            (def (main) (x))
            (export main)))
  (error  CDZ0201))

(case "a cycle of module imports is rejected"
  (doc    "Witnesses modules-and-namespaces.md #Cyclic Module Dependencies Are Rejected: a set of
           modules whose import relationships form a cycle is rejected at compile time (CDZ0201). Here
           the entry imports `lib`, and `lib` imports back from the entry — a dependency loop.")
  (module "lib"
    (do (import "main" (seed)) (def (helper) (seed)) (export helper)))
  (input  (do
            (import "lib" (helper))
            (def (seed) 1)
            (def (main) (helper))
            (export main)
            (export seed)))
  (error  CDZ0201))

(case "an imported helper reaches its own file's private definition when inlined"
  (doc    "Witnesses that linking preserves each file's scope through monomorphization: `lib` exports
           `pub-helper`, whose body calls a PRIVATE sibling `priv-helper` (defined in `lib`, not
           exported, not imported by the entry). When `pub-helper` inlines into `main`, its body's
           reference to `priv-helper` still resolves in `lib`'s scope — cross-file β-copy hygiene.")
  (module "lib"
    (do (def (priv-helper) 40)
        (def (pub-helper) (+ (priv-helper) 1))
        (export pub-helper)))
  (input  (do
            (import "lib" (pub-helper))
            (def (main) (+ (pub-helper) 1))
            (export main)))
  (output (: 42 Int64)))

; --- A sum value crosses a module boundary ---------------------------------------------------------
; core-semantics.md #Sum Types Are Structural Types + modules-and-namespaces.md #Imports Are Explicit:
; a sum is an ordinary value, so an exported function may RETURN one and the importing entry matches it
; exactly as a local sum value. These pin that the sum construct/match machinery composes with linking —
; a variant value built in one file dispatches correctly in another after the exported producer inlines.

(case "a prelude Option value crosses a module boundary as an export result"
  (doc    "`lib` exports `parse` returning an `Option Int64` (`(Some b)` for a positive input); the entry
           imports it and matches the result. The Option value built in `lib` carries its variant tag
           across the link so the entry's `(Some n)` arm binds n = 5. Pins that a prelude sum is an
           ordinary cross-module value — its construction in one file and its match in another compose
           through linking, no special handling for a sum at the boundary.")
  (module "lib"
    (do (def (parse (: b Int64)) (if (> b 0) (Some b) (None))) (export parse)))
  (input  (do
            (import "lib" (parse))
            (def (main) (match (parse 5) ((Some n) n) ((None) 0)))
            (export main)))
  (output (: 5 Int64)))

(case "a recursive user sum built in a lib is folded by the entry over the imported type"
  (doc    "`lib` declares a cons-list sum `L`, exports it CONCRETELY with the wildcard `(. L *)` (the
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
    (do (type L (Nil) (Cons Int64 L))
        (def (mk) (L.Cons 5 (L.Cons 6 (L.Nil))))
        (export (. L *) mk)))
  (input  (do
            (import "lib" (L mk))
            (def (sm (: l L)) (match l ((L.Nil) 0) ((L.Cons h t) (+ h (sm t)))))
            (def (main) (sm (mk)))
            (export main)))
  (output (: 11 Int64)))

(case "a GENERIC user sum crosses a module boundary at a concrete instantiation"
  (doc    "The generic companion of the recursive-sum crossing above: `lib` declares a GENERIC `(type Box
           (W a) (E))` and exports `mk` building `(Box.W 42)` at `a = Int64`; the entry declares its own
           structurally-identical `Box` and matches the imported value, binding the payload at Int64 → 42.
           Pins that a generic user sum composes across the module boundary at a concrete instantiation —
           the crossing value carries its variant + payload exactly as a monomorphic one does, and the two
           modules each declaring `Box` is NOT a duplicate (each module has its own type namespace; the
           duplicate-declaration check is per-module). Both `Box` declarations are user types of the same
           structural shape, so the imported `(Box.W 42)` matches the entry's `(Box.W n)` arm.")
  (module "lib"
    (do (type Box (W a) (E))
        (def (mk) (Box.W 42))
        (export mk)))
  (input  (do
            (import "lib" (mk))
            (type Box (W a) (E))
            (def (main) (match (mk) ((Box.W n) n) ((Box.E) 0)))
            (export main)))
  (output (: 42 Int64)))

(case "a sum TYPE and its constructors are imported by a wildcard and constructed in the entry"
  (doc    "Beyond exporting a sum VALUE (the cases above, where the entry RE-DECLARES a structurally-
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
      (export (. Color *))
      (export to-int)))
  (input  (do
            (import "lib" (Color to-int))
            (def (main) (to-int (Color.Green)))
            (export main)))
  (output (: 2 Int64)))

(case "a wildcard-exported variant whose name shadows a prelude type is constructible in an importer"
  (doc    "The prelude-collision case of the wildcard import above: `lib` declares `(type T (Foo Int64)
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
      (export (. T *))
      (export sz)))
  (input  (do
            (import "lib" (T sz))
            (def (main) (sz (T.List (list))))
            (export main)))
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

(case "an abstract type's constructor is not reachable outside its module"
  (doc    "`lib` exports the type HANDLE `Color` (bare `(export Color)`) and a smart constructor `mk`, but
           NOT `Color`'s variant constructors. The entry imports `(Color mk)` and tries to CONSTRUCT
           `(Color.Green)` directly — reaching a constructor the module kept private. That is rejected
           CDZ0214: `Color`'s handle is visible here (the entry may name the type and hold its values) but
           its constructor `Green` is withheld, so a `Color` value is built only through `mk`. Pins that a
           bare type-handle export is ABSTRACT — the constructor is hidden on purpose, distinct from a
           plain unbound name (the type IS in scope). The fix is to call the module's exported `mk`, or for
           the module to export `Color.*`.")
  (module "lib"
    (do
      (type Color (Red) (Green) (Blue))
      (def (mk) Color.Green)
      (export Color)
      (export mk)))
  (input  (do
            (import "lib" (Color mk))
            (def (main) (Color.Green))
            (export main)))
  (error  CDZ0214))

(case "an abstract type is used through the module's exported constructor and accessor"
  (doc    "The companion of the reject above: the SAME abstract `lib` (handle `Color` + `mk` + `rank`, no
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
  (input  (do
            (import "lib" (Color mk rank))
            (def (main) (rank (mk)))
            (export main)))
  (output (: 2 Int64)))

(case "a specific constructor export exposes one variant and keeps the rest private"
  (doc    "Between fully-abstract and fully-concrete: `lib` exports the handle `Color` plus ONE constructor
           `(. Color Green)`, keeping `Red`/`Blue` private. The entry may construct `(Color.Green)` (the
           exported constructor) — `rank` reads it → 2 — but constructing `(Color.Red)` would be CDZ0214.
           Pins that constructor visibility is per-constructor, not all-or-nothing: `(export (. Color G))`
           publishes exactly the named constructor, the partial point on the abstract↔concrete axis.")
  (module "lib"
    (do
      (type Color (Red) (Green) (Blue))
      (def (rank (: c Color)) (match c ((Color.Red) 1) ((Color.Green) 2) ((Color.Blue) 3)))
      (export (. Color Green))
      (export rank)))
  (input  (do
            (import "lib" (Color rank))
            (def (main) (rank (Color.Green)))
            (export main)))
  (output (: 2 Int64)))

(case "a built-in comparison on an abstract type's value is rejected outside its module"
  (doc    "`lib` exports the HANDLE `Color` (abstract) + a smart constructor `mk`. The entry may name
           `Color` and obtain values via `mk`, but comparing two of them with the built-in `=` observes
           the equality of `Color`'s PRIVATE representation, which the handle-only export withheld — so it
           is rejected CDZ0202 (the nominal-boundary code). A built-in structural comparison is not one of
           the operations a handle-only export publishes; a module that wants its abstract type compared
           exports a comparison FUNCTION (`(def (eq (: x Color) (: y Color)) …)`), the ML discipline —
           the representation stays hidden and only the module's published operations are available.
           Within the declaring module (or a concrete `Color.*` importer) `=` on `Color` is unaffected.")
  (module "lib"
    (do
      (type Color (Red) (Green) (Blue))
      (def (mk) Color.Green)
      (export Color)
      (export mk)))
  (input  (do
            (import "lib" (Color mk))
            (def (main) (= (mk) (mk)))
            (export main)))
  (error  CDZ0202))
