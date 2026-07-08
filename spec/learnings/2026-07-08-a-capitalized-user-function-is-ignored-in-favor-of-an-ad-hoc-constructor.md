# A capitalized user function is ignored in favor of an ad-hoc constructor

*2026-07-08*

**What happened.** Adversarial probing of binding/shadowing found that a user-defined function whose
name is capitalized is silently ignored: the compiler treats a capitalized name in call position as an
ad-hoc constructor and never resolves the user's `def` binding. `(module m (def (Foo x) (+ x 1)) (def
(main) (Foo 10)))` returns `(Foo 10)` — the constructor value — instead of `11`, the function's result.
The user's `Foo` (computing `x + 1`) is bypassed entirely. The nullary form `(def (Foo) 5)` then `(Foo)`
returns `(Foo unit)` instead of `5`. The lowercase companion `(def (bar) …)` is called correctly, so
the sole determinant is capitalization. The same override hits a built-in module name — `(def (List) 5)`
then `(List)` returns `(List unit)`, and `(def (String) 42)` then `(String)` returns `(String unit)`.

**Why it is a break.** core-semantics.md #Binding Is Lexical: "A name MUST resolve to the nearest
enclosing binding of that name." A `(def (Foo x) …)` binds `Foo` in the module's scope (#A Module Binds
Its Name In Its Enclosing Scope; each definition registers its name), so `(Foo 10)` MUST resolve to that
binding and invoke it, yielding `11`. `Foo` is not a variant of any declared sum type, and even if a
same-named variant existed, the user's `def` is the nearest binding. Returning `(Foo 10)` is a wrong
value — the function is bypassed — contradicting #Binding Is Lexical, and for the `List`/`String` cases
also core-semantics.md line 225 ("the language MUST NOT recognize a built-in module's name in any
position a program-defined module's name would not be recognized"). Capitalization is not a
binding-precedence rule; the spec's resolution order is lexical nearest-binding, not name-shape.

**Root cause (likely) — the constructor fallback fires before the user-binding lookup.** In call
position, the seed appears to classify a capitalized head name as a constructor (synthesizing a tagged
`(Foo <arg>)` / nullary `(Foo unit)`) before consulting the module's own `def` bindings. So a user `def`
of a capitalized name never shadows the constructor interpretation: the fallback wins unconditionally.
The prelude MUST bind constructor values only for declared sum-type variants (core-semantics.md #A Sum
Type Constructor …, "The prelude MUST bind Constructor values only for sum type variants"), so a
capitalized name that is NOT a declared variant is not a constructor — it is either a user binding (call
it) or unbound (CDZ0101). The fix is to resolve a call's head against the lexical environment (user
`def`s, `let`s, params, then declared constructors) FIRST, and treat a capitalized name as a constructor
only when it actually names a declared variant — never as a blanket fallback that overrides a user
binding.

**The lesson.** A syntactic shortcut — "an uppercase head is a constructor" — was applied as an
unconditional classification rather than a last resort after lexical resolution, so it silently overrides
a real binding. Name RESOLUTION must be lexical-scope-first (nearest binding wins), and a "this looks
like a constructor" heuristic must be gated on the name actually being a declared variant AND no nearer
binding shadowing it. The tell: the identical `def` is honored for a lowercase name and ignored for an
uppercase one — the resolver branched on capitalization instead of on what is bound. (Adjacent, left
unpinned as a separate design question: whether `(Foo 10)` for a wholly-undeclared, unbound `Foo` should
be CDZ0101 rather than a synthesized open constructor — the seed currently answers `(Foo 10)`; this case
pins only the unambiguous half, that a USER-BOUND capitalized name must be called.)

**Corpus case added.** `spec/semantics/09-functions.sexp` §"a function whose name is capitalized is
called, not treated as a constructor" — `(module m (def (Foo x) (+ x 1)) (def (main) (Foo 10)))` MUST
output `11`. Native seed; the behavior gate catches it (expected `11`, observed `(Foo 10)`). A generation
that does not resolve a capitalized name to its user binding declines rather than answering `(Foo 10)`.
