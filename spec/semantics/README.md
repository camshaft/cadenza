# Executable Semantics

This directory is the **single source of truth for what every Cadenza construct does**. It is
normative *by execution*, not by RFC-2119 extraction: it carries no MUST sentences and is not listed
in the duvet requirement gate. Its gate is the **behavior gate** — every case here must execute to
its recorded output on a promoted compiler (see
[capabilities/conformance-gate.md](../capabilities/conformance-gate.md) §"The Behavior Gate" and
[capabilities/compiler-pipeline.md](../capabilities/compiler-pipeline.md) §"The Behavior Gate").

This corpus exists because earlier Cadenza let the meaning of the language live in several places at
once — an interpreter, a separate document, a generated implementation, and a formal model — which
drifted apart (see [learnings/2026-07-02-parallel-semantics-drifted.md](../learnings/2026-07-02-parallel-semantics-drifted.md)).
There is now one place a construct's meaning lives, and it is runnable. This corpus itself is the
behavioral oracle: its recorded results are the authority the compiler and every tool agree with. A
reference interpreter (see
[capabilities/self-hosting-and-bootstrap.md](../capabilities/self-hosting-and-bootstrap.md)) is an
optional, independent realization that MAY cross-check those results, not the oracle.

## The form: s-expression cases

Each case is an **s-expression**, so the whole corpus is parseable by a minimal reader — the seed
toolchain needs only an s-expression reader to run the behavior gate, not the full surface parser.
This is deliberately the easiest thing to bootstrap. Cases live in `NN-feature.sexp` files, one
feature per file.

A case is a small fixed test-DSL vocabulary wrapping program fragments that are themselves written in
Cadenza's **canonical homoiconic representation** (see [`options/code-shape/`](../../options/code-shape/)):

```
(case "integer addition"
  (input  (+ 2 3))
  (output (: 5 Int64)))

(case "no implicit promotion between numeric types"
  (input    (+ 2 2.0))
  (trap     "numeric type mismatch")
  (compiler (error CDZ0301)))

(case "a documented case"
  (doc    "Notes for humans and agents; part of the case, not stripped.")
  (input  (let ((x 10)) x))
  (output (: 10 Int64)))
```

### The test-DSL vocabulary

Each case has one `input`, one **primary result clause** (the recorded behavior of the one executable
semantics, which is the oracle), and optional annotations. The corpus is **one flat set**: differences
between generations are annotated *inline* rather than split into separate files, so there is exactly
one place a construct's meaning lives.

- `(case "<description>" <clause>...)` — one case; the description is a short human/agent-readable label.
- `(input <program>)` — the program to run, in the canonical representation.
- `(doc "<text>")` — optional prose attached to the case; documentation, never affecting the check.
- `(call <export> <arg>...)` — optional; run the program's `<export>` entry with the given **runtime
  arguments** rather than as a nullary entry. Each `<arg>` is a `(: <value> <Type>)` value-form supplied
  to the entry from outside the component; the entry's exported signature is then `input -> output`, its
  parameter types read from its declared signature (contracts/component-abi.md §"The Entry Is A Plain
  Function", §"The Exported Interface Is The Declared Signature"). This is the case's channel for a
  value that arrives at run time — distinct from `host-responses`, which fixes the returns of host calls
  the program *makes*. A value supplied here cannot be constant-folded, so a `(call …)` case exercises
  the emitted component's runtime machinery (a parameter crossing the boundary, an operation over it
  running as a real instruction) that a nullary entry, whose body folds to a value at compile time, never
  reaches. Omitted for the common nullary case, where the sole export is run with no arguments. A
  parameter that crosses the boundary MUST be annotated (`(: x Int64)`): its boundary representation
  follows its declared type, so an unannotated parameter has no boundary form and the compiler declines.

  **A case may pair SEVERAL `(call …)`s, each with its own result** — exercising one program at several
  runtime arguments without duplicating the whole case. Write the pairs interleaved: each `(call …)` is
  immediately followed by the result clause it produces (an `(output …)`/`(trap …)`/`(error …)`), e.g.
  `(call main (: true Bool)) (output (: 1 Int64)) (call main (: false Bool)) (output (: 2 Int64))`. The
  program is compiled ONCE and run once per call; the case passes iff EVERY pair matches (a failing pair
  is reported with its call). Prefer this to two near-identical cases that share an `(input …)` and
  differ only in the argument. Distinct results per call are fine — one may `(output …)` and another
  `(trap …)` (e.g. a shift that fits for one operand and overflow-traps for another).

**Primary result clause — one per call (usually exactly one), the oracle.** This is the recorded result
the corpus fixes for `input` (or for each `(call …)`); every generation that runs the case reproduces
it, and the corpus's recorded value — not any one implementation — is the authority (constitution §IX;
§XIV). A case with no `(call …)`, or a single call, has exactly one; a case with several `(call …)`s has
one result clause after each (see the `(call …)` clause above). Usually a *terminal clause* — the
outcome of running the program:
- `(output <value-form>)` — the value the run produces on normal termination.
- `(trap "<reason>")` — the run halts at a defined point with this reason (for example, a checked overflow).

For a program the compiler **rejects at compile time** rather than runs — whether the rejection needs
no type system (an unbound name, core-semantics.md §"Binding Is Lexical", or an undeclared capability,
capabilities-and-effects.md §"Undeclared Capability Is A Compile-Time Error") or is a type rejection (a
nominal-boundary comparison, a numeric mismatch, a non-exhaustive match, a contradicting annotation) —
the primary clause is instead:
- `(error <CODE>)` — the diagnostic code the rejection carries (from the pinned registry,
  [`options/diagnostics-schema/`](../../options/diagnostics-schema/)). A rejection is the program's
  recorded outcome: an ill-typed or ill-formed program has no run and therefore no terminal value, so
  the corpus records the rejection itself rather than what some evaluator might have produced had the
  program run. Cadenza has one implementation kind — a compiler — so there is no second, dynamic
  outcome to record.
  - Diagnostic **prose** may be pinned alongside the code, on `(error …)`, `(declines …)`, and
    `(warning …)` (and the `(warns …)` presence clause): `(message "<phrase>")` requires the emitted
    diagnostic to CONTAIN `<phrase>` — repeatable, ALL required (AND). Its complement `(not "<phrase>")`
    requires the diagnostic to NOT contain `<phrase>` — also repeatable, ALL required (a message-ABSENCE
    assertion, e.g. that a user-facing decline does not leak an `"internal error"` phrase). Positive and
    negative pins compose: `(declines CDZ0900 (message "not yet") (not "internal error"))`. (These prose
    pins are graded on the sexp `test-run.ast` path; the flat direct-gate manifest checks code + a single
    positive phrase.)

**Observation clause — optional.**
- `(host-calls <call>...)` — the exact ordered sequence of host calls the run makes, each `<call>`
  written `(call <fn> <arg-value-form>...)` where `<fn>` names a performed operation `<name>.<op>` that
  an entrypoint delegated to the host; part of observable behavior (core-semantics.md §"Host Calls Are
  Ordered And Part Of Observable Behavior"). `(host-calls)` asserts none was made. Which host functions
  exist is the target's concern, not the language's; a case declares whatever WIT-typed operation it
  needs as a routing-agnostic effect and delegates it at the entrypoint (see the `(host …)` form below).

**Host-response fixture — optional.**
- `(host-responses <respond>...)` — the response each host call returns, supplied in call order, each
  `<respond>` written `(respond <fn> <value-form>)`. Because a run's behavior is a deterministic
  function of its input **and the responses to its host calls**, a case whose program consumes a host
  call's return value fixes those responses here so the recorded result is reproducible. A host call
  whose WIT signature returns unit needs no `respond`. This fixture is the ordered response sequence the
  run's determinism is defined against (capabilities-and-effects.md §"A Run Is A Deterministic Function
  Of Its Input And Responses"): every generation feeds the responses in order and reproduces the recorded
  terminal — a determinism assertion, independent of how a host resolves the calls.

**Declaring a host function and delegating it.** A program that makes a host call declares the function
as a **routing-agnostic effect** — the single surface for every effect, host or intra-program (there is
no separate import form): `(effect <name> (op <op> (-> <param-type>... <result-type>)))` inside a module.
The declaration says nothing about where the effect is discharged. An **entrypoint** then routes it to
the boundary with a `(host (<name>...) <body>)` delegation, the boundary counterpart of `(handle …)`:
within `<body>` the named effects are discharged at the component boundary as plain imported-function
calls the host resolves, and enumerated in the manifest, and the complete WIT-typed signature on the
operation lets the compiler emit
the import into the component's world (host-interface-binding.md §"A Host Import Is A WIT-Typed Function
The Manifest Enumerates"). An operation is performed as `<name>.<op>`, and reaching an operation with
neither an enclosing handler nor an enclosing delegation is `CDZ0401` (the single "no home" check); a
delegation naming an effect never reached is `CDZ0404` (latent authority). The delegation **is** the
manifest grant — there is no separate `(use (capability …))` form; the manifest is the union of the
entrypoints' host delegations (capabilities-and-effects.md §"An Effect Is Routed By A Handler Or By Host
Delegation"). In `(host-calls …)`, `<fn>` names the performed operation `<name>.<op>`.

**Incremental realization.** A `(error <CODE>)` primary is the outcome every generation must
eventually produce, but the static-typing floor is realized **incrementally** over the type rules a
generation's compiler covers (constitution §VII; Amendment 0.4.0). For a rule it does not *yet* cover,
a generation MUST **decline** — refuse to derive a component — rather than run the ill-typed program to
a wrong value (reject-don't-miscompile,
[`spec/learnings/2026-07-03-decline-do-not-miscompile.md`](../learnings/2026-07-03-decline-do-not-miscompile.md)).
The differential gate treats a decline as *todo*, not as disagreement, so a green gate still means
every program a generation *does* compile agrees with the recorded outcome. There is no separate
"dynamic" outcome recorded alongside the rejection: the `(error <CODE>)` clause is the whole story.

**Generation divergence — expressed by the DECLINE mechanism, not a tag.** A case whose input a
generation does not yet realize is not skipped: it is compiled and run, and the compiler's own
**decline** (reject-don't-miscompile) scores it *todo*. The former `(needs <capability>)` annotation —
which pre-empted the run — has been retired: the decline mechanism already expresses "todo" correctly
and automatically, decided by the compiler rather than hand-annotated, and running the case keeps both
its pass-guard (when the generation does realize it) and its regression-catch (a wrong value / dropped
trap). So there is no generation-divergence tag; every case runs on every generation.

The result value form is `(: <value> <Type>)` — a value paired with its type — serialized under the
canonical value form ([`contracts/deterministic-value-form.md`](../contracts/deterministic-value-form.md)),
so a case's expected output is byte-exact. A case that carries no `(compiler …)` clause is one every
generation reproduces from the recorded oracle (or declines → todo) — the common case, and the concrete
meaning of "a well-typed program does not go wrong."

## Authoring rules

- **A case is executable.** Every case must be runnable — by a compiled component, and optionally by a
  reference interpreter — and carry a definite primary result clause — a terminal clause (`output`,
  `trap`) or a front-end `error` (unbound name or undeclared capability) — optionally with
  a `host-calls` observation, a `host-responses` fixture, and an inline `(compiler …)`
  annotation; a case with no definite primary result is not a case.
- **A case covers one behavior.** Prefer many small cases over one large program, so a behavior-gate
  failure names the construct that broke.
- **The corpus is complete per realized capability.** Every behavioral requirement of a capability a
  generation *realizes* is witnessed by at least one case that generation runs, so its behavior gate
  exercises what its requirement gate cites (conformance-gate.md §"A Generation Is Judged Against The
  Capabilities It Realizes").
- **Determinism is part of the check.** A case's output is byte-exact; a construct whose output could
  vary is either given a deterministic specification or is not admitted.
- **Bound a scalar-indexed String walk by `String.scalar-len`, never `String.byte-len`.** `String.at`
  and `String.slice` are SCALAR-indexed (per the prelude); a loop/recursion that walks a `String` with
  `String.at s i` / `String.slice s a b` must seed its bound from `String.scalar-len s`, not
  `String.byte-len s`. For a multibyte scalar `byte-len > scalar-len`, so a byte-len bound drives the
  index PAST the last scalar into `String.at`'s `(None …)` arm — dropping the final element, returning a
  sentinel, or (under `Option.expect`) HARD-TRAPPING — even though the case passes on ASCII inputs where
  the two lengths coincide. `String.byte-len` is correct ONLY for a `Bytes.at`/`Bytes.slice` byte-indexed
  walk (Bytes is byte-addressed) or as a plain output measurement never re-consumed as a `String.at`
  index. A multibyte input (e.g. one containing `é`) makes the distinction observable; prefer one when a
  case's framing is a "scalar walk". (This class has recurred repeatedly — paren-scan/split, parse-int,
  LCP, run-length, word-count, roman/hex decoders, one-edit/rotation/Levenshtein — each latent behind
  ASCII inputs; see the byte-len→scalar-len fixes in `13-strings.sexp`/`10-bytes.sexp`.)

## Which cases a generation runs

A generation's behavior gate runs **every** case — there is no per-case skip filter. A generation that
does not yet realize what a case exercises **declines** it (reject-don't-miscompile), and the gate
scores that decline as *todo*, not disagreement — so the compiler itself decides "this generation
doesn't do it yet," per case, at run time (conformance-gate.md §"A Generation Is Judged Against The
Capabilities It Realizes"):

- A `(output …)` / `(trap …)` primary clause is the recorded result every generation that COMPILES the
  program must reproduce; one that cannot yet compile it declines → todo.
- An `(error <CODE>)` primary clause is the rejection every generation that covers the relevant rule
  must produce; a generation that does not yet cover the rule **declines** (reject-don't-miscompile)
  and the gate scores that as todo, not disagreement. So mixed-type arithmetic `(+ 2 2.0)` records the
  single outcome `(error CDZ0301)` — the rejection is the behavior, not a footnote to a value the
  program never produces.

The **seed** is a compiler that realizes the static-typing floor incrementally
(constitution §VII; Amendment 0.4.0;
`../learnings/2026-07-04-static-typing-is-mandatory-post-pivot.md`). It thus runs every
case: producing the `(error <CODE>)` rejection where a case records one for a rule it
covers, reproducing the terminal clause otherwise, and enforcing the capability floor. It
realizes lowering, binding, control flow, matching, structural equality, the static-typing floor,
runtime traps that survive type-checking (overflow, division by zero, index out of bounds),
observable behavior, and the primitive value forms — and it declines (rather than miscompiles) a
construct it does not yet realize or a type rule it does not yet cover.

## Files

The corpus is organized by feature, numbered for a natural reading order — one flat set; generation
differences are inline annotations, not separate files. It grows as capabilities are specified.

- `01-literals.sexp` — literals and their types
- `02-binding-and-control.sexp` — lexical binding, shadowing, `do` sequencing (in-order evaluation, last-form value), conditionals, pattern bindings, unbound-name rejection
- `03-equality-and-observation.sexp` — structural/float equality, ordering (including the three-way `compare` yielding the `Ordering` sum), ordered host calls
- `04-capabilities.sexp` — the mandatory capability floor: an entrypoint delegates routing-agnostic effects to the host with `(host …)`, reaching an effect with no handler and no delegation is rejected (`CDZ0401`), delegating an unreached effect is rejected (`CDZ0404`), an empty delegation is pure, and a response-returning host call fixes its response with `(host-responses …)`
- `05-compound-types.sexp` — records, sum types, lists, maps; member access (the `.` accessor: field read, non-record trap, missing-field trap); structural equality (runtime) with `(compiler …)` for the static nominal/structural and exhaustiveness rejections
- `06-numeric-model.sexp` — checked `Int64` core; `(compiler …)` for compile-time no-promotion; rational/wrapping/floating-point arithmetic a later generation realizes
- `07-type-system.sexp` — annotation-vs-inference and ill-typedness, as `(compiler …)` rejections the typed compiler makes; the primary clause records what an untyped dynamic evaluator would produce; plus the `Never` empty-sum surface (a diverging expression unifying with any type, a zero-arm match on an uninhabited scrutinee — the `Never` surface a later generation realizes)
- `08-self-hosting-surface.sexp` — reader/printer round-trip over a program's AST, the self-hosting surface a later generation realizes (no `eval`: the compiler needs AST construction, not execution)
- `09-functions.sexp` — first-class functions and closures: `fn` values, application, closure capture, higher-order functions, and recursion (core; the seed realizes these)
- `10-bytes.sexp` — the `Bytes` byte-sequence value form (construction, equality, length, concatenation, out-of-range construction trap), the `bytes` capability; the seed realizes it so the Cadenza-authored compiler can build a component's wasm bytes as an ordinary value (bootstrap.md §"The Self-Hosted Compiler Is Authored In Cadenza"; `options/realized-capability-set/`). Its indexing and slicing are fallible (Option-returning) — the `fallible-access` capability the seed does not yet realize (collections-and-text.md §"Indexing And Lookup Are Fallible, Not Trapping")
- `11-modules.sexp` — single-module semantics: a module declaration binds its name in the enclosing scope (used via a `do` block, no `let` wrapping) to a record of its exports (each `def` a reachable export field), and carries its capability manifest and entry as metadata reached by a `(meta …)` key distinct from every export, so a declared capability is not an export and a like-named export and metadata key do not collide (core-semantics.md §Modules); AND multi-file PACKAGE composition — explicit `(import "path" (name…))`, per-file visibility, cyclic-import and colliding-import rejection (`CDZ0201`), a sum TYPE imported across the link. OPAQUE (abstract) types: a type declaration's handle and its constructors are independently exportable — `(export T)` publishes the HANDLE ONLY (abstract: an importer may name `T` and hold its values but constructing or matching its variants is `CDZ0214`, the constructor is withheld), `(export (. T *))` the wildcard publishes the handle + ALL constructors (concrete), and `(export (. T A))` publishes the handle + exactly the named constructor (partially concrete), so a module enforces a type's invariant through private constructors + exported smart constructors (modules-and-namespaces.md §Visibility Is Explicit)
- `12-metaprogramming.sexp` — quote/quasiquote as AST *construction* (the core cases the seed runs: quote produces an AST sum value, quasiquote embeds `,`-unquotes and splices `,@`, and a quote-built AST equals and encodes identically to the same tree built by the `Ast.*` constructors), and the *pattern*-position dual — quote patterns destructuring an `Ast` scrutinee (`` `(+ ,a ,b) `` lowering to the `Ast.*` constructor patterns, fixed arity, final-position `,@`, reused exhaustiveness `CDZ0210`, ill-formed non-final splice `CDZ0221`), the `quote-patterns` capability a later generation realizes (`options/quote-patterns/`); `eval` (executing an AST) is the optional `eval` affordance, not part of the self-hosting surface
- `14-effects-and-handlers.sexp` — routing-agnostic effect declarations routed either by a lexical `(handle …)` or an entrypoint `(host …)` delegation: a delegated call is a plain imported-function call the host resolves (a run is deterministic in its input and ordered responses; resumption is host policy), an intra-program effect discharged by a handler that does not escape to the manifest, a handler interposing on a delegated effect and forwarding to the boundary, one-shot continuations, and purity as the empty row; the `effects` capability a later generation realizes
- `15-rows-and-open-sums.sexp` — row-polymorphic open records (a function open over a record's extra fields; subset comparison as explicit projection), the explicit record row operations that reshape a record to a new closed value (`Record.project` restrict, `Record.without` drop, `Record.merge` disjoint union, and the derived `Record.extend`/`Record.with`/`Record.pop`; `CDZ0211` on a shared/added field, `CDZ0212` on an absent field) and their positional tuple analogues (`Tuple.concat`/`Tuple.split-at`/`Tuple.remove`), open sums with a mandatory open-tail arm, and schema-typed payload decoding to a typed result; the `rows` / `open-sums` capabilities a later generation realizes (`options/record-tuple-operations/`)
- `16-binary-matching.sexp` — the `(bin …)` binary construction-and-matching form (one dual keyword: construct in expression position, destructure in pattern position) over the `Bytes` value form: fixed-width integer segments with explicit endianness/signedness, sub-byte bit-fields, and dependent-size `bytes` segments; static byte-alignment rejection (`CDZ0220`) and runtime segment-fit trap; the `binary-matching` capability a later generation realizes (`options/binary-syntax/`)
- `17-symbols.sexp` — the `Symbol` interned-name value form: `Symbol.of` interns a `String` to a `Symbol` compared by content in constant time, `Symbol.to-string` recovers it, with a `#"<text>"` reader literal; a Symbol is a nominal value over `String`, so equality reuses String equality and the nominal boundary reuses `CDZ0202`; the `symbols` capability a later generation realizes (`options/symbol-interning/`)
- `19-sets.sexp` — the `Set` unordered unique-element collection (the third built-in beside `List` and `Map`, options/set-collection/): construction with deduplication, order-independent equality, total membership (`Set.contains`, no positional access), cardinality, insert/remove, set algebra (union/intersection/difference), the empty set, and the crucial counterpoint that two sets of the same element type are the same type regardless of their elements (a set's elements are runtime data, not part of its type); the `sets` capability a later generation realizes
- `13-strings.sexp` also carries the `Char` validated-Unicode-scalar surface (`String.scalar-at`, `Char.to-int`/`from-int`, the `#\<scalar>` literal and its `CDZ0002` non-scalar rejection, char ordering/equality); the `chars` capability a later generation realizes
- `18-units-of-measure.sexp` — the optional, compile-time-only dimensional-analysis layer over the numeric core: a quantity `(Qty T u)` pairs an underlying numeric type `T` with a compile-time unit `u` drawn from a free abelian group over `Symbol`-named base dimensions (`Unit.one`/`Unit.base`/`Unit.*`/`Unit./`/`Unit.^`); each dimension is a family of interconvertible units carrying an exact `Rational` scale to a reference unit, so units of one dimension mix (`1 inch + 1 mm`) by automatic exact conversion (`Unit.of`/`Unit.in`) while a mix of dimensions stays `CDZ0501`, with SI-decimal and IEC-binary prefixes as exact scales (`Unit.prefix`); `+`/`-`/comparison require equal dimension, `*`/`/` compose it, a mismatch is the compile-time `CDZ0501`, and `(Qty T u)` erases its dimension to `T` (a scale conversion is the exact arithmetic the source denotes); the dimensional core is realized over `Int`/`Float` magnitudes (construction, erasure, arithmetic, `CDZ0501`, named families and SI/IEC prefixes, automatic and explicit conversion — constant and runtime), and a named unit's conversion is unique (a conflicting redeclaration is `CDZ0502`); the cases whose magnitude is an exact `Rational` await the exact-rational numeric type (`options/units-of-measure/`)
- `25-verification.sexp` — the trust-boundary soundness pins for machine-checked verification (an LCF-style HOL theorem-prover kernel baked into Cadenza as a library; `implementation/design/DESIGN-verification-hol-kernel.md`): a kernel's theorem type `Thm` must be UNFORGEABLE, realized as an ABSTRACT (opaque) type whose constructor is a private inference-rule entry point, so an importer can neither construct (`CDZ0214`), match its variants (`CDZ0214` for a multi-variant proof type), structurally compare (`CDZ0202`), nor forge-by-re-declaration (a same-name re-declared type is a DISTINCT nominal type, `CDZ0203`) a theorem outside the kernel — it obtains one only through the kernel's exported inference rules and reads it only through exported accessors; pins the opaque-type boundary (modules-and-namespaces.md §A Type's Handle And Its Constructors Are Independently Visible; type-system.md §An Abstract Type's Representation Is Not Observable Across Its Boundary) for the `Thm`-shaped type an LCF kernel uses (the `verification` direction below); also runs the kernel SKELETON end-to-end over a realistic HOL fragment (Term concrete: variable/application/equality; Thm abstract: a sequent of hypotheses and a conclusion; the leaf inference rules `refl` ⊢t=t and `assume` p⊢p; a recursive structural term equality; conclusion/hypothesis accessors) — proving reflexivity and `assume` derive real theorems while the sequent `Thm` stays unforgeable; the `eval`-forge pins GUARD the now-fixed reflection hole (`e1506bd7c`), and only the single-variant-match diagnostic pin still awaits its fix (see the file header)
- `26-program-conditions.sexp` — program pre/post-conditions whose proofs are DISCHARGED by the verification kernel (Increment-b; `implementation/design/DESIGN-verification-program-conditions.md`): a pre/post-condition on a Cadenza program denotes into a HOL obligation `Term`, and the kernel discharges it into a `Thm` — and a DISCHARGED obligation is a first-class optimizer input (a proven `no-overflow@Id` lets the Core-tier elision pass drop the overflow guard; four-way seam with v-core-opt/v-wasm-opt/v-rust-backend). Since the HOL kernel has no built-in arithmetic decision procedure, a no-overflow obligation is discharged from an EXPLICIT minimal trusted arithmetic-axiom base (the HOL-Light `ARITH` analogue: numeral order facts `le-ax`, monotonicity `mono-add-r`, transitivity `trans-le`) — for a checked `x + k` under precondition `x <= c`, `assume (le x c)` → `mono-add-r` → `le (add x k) (add c k)` → `trans-le` with a numeral fact → `le (add x k) MAXINT`; arithmetic head-symbols encode as `Const`-headed `Comb` applications so no kernel extension is needed. The `bounds` kernel keeps the same LCF discipline as `hol` (abstract `Thm`, private constructor, rules the only way to mint one). These b1 cases hand-author the obligations and prove them through the kernel (no compiler change), pinning both the positive discharge and the default-is-always-the-check negative (an unconstrained add is NOT dischargeable → the guard stays)
- `26-runtime-params.sexp` — runtime parameters via the `@param` annotation-driven codegen (`implementation/design/DESIGN-runtime-parameter-host-effect.md`; operator direction): a function/value marked `@param(widget: …, …) name : Type` is a RUNTIME INPUT the host supplies, and a build-time sidecar scans every `@param` site and GENERATES a single strongly-typed `Param` effect with one accessor op per param (`Param.width : Int64`, …) that the host binds at run time — a four-way seam where the `@param` annotation surface is v-syntax's, the scan+generate is v-metaprogramming's, and the run-time bind is v-effects' host-effect mechanism; the canonical shape `@param(widget: slider, …) width : Type` parses to `(: (@ (param (: widget slider) …) width) Type)`

Planned as the capabilities they witness are filled in: traits as explicitly-passed dictionaries,
documentation and comments (each a node the compiler sees through, witnessing that it is semantically
inert).
