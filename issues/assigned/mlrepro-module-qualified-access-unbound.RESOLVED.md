# Module-qualified access `Temp.c-to-f` fails: `CDZ0101 unbound name Temp`

**Reported by operator, 2026-07-15, via concierge.**

Calling a module member with dotted access from outside the module is rejected as if the
module name were an unbound value. Modules appear "a bit broken" from the user's seat.

## Repro (ML surface)

```
module Temp {
  def c-to-f(c) = c * 9 / 5 + 32

  export { c-to-f }
}

def main() = Temp.c-to-f(100)

export { main }
```

Expected: `main` type-checks and runs to `212` (100°C → 212°F).

Actual (`target/release/cdz check`):

```
/tmp/mod-repro.cdz:7:14: error [CDZ0101]: unbound name `Temp`
```

`cdz type main` → `unknown`; `cdz exports` → `main : unknown`.

## What I confirmed (to scope the fix)

- **Parse is clean.** `cdz convert --to sexpr` yields exactly what you'd want:
  ```
  (do
    (module Temp (def (c-to-f c) (+ (/ (* c 9) 5) 32)) (export c-to-f))
    (def (main) ((. Temp c-to-f) 100))
    (export main))
  ```
  So the module form, its body, its `(export c-to-f)`, and the call site
  `(. Temp c-to-f)` all parse correctly. This is a **name-resolution** bug, not a syntax bug.

- The failure is on the **dotted access** `(. Temp c-to-f)`: `Temp` is resolved as an ordinary
  value name (→ unbound) rather than as a module whose member `c-to-f` should be selected.
  The `(. <module> <member>)` path either isn't wired to look modules up in scope, or a
  top-level `(module …)` inside a `(do …)` block isn't registering `Temp` as a resolvable
  module binding in the enclosing scope.

- **Intra-module refs are fine.** A sibling `def g() = c-to-f(100)` inside the same module
  checks clean — only the *cross-module qualified* reference breaks.

## Likely area
Resolver handling of `Core::` module bindings + the `(. head member)` selection form when
`head` names a module (vs a record/value). Compare against how cross-*file* imports resolve
qualified names (those reportedly work — `spec@aff30766`), and against record field access
`(. r field)`, which shares the `.` node.

## Acceptance
- The repro above checks clean and runs to `212`.
- Add a corpus case (module-qualified call from `main`) as a regression guard.
- Confirm intra-module refs and record field access still resolve (no `.`-node regression).

;; RESOLVED 2026-07-15 (trunk@2ac25eab): fix landed, gate PASSes. Agent self-removed.
