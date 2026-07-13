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
; Cases with no (needs …) are core (the seed runs them); those comparing against a built manifest list
; carry (needs collections).
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
           `fac` calls itself; fac(5) = 120. A recursive export is the same lexical resolution as a
           top-level recursive def, which already works.")
  (input  (do
            (module lib
              (def (fac n) (if (= n 0) 1 (* n (fac (- n 1))))))
            ((. lib fac) 5)))
  (output (: 120 Int64)))

(case "a module's delegated capability is reachable as metadata, not as an export"
  (doc    "Witnesses core-semantics.md #A Module Carries Its Manifest And Entry As Metadata:
           the capabilities are reached by the (meta …) key, distinct from the export
           namespace, so they never collide with an export. The module declares the routing-agnostic
           effect `log` and its entry `main` DELEGATES it to the host with `(host (log) …)`; the manifest
           is the union of the entry's delegations, so the capabilities metadata contains \"log\" (the
           delegation — not the declaration — is the grant, capabilities-and-effects.md #The Program
           Manifest Is The Union Of Its Entrypoints' Delegations).")
  (needs  collections)
  (needs  effects)
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
  (needs  effects)
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
; today defines one key, `default-integer` (its behavior witnessed in 06-numeric-model.sexp under
; `needs numeric-model`); these cases pin the general mechanism. `(needs module-pragmas)`: the pragma
; channel is realized by a later generation, so the seed's gate skips these — they pin the contract.

(case "an unrecognized pragma key is rejected rather than ignored"
  (doc    "`(pragma frobnicate 3)` names a key the pinned registry does not define, so the module is
           REJECTED (CDZ0601, modules-and-namespaces.md #An Unrecognized Module Directive Is Rejected),
           not silently ignored. THE reason the channel is strict: a dropped meaning-changing directive
           would make one source mean two things on two toolchains. The general-mechanism companion of
           the numeric `default-integer` cases.")
  (needs  module-pragmas)
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
  (needs  module-pragmas)
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
  (needs  collections)
  (needs  effects)
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
