/// The shared ASSERT PRELUDE for `<Runnable mode="test">` examples — `assert` / `assert-eq` / `assert-ne`
/// defined via `trap`, the only failure signal an `@test` needs (a clean return = pass, a `trap` = fail).
/// These are NOT compiler builtins (there is no assert prelude in the language), so a testing example would
/// otherwise have to redefine them; a `mode="test"` Runnable prepends this shared prelude (its `prelude`
/// prop defaults here) so example authors write just their `@test` defs. Semantics confirmed with
/// v-property-testing (any assert helper just has to `trap` on mismatch).
///
/// Per surface: the def SYNTAX differs (s-expr `(def (assert-eq …) …)` vs ML `def assert_eq(…) = …`), the
/// bodies mirror each other. `assertPreludeFor(surface)` returns the right one to prepend.

import type { Surface } from "../compiler/client.ts";

/// The s-expr assert prelude — three defs, ready to prepend (a trailing newline separates it from the
/// example body when concatenated).
export const ASSERT_PRELUDE_SEXPR = `(def (assert (: cond Bool) (: msg String)) (if cond unit (trap msg)))
(def (assert-eq a b (: msg String)) (if (= a b) unit (trap msg)))
(def (assert-ne a b (: msg String)) (if (not (= a b)) unit (trap msg)))
`;

/// The ML assert prelude — the same three helpers in ML surface. Names are KEBAB (`assert-eq`, not
/// `assert_eq`): a Cadenza name is kebab across BOTH surfaces (an s-expr `(def (assert-eq …) …)` renders
/// to ML as `def assert-eq(…) = …`, and `assert_eq` with an underscore is a DIFFERENT name). A test body
/// authored in s-expr and shown in ML calls `assert-eq`, so the ML prelude MUST define `assert-eq` too —
/// an underscore spelling here left every ML @test example failing `CDZ0101 unbound name \`assert-eq\``.
export const ASSERT_PRELUDE_ML = `def assert(cond: Bool, msg: String) = if cond then unit else trap(msg)
def assert-eq(a, b, msg: String) = if a == b then unit else trap(msg)
def assert-ne(a, b, msg: String) = if not (a == b) then unit else trap(msg)
`;

/// The assert prelude for a surface — prepended to a `mode="test"` example's source so the example's
/// `@test` defs can call `assert`/`assert-eq`/`assert-ne` without redefining them.
export function assertPreludeFor(surface: Surface): string {
  return surface === "ml" ? ASSERT_PRELUDE_ML : ASSERT_PRELUDE_SEXPR;
}
