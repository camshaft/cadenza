# Self-hosting is gated on generics; the rest is libraries and scale

*2026-07-05*

**What happened.** We ran a three-front gap analysis — inventory the Rust seed compiler as a
*program*, survey what the language spec offers on paper, and confirm what the seed *realizes*
end-to-end — to answer one question: what is missing before the Cadenza compiler can be authored in
Cadenza and compiled by the seed. Three facts reframed the gap smaller than "port the whole
workspace":

- **The self-host target is the pure `bytes → bytes` core, not the workspace.** Only
  `codegen.rs` + `ast.rs` + `diagnostics.rs` (~7,300 lines of Rust) must be re-authored. The
  `wasmtime` host, `sha2`, and the CLI stay foreign Rust forever — they are the oracle/driver that
  runs whichever compiler component (Rust seed or Cadenza-authored) is handed to them, and the
  self-hosting fixpoint is a byte-identity check between the two components' output. This is
  [bootstrap targets the compiler directly](./2026-07-03-bootstrap-targets-the-compiler-directly.md)
  and [the compile seam is statically typed](./2026-07-03-the-compile-seam-is-statically-typed.md)
  made quantitative.
- **The seam is a pure function**, `compile : list<u8> -> result<list<u8>, list<diagnostic>>`. The
  core has no filesystem, env, or process access. So **effects and handlers are NOT on the critical
  path** — the hardest-looking capability is irrelevant to bootstrapping.
- **The core consumes binary AST (CBOR), not text.** The recursive-descent text reader in `ast.rs`
  is explicitly outside the trusted derivation path. So the Cadenza compiler needs a **CBOR decoder**,
  not a text reader; the reader stays a host convenience.

What the seed already runs makes the *data-structure* port idiomatic: recursive sum types
(`Node`/`CVal`/`Shape` map directly onto Cadenza sums), `match` + exhaustiveness, records, closures +
currying + recursion + HOF pass/return, single modules, `Bytes` (the wasm-buffer substrate),
`String` + NFC-equality, `Int64` checked arithmetic + shifts (LEB128 is a proven corpus idiom), and
the full reject/decline diagnostic machinery. The gaps are not in expressing the compiler's data —
they are in **generics, libraries, and scale**. Sorted by what blocks the bootstrap:

**Tier 1 — blocking, language-level (must be built into the language):**

*Correction (2026-07-05, live re-verification).* The original framing below — "the linchpin is full
HM; the seed declines a polymorphic `id`" — was **too pessimistic**, corrected here against the seed
as it actually runs. Live: `(def (id x) x)` applied at TWO different types in ONE program
(`(both 5 true)` → `(tuple (id a) (id b))`) COMPILES today, because `gen_call` realizes polymorphism
by **per-call-site inlining/monomorphization** (bind the parameter to the argument node, emit the
body) rather than by declining. So a usable slice of polymorphism already exists, and the port can
make real progress *before* the full principal-type HM increment. What actually blocks the
compiler-idiom code, verified by probing the real idioms, is narrower and more concrete:

- **Runtime lists you can recurse over, and lambdas you can pass as arguments.** This — not abstract
  HM — is the true first wall. A compiler is `map`/`fold`/recursion over a `Vec<Node>`, and every one
  of those idioms declines today: a `(fn …)` passed as an ARGUMENT → `bare lambda in scalar position`
  (a let-bound lambda works, so it is a dispatch gap, not a missing feature); a recursive function
  that consumes/builds a RUNTIME list via `match` → `constant compound … no runtime constructor` (only
  compile-time-constant lists fold; there is no `cons`/empty destructuring pattern and no runtime list
  recursion); and there is no iteration layer at all (`List.map`/`fold`/`len`/`at`/`rest` are not even
  spec primitives — only total-or-trap `List.at` is pinned). Sorted cheapest-first: (1) lambda-as-
  argument, (2) runtime list recursion + `cons`/empty `match`, (3) a real collection-ops layer.
- **Generics + full principal-type HM inference** is still needed EVENTUALLY — the container-heavy
  core (`Vec<T>`, `BTreeMap<K,V>`) must type-check its own polymorphic code cleanly, which the
  per-call inlining slice does not fully cover — but it is **not the first gate**: the list/lambda
  idioms above are, and they are far smaller. It is
  [generics-are-type-valued-parameters](./2026-07-04-generics-are-type-valued-parameters.md) +
  [inference-is-hindley-milner](./2026-07-04-inference-is-hindley-milner.md) +
  [the bidirectional boundary](./2026-07-04-inference-meets-first-class-types-at-a-bidirectional-boundary.md),
  all specified, partially realized (per-call monomorphization). It carries no milestone label in the tree.
- **Deterministic ordered maps/sets.** The seed's own symbol tables use `BTreeMap`/`BTreeSet`
  *for reproducible output*. Cadenza has a primitive `map`, but iteration-order determinism is
  deferred and two open miscompiles exist (different-keyset comparison, list-of-maps homogeneity).
- **Closure edge-declines.** A compiler is `map`/`fold` over node lists with *named* helpers and
  stores dispatch functions in tables. Two current declines hit exactly that: a named-def HOF
  receiving a lambda argument, and a function stored in a collection then called.
- **A growable-buffer story.** wasm emission is `Vec<u8>` push/extend; on an immutable acyclic heap
  naive append is O(n²). The rope-backed iolist/`Bytes` direction is on paper, not realized.

**Tier 2 — blocking, library-level (the language can express them; they must be *authored in
Cadenza*):** a canonical **CBOR codec** (the binary-AST bijection); **Unicode NFC normalization**
(algorithm + data tables; no primitive); **string building / int→string / formatting** (pervasive
for diagnostics and canonical text — the seed bakes its own `itoa`; Cadenza has `String.concat` but
no formatting); **float bit-access** (`to_bits`/`from_bits`/`is_nan`/`fract`, for canonical float
equality and `{:.0}.0` rendering — rides the unrealized float-arithmetic layer); and reproducing the
**baked component-envelope byte constants** (`RT_HEAD`/`RUNNABLE_ENVELOPE_TAIL`/`HOST_MEM_MODULE` —
just `Bytes` literals, but load-bearing for byte-identity). This tier is the bulk of the *writing*
effort, but it is purely additive Cadenza source authored against the seam
([the assembler lives in Cadenza](./2026-07-03-the-assembler-lives-in-cadenza.md),
[the seed realizes Bytes](./2026-07-03-seed-realizes-bytes-so-the-compiler-emits-components.md)).

**Tier 3 — scale/correctness defects in the seed that will bite a 7,300-line self-compile:** no
TCO / bounded stack (the tree-walking compiler traps at the host call-stack limit, ~15–18k frames —
a large nested source can hit it); and the seed is **2ⁿ in `let`/`if` nesting depth** (environment
deep-cloned per level; ~depth 28 hangs). Both are seed defects, but the self-host source is exactly
the deeply-nested input that triggers them, so they must be fixed before the seed can chew through
the Cadenza-authored compiler.

**Tier 4 — specified but NOT required for the seam (defer):** effects/handlers, multi-module
composition (one big module works), traits (explicit-dictionary dispatch suffices; specified with
zero corpus), symbol *interning* (a String-backed symbol table works — interning is a speed win, not
a requirement), macros, `eval`, rows/open-sums, `bin` matching, units, verification, PBT, and the
text reader (stays host-side). Width-indexed integers (the only named future milestone, "M4") are
*nice-to-have* — `Int64` + masking already expresses LEB128 — and ride generics anyway.

**Why.** The gap has this shape because of two prior decisions compounding. First, the seam was made
a **pure, statically-typed, byte-to-byte function** with the host owning all I/O and value shapes
([host is value-agnostic](./2026-07-04-host-is-value-agnostic-compiler-owns-reader-printer.md),
[two compilers not an interpreter and a compiler](./2026-07-04-two-compilers-not-an-interpreter-and-a-compiler.md)).
That is what pushes effects, the reader, and the runtime host out of the bootstrap's critical path —
the compiler-as-a-program is pure computation over sum types and byte buffers, nothing more. Second,
**generics were unified with first-class types and compile-time evaluation into one mechanism**
([generics are type-valued parameters](./2026-07-04-generics-are-type-valued-parameters.md),
[compile-time evaluation is one tier](./2026-07-04-compile-time-evaluation-is-one-tier.md)). That is
elegant and correct, but it means there is no partial generics — the seed either has the one
compile-time evaluation tier with HM inference or it declines all polymorphism. There is no "just add
`Vec<T>`" without it. So the entire self-host is gated on a single unbuilt increment, and once that
increment lands the remaining work is *additive*: author libraries in Cadenza (Tier 2) and harden the
seed's evaluator against depth (Tier 3), neither of which changes a frozen contract. The port is not
a rewrite fighting the language; it is one hard feature followed by a lot of ordinary code — which is
the good failure mode, given [decline-don't-miscompile](./2026-07-03-decline-do-not-miscompile.md)
keeps every gap observable rather than silently wrong.

**The requirement it drove.** No RFC-2119 sentence — this is a course-setting analysis. It confirms
and sharpens the existing operator roadmap (M0–M9) rather than replacing it. The full ladder is
operator steering, not written into the tree: **M0/M1 done, M2 done through Phase C**; **M3 = static
type system + rows** is exactly where generics + principal-type HM inference lands; then M4 numeric +
open sums, M5 traits, M6 effects, M7 verification, **M8 = re-author `compiler.cdz` in the full
language**, M9 = the byte-identity fixpoint. The in-tree milestone log (`implementation/DECISIONS.md`,
naming only M0–M2; `seed-ignition-set.md` referencing M4) is stale against that ladder — a
documentation gap, not a design one. What this analysis *adds* is the reason the operator's
"grow the full language in the Rust seed first, defer self-hosting to M8/M9" steer is the right
order: **M3 (generics/HM) is the single gate** — because generics were unified with first-class types
into one all-or-nothing compile-time tier, there is no polymorphic compiler code, no container
library, and no width-indexed integers (M4) until M3 lands. It also surfaces two items the ladder
does not call out as bootstrap-blocking: the **Tier-3 seed scale defects** (no TCO/bounded stack, 2ⁿ
nesting) must be fixed before the seed at M8 can compile a 7,300-line source, and the **Tier-2
libraries** (CBOR, NFC, int→string/format, float-bits, baked envelope bytes) *are* the substance of
the M8 re-authoring, authored in Cadenza against the seam once M3–M7 make the language rich enough to
express them cleanly. This composes with the tag-free-runtime line
([the runtime is tag-free](./2026-07-05-the-runtime-is-tag-free-rendering-walks-a-static-shape.md)) —
no type erasure is precisely why generics/monomorphization is the one thing that must exist before a
compiler that manipulates its own types can be compiled.
