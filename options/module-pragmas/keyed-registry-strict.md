# Module Pragmas — Choice: keyed-registry-strict

> **The default choice for the `module-pragmas` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It pins the concrete `(pragma …)` surface and
> the registry of pragma keys that realizes the modules capability's requirements that a module
> directive be drawn from a fixed set, that an unrecognized directive be rejected rather than ignored,
> that a meaning-changing directive be part of the canonical form, and that every directive be
> compile-time only.
>
> A pragma is resolved at compile time and then erased, so it introduces nothing into the emitted
> component — tuning this choice does not alter any component's bytes.

## The form

A module directive is written at the top of a module as

```
(pragma <key> <arg>…)
```

where `<key>` is a **bare identifier** naming a directive from the pinned registry below, and the
`<arg>…` are the arguments that key defines. `pragma` is a module-level form, alongside `def` and
`effect`; a program writes its pragmas before its definitions.

```
(module crypto
  (pragma default-integer BigInt)     ; bare literals in this module are BigInt
  (def (double x) (* x 2)))
```

## The rule that makes it safe: unknown is rejected, never ignored

The one non-negotiable property, and the reason this is **not** C's `#pragma`:

- **An unrecognized key is a compile-time error (`CDZ0601`).** `(pragma frobnicate 3)` — a key the
  registry does not define — is **rejected**, not ignored (modules-and-namespaces.md §"An Unrecognized
  Module Directive Is Rejected"). A directive that some toolchain silently dropped could make one
  source compile to two meanings; rejecting the unknown key makes that impossible. Ignoring directives
  "can lead to bugs and miscompilations" — so the channel never ignores one.
- **Malformed arguments are a compile-time error (`CDZ0602`).** A recognized key with the wrong number
  or kind of arguments — `(pragma default-integer)` (missing), `(pragma default-integer BigInt Int64)`
  (too many) — is rejected against the shape the key defines. (A *well-formed* directive whose argument
  violates a *domain* rule carries that domain's code instead — a `default-integer` naming a non-integer
  type is the numeric model's `CDZ0303`, not `CDZ0602`, because the argument is a syntactically valid
  type that fails an integer-domain predicate.)
- **A meaning-changing pragma is part of the canonical form.** `default-integer` changes what type the
  module's literals take, so it is carried in the module's canonical binary form (constitution §X): the
  module's meaning is fixed by its canonical form alone, never by a compilation flag outside it. Two
  builds of the same canonical module resolve every pragma identically.
- **Every pragma is compile-time only.** A pragma is resolved during compilation and then types erase;
  it adds no runtime representation and crosses no boundary (modules-and-namespaces.md §"A Module
  Directive Is Compile-Time Only"). A module compiles to the identical bytes it would if the pragma's
  effect had been written out explicitly at every site.

## The pinned pragma registry

A pragma key is defined here — the declared-default location — so that two builds give the same key the
same meaning and an unknown key has a fixed, rejected status. A new directive is a **new key added here
by a governed act**, exactly as a new diagnostic is a new code in `options/diagnostics-schema/`; it is
never invented per program. Each key fixes its argument shape, its meaning, and whether it changes the
module's meaning (and so must be in the canonical form).

| Key | Arguments | Meaning | Meaning-changing? | Realizes |
|---|---|---|---|---|
| `default-integer` | one integer type `<T>` | within this module, an integer literal with no other constraint takes `<T>` instead of the numeric model's default `Int64`; fixes a type, never a conversion; definition-site scoped | **yes** — carried in the canonical form | numeric-model.md §"Default Literal Type"; `options/numeric-model/` |

The registry opens with the one key the numeric-model work needs. The band and the mechanism are built
so later keys drop in without reshaping anything — candidates the language may add by the same governed
act (each would state its argument shape and whether it is meaning-changing):

- a default for another literal kind (e.g. a default float type), by the same definition-site,
  fixes-a-type-not-a-conversion discipline;
- a lint or warning level for a module, which — being **advisory**, not meaning-changing — would *not*
  be part of the canonical form, though an **unknown** lint key is still rejected;
- a module-wide verification-layer toggle consistent with verification-layers.md's optionality.

Whatever is added, the two invariants hold: the key comes from this registry (unknown → `CDZ0601`), and
a meaning-changing key is in the canonical form.

## `default-integer` in detail

The one registered key today. `(pragma default-integer <T>)` makes an otherwise-unconstrained integer
literal in the module take `<T>`. Its full semantics are pinned in `options/numeric-model/`
(§"Default integer literal type"); the load-bearing points, restated so the pragma's contract is
self-contained:

- **Fixes a type, not a conversion.** It changes only what type an unconstrained literal *starts as*;
  every no-silent-promotion rule then applies unchanged (numeric-model.md §"A Declared Default Fixes A
  Type, Not A Conversion"). In a `default-integer BigInt` module, `(+ 2 someInt64)` is still `CDZ0301`
  — the pragma made `2` a `BigInt`, it did not add a coercion.
- **Definition-site scoped.** The default in force for a literal is the one the *module the literal is
  written in* declares, never a module that imports it (numeric-model.md §"A Declared Default Applies At
  The Definition Site"). Importing a module never changes the type of code inside it.
- **An explicit constraint wins.** An annotation on a literal (`(: 5 Int64)`) overrides the module
  default; the pragma only decides the otherwise-unconstrained case.
- **Argument domain check.** `<T>` must be an integer type the numeric model admits; a non-integer
  `<T>` (`(pragma default-integer Float64)`) is `CDZ0303` — the numeric-domain rejection, distinct from
  the structural `CDZ0602`.
