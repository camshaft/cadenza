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
; Multi-module composition — explicit imports, visibility, cyclic-dependency rejection, deterministic
; initialization order, colliding-import rejection (modules-and-namespaces.md) — is deferred beyond a
; single module AND has no pinned surface form in the core symbol table yet, so it is intentionally not
; witnessed here; cases arrive with the generation that realizes it. Cases with no (needs …) are core
; (the seed runs them); those comparing against a built manifest list carry (needs collections).

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

(case "a module's declared capability is reachable as metadata, not as an export"
  (doc    "Witnesses core-semantics.md #A Module Carries Its Manifest And Entry As Metadata:
           the capabilities are reached by the (meta …) key, distinct from the export
           namespace, so they never collide with an export. The module imports and declares the
           host function `log`; its capabilities metadata contains \"log\".")
  (needs  collections)
  (input  (do
            (module m
              (import (host log (func (String) unit)))
              (use (capability log))
              (def (answer) 42))
            (= (. m (meta capabilities)) (list "log"))))
  (output (: true Bool)))

(case "a declared capability is not itself an export field"
  (doc    "Witnesses core-semantics.md #A Module Carries Its Manifest And Entry As Metadata (1st
           sentence): a declared capability is carried as metadata SEPARATE from the exported fields,
           so it is not itself an export. The module declares the host function `log` but exports only
           `answer`; projecting `log` as an export field finds no such field and traps (the capability
           lives in `(meta capabilities)`, witnessed by the case above), rather than resolving to the
           manifest or to the host function.")
  (input  (do
            (module m
              (import (host log (func (String) unit)))
              (use (capability log))
              (def (answer) 42))
            (. m log)))
  (trap   "no such field"))

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
           cannot collide with an export. This module both declares the host function `log` AND
           exports a definition literally named `capabilities`. The export `(. m capabilities)` resolves
           to that definition (applied, it yields 7), while `(. m (meta capabilities))` resolves to the
           manifest — the same spelling in the two channels denotes two different things, which is the
           whole reason metadata lives behind (meta …).")
  (needs  collections)
  (input  (do
            (module m
              (import (host log (func (String) unit)))
              (use (capability log))
              (def (capabilities) 7))
            (if (= ((. m capabilities) unit) 7)
                (= (. m (meta capabilities)) (list "log"))
                false)))
  (output (: true Bool)))
