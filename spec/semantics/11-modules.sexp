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

(case "a module's declared capability is reachable as metadata, not as an export"
  (doc    "Witnesses core-semantics.md #A Module Carries Its Manifest And Entry As Metadata:
           the capabilities are reached by the (meta …) key, distinct from the export
           namespace, so they never collide with an export. The module declares emit-event;
           its capabilities metadata contains \"emit-event\".")
  (needs  collections)
  (input  (do
            (module m
              (use (capability emit-event))
              (def (answer) 42))
            (= (. m (meta capabilities)) (list "emit-event"))))
  (output (: true Bool)))

(case "a declared capability is not itself an export field"
  (doc    "Witnesses core-semantics.md #A Module Carries Its Manifest And Entry As Metadata (1st
           sentence): a declared capability is carried as metadata SEPARATE from the exported fields,
           so it is not itself an export. The module declares emit-event but exports only `answer`;
           projecting `emit-event` as an export field finds no such field and traps (the capability
           lives in `(meta capabilities)`, witnessed by the case above), rather than resolving to the
           manifest or to the host operation.")
  (input  (do
            (module m
              (use (capability emit-event))
              (def (answer) 42))
            (. m emit-event)))
  (trap   "no such field"))

(case "an export and a like-named metadata key do not collide"
  (doc    "Witnesses core-semantics.md #A Module Carries Its Manifest And Entry As Metadata (2nd
           sentence): metadata is reached by a key distinct from every export name, so metadata access
           cannot collide with an export. This module both declares the emit-event capability AND
           exports a definition literally named `capabilities`. The export `(. m capabilities)` resolves
           to that definition (applied, it yields 7), while `(. m (meta capabilities))` resolves to the
           manifest — the same spelling in the two channels denotes two different things, which is the
           whole reason metadata lives behind (meta …).")
  (needs  collections)
  (input  (do
            (module m
              (use (capability emit-event))
              (def (capabilities) 7))
            (if (= ((. m capabilities) unit) 7)
                (= (. m (meta capabilities)) (list "emit-event"))
                false)))
  (output (: true Bool)))
