# Compiler primitives for userspace contract-building — import-reflection, const-execution, blake3

**Status:** design/scoping only — nothing landed. Written 2026-08-25 by the `design-compiler-primitives`
fleet agent, NON-interactively, on an operator directive. This is a doc-only PR (opened to GitHub `main`,
platform-lane direct-to-main model); it changes no compiler code and does not touch the in-flight WIT work.
It hands a build plan to the verticals named in §10. Line numbers are landmarks at trunk `b581d93e1`.

> **The operator's mandate (2026-08-25), verbatim (via the concierge seed).** "The compiler should NOT
> know about contracts — it should expose three lower-level primitives and let a userspace Cadenza program
> compose them: (1) IMPORT REFLECTION — a Cadenza program can import not just a file's exported items but
> the AST itself. (2) CONST EXECUTION — run that AST through a transform at COMPILE TIME (compile-time
> evaluation), emitting the encoded contract in binary. (3) HASHING — add blake3 to BOTH the compiler and
> the runtime component, so contract-hash is callable at compile time AND at runtime (same hash both
> places). Then userspace builds the contract hash itself from these primitives; no contract-aware compiler
> logic, no per-`@!contract` codegen constant."

---

## 1. The problem, and why it is a primitives problem

`design/cadenza-platform.md §1` is unambiguous: identity is the **contract hash and nothing else**. A
`contract = (name, input: Type, output: Type)`; its `contract-id = hash(contract)` over the declaration's
canonical binary form; a program **answers a declared set of contract-ids** and routing is exact hash
equality (§1, lines 32-90). The well-formed reducer the operator wants selects each operation by the
incoming message's **contract** — a distinct contract per op — never by a payload sentinel.

**That reducer is not authorable today.** A Cadenza guest cannot obtain a contract-id at compile time, so
it cannot compare `msg.contract` (opaque 33-byte `Bytes`) against a contract it knows:

- No `Blake3.of` in the guest prelude and no `Contract.id` intrinsic (`rcdzc/src/prelude.rs` enumerates the
  prelude modules — no `Contract`, no `Hash`). `@!contract "name"` is recorded as a **name string only**,
  never an importable id value.
- A contract-id is `blake3(<encoded declaration>)` — mintable only in Rust today
  (`cdz-platform/src/hash.rs:40` `Hash::of` = `blake3::hash`; the platform's sole identity, §8). A guest
  gets an id ONLY by echoing back `msg.contract` or reading one out of a payload field.
- Merged platform work (§7 state reducer) therefore ships an **interim payload-sentinel dispatch**; the
  generic event reducer (the flagship default-handler) is **HELD** pending a real fix.

The operator's ruling is deliberately NOT "add a `Contract.id` intrinsic" — that would put contract
knowledge in the compiler, violating platform principle 1 ("the runtime/compiler knows nothing specific").
Instead the compiler gains **three general primitives** that have value far beyond contracts, and
**userspace** assembles a contract-id from them. The compiler never learns what a contract is.

### What already exists that this design builds on (the crucial assets)

- **The `Ast` type is first-class.** `rcdzc` has a built-in `Ast` sum (Int/Float/Bool/Str/Name/List/Bytes),
  quote/quasiquote, and **`Ast.encode : Ast → Bytes`** / **`Ast.decode : Bytes → (Result Ast e)`** that
  serialize through the canonical `cadenza-ast` codec (`resolved.rs:488-497`; `Prim::AstEncode`/`AstDecode`).
  There is one canonical byte form for hashing, equality, output, and interchange
  (`spec/contracts/ast-encoding.md`, `spec/capabilities/value-interchange.md`) — no second encoding.
  **Caveat that shapes primitive 2:** there is deliberately **no `Core::ConstBytes`** (`core.rs:531`) —
  encoding a value TO `Bytes` is a *runtime* `value-encode` op, so `Ast.encode` does NOT today fold to a
  compile-time bytes constant; a constant value rides to runtime and is encoded there. Producing a
  compile-time bytes constant is precisely the capability primitive 2 must add (§3b).
- **Compile-time evaluation is ONE TIER.** `metaprogramming.md §Compile-Time Evaluation Is One Tier`:
  macro expansion, generic reduction, monomorphization, and constant folding are the SAME mechanism.
  `(eval AST)` reconstructs the source the AST denotes and folds it through that one path
  (`rcdzc/src/eval_ast.rs:1-34`). A macro is "an ordinary compile-time function over the AST." This is the
  substrate const-execution rides — not a new interpreter.
- **Each module is already stored AS its canonical binary AST.** Package linking reads each imported file
  as a `cadenza-ast` document and splices its *items* (`rcdzc/src/link.rs:1-42`, `resolve_import_clause`
  :542). The AST the reflection primitive must expose is **right there** at link time; today only the
  names are bound, never the tree.
- **blake3 is already the fleet's one digest, already a compiler dependency.** `cdz-platform/src/hash.rs`
  (`Hash([u8;32])`, `Hash::of = blake3::hash`); `rcdzc` already links `blake3`. The runtime component does
  NOT yet expose hashing to a guest.

So none of the three primitives is built from nothing: reflection exposes an AST link already has,
const-execution reuses the one-tier evaluator, and hashing wires an existing digest into two places.

---

## 2. The direction — three primitives; userspace composes the contract-id

**In one sentence:** the compiler learns to (1) bind an imported module's **AST itself** as a compile-time
`Ast` value, (2) **fully evaluate a userspace transform** applied to that value to a constant at compile
time, and (3) compute **blake3** at compile time and at runtime with byte-identical results — and a
userspace Cadenza library uses all three to turn a contract *declaration* into its *contract-id*, with the
compiler never modelling "contract."

The userspace composition the primitives unlock (illustrative — this lives in a `.cdz` library, NOT in the
compiler):

```
import { __ast__ as decl } from "temp-celsius.cdz"   -- (1) decl : Ast — the declaration file's whole AST
let bytes = Ast.encode (canonicalize-contract decl)  -- (2) transform + encode, folded to a compile-time Bytes constant
let temp-celsius-id = Blake3.of bytes              -- (3) blake3 → the contract-id, a compile-time constant

fold msg =                                       -- true distinct-contract dispatch, no sentinel:
  if Bytes.eq msg.contract temp-celsius-id then … else …
```

- `canonicalize-contract` is **ordinary Cadenza** — it walks the declaration `Ast` and produces the
  contract's canonical declaration value `(contract name input-type output-type)` per `platform.md §1`. The
  compiler does not know this function means "contract"; it is just a total function over `Ast` that
  const-folds because its input is compile-time-visible.
- `temp-celsius-id` is a compile-time **constant** (a folded `Bytes`/`Hash`), so comparing `msg.contract`
  against it is an ordinary runtime byte-equality — the exact-hash routing `platform.md §1` mandates.
- At runtime, the SAME `Blake3.of` op lets a program hash a declaration it *receives* (e.g. an adapter that
  learns a contract at runtime), and get the identical id the compile-time fold produced.

Nothing here is contract-specific in the compiler. A different userspace library could use the same three
primitives to content-address a config, derive a cache key from a schema, or build a capability token.

---

## 3. The target shape

### 3a. Primitive 1 — IMPORT REFLECTION: bind a module's AST as an `Ast` value

Today `import { a, b } from "path"` binds **names** (`parser.rs:2952` lowers to `(import "path" (name…))`;
`link.rs:542` resolves each name to the sibling module's export). The new surface exposes the imported
module's **canonical binary AST** as a **reserved "magic" name** the module implicitly exports — so a
program imports it in the SAME name-list as the module's functions and types:

```
import { convert, Celsius, __ast__ } from "temp-celsius.cdz"   -- convert/Celsius are ordinary items;
                                                                -- __ast__ : Ast, the whole module document
import { __ast__ as decl } from "temp-celsius.cdz"             -- rename on import (see D1 on the alias form)
```

- **Surface (chosen default, D1 — REVISED per operator review, was a separate `(import … (ast …))` clause):**
  reflection is a **reserved magic name** (`__ast__`, double-underscore convention) that every module
  implicitly exports and that resolves to its canonical AST reified as an `Ast` value. A program imports it
  through the ordinary `import { … } from "path"` name-list, so it can bind the module's **items AND its
  AST together** in one import (the operator's point) — no separate clause, no mutually-exclusive form, and
  the reserved name namespaces it out of collision with user names. It reuses the whole link path
  (`link.rs` already loads `"path"` as a `cadenza-ast` document); resolving the magic name binds it to that
  document reified as an `Ast` value — the same `Ast` a `quote` of the module body would produce. No new
  resolver clause, no runtime cost: the bound value is a compile-time constant.
  - **Alias form:** to bind `__ast__` under a friendlier local name, the `import { __ast__ as decl }` alias
    is the natural spelling, but the alias-import form currently DECLINES (`link.rs:534`). Default: enable
    the `as` alias for imported names (a small `link.rs` extension, generally useful, not reflection-
    specific); if that slips, a program binds `__ast__` directly and `let decl = __ast__`.
- **The bound value is compile-time-visible**, which is exactly what primitive 2 needs. It is an ordinary
  `Ast` value: `Ast.List`/`Ast.Name`/… — walkable by userspace `Ast` operations, encodable by `Ast.encode`.
- **What it reflects (chosen default, D2): the module's full canonical AST** (its `(do …)`/module body as
  stored), NOT a pre-digested "exports only" projection. Reflection stays a raw, general primitive; any
  projection (e.g. "just the exported `@!contract` decls") is a userspace transform (primitive 2). This
  keeps the compiler contract-agnostic — it exposes syntax, not meaning.
- **Erasure:** like `Type.of`, an `Ast` value that is only consumed at compile time (fed to a transform +
  `Ast.encode` + `Blake3.of` that all fold) leaves no runtime residue. An `Ast` value CAN also cross the
  boundary at runtime (the `Ast` sum has a runtime representation and `Ast.encode` works at runtime), so
  reflection is not erasure-fenced the way `Type.of` is — it is a real value.

This is the smallest possible reflection: it does not add a query language or a typed-surface API; it hands
userspace the one thing it cannot get today — the imported file's tree — and lets userspace do the rest.

### 3b. Primitive 2 — CONST EXECUTION: fold a userspace transform to a constant

The requirement: a userspace transform applied to a compile-time-visible `Ast` value must **fully evaluate
at compile time to a constant** (the encoded contract `Bytes`), with that constant baked into the program.

The mechanism is the **one-tier evaluator** already in place (§1): `eval`/reify + constant folding. This
primitive pins down the **guarantee and its boundary** AND adds the one missing representation — a
compile-time bytes constant — not a new evaluator subsystem:

- **Guarantee:** when every input to a pure, total transform is compile-time-visible (an imported `Ast`
  from 3a is), the compiler evaluates the whole application — including `Ast` walking, `(list …)`
  operations, **recursion over the tree**, and the terminal `Ast.encode`/`Blake3.of` — to a constant value,
  and bakes that constant. The transform runs ZERO times at runtime.
- **Gap A — the missing constant kind (`ConstBytes`), chosen default D3:** today there is **no
  `Core::ConstBytes`** (`core.rs:531`): encoding a value to `Bytes` is a *runtime* `value-encode` op, so
  `Ast.encode`/a transform's `Bytes` result cannot become a compile-time constant — it rides to runtime.
  Const-execution to "the encoded contract in binary" therefore REQUIRES a compile-time bytes constant.
  Default: add a `Core::ConstBytes` (a folded `[u8]` literal) and a const-fold arm for `Ast.encode`/
  `Value.encode` over a compile-time-visible value that produces it (the codec runs in `rcdzc` at compile
  time, exactly as `blake3::hash` will for `Blake3.of`). The backend lowers a `ConstBytes` into the program's
  data section like any literal.
- **Gap B — folding a user function over `Ast`, chosen default D4:** today's constant folding is scalar-
  only and non-recursive across user calls (`[[const-fold-nonrecursive-call-scalar-only]]`), and the
  evaluator declines recursive fns to normal form (`eval.rs:1082`); `eval` reconstructs-and-folds only
  fully-reconstructable literal `Ast`. Const-execution over an *imported* AST needs the evaluator to fully
  evaluate **a user function applied to a compile-time `Ast` value** — recursion + compound (`list`/`Ast`)
  intermediates. Default: **extend the one-tier evaluator (`eval.rs` `beta_reduce`/`apply_lambda`) to fully
  evaluate a compile-time-total application over `Ast`/compound data**, bounded by the existing eval
  step/depth guard, reusing the `eval`/reify substrate — NOT a second evaluator (`metaprogramming.md`
  mandates ONE tier). This is the meatiest slice; `v-metaprogramming` (owns quote/eval) + `v-inference`.
- **Decline, never miscompile (the discipline this must preserve):** if the transform cannot be fully
  evaluated at compile time (a non-total function, an unbounded recursion, a runtime-only input), the
  compiler **DECLINES** (grades `todo` / emits a coded reject) — it MUST NOT emit a half-evaluated
  transform or silently defer it to runtime. Contract-id construction that does not fold is a build error
  the author fixes, exactly as an over-budget `eval` is today.
- **Output form:** a first-class compile-time constant (the new `ConstBytes`, or a `ConstInt`/`ConstBool`
  where the transform is scalar), usable anywhere a literal is — bound with `let`, compared with `Bytes.eq`,
  embedded in a larger structure. NOT a bespoke "emit a data section" side-channel: "emitting the encoded
  contract in binary" IS the folded `ConstBytes`, which the ordinary backend lowers into the program's data.

No `@!contract`-aware codegen, no per-contract constant minting: the compiler folds a general application;
the fact that the application computes a contract-id is invisible to it.

### 3c. Primitive 3 — HASHING: blake3 at compile time AND runtime, same bytes

Add blake3 as a guest-callable hash in **both** places, producing byte-identical 32-byte digests:

- **Guest prelude surface — NAME THE ALGORITHM (chosen default, D5, REVISED per operator review):**
  `Blake3.of : Bytes → Bytes` (a 32-byte blake3 digest as a `Bytes` value), a **`Blake3`** module in
  `rcdzc/src/prelude.rs` mirroring the `Type`/`Qty` module pattern (`Prim::Blake3Of` + `from_name
  "blake3-of"`). The surface **names the algorithm** rather than exposing a generic `Hash` — the operator's
  point: which digest is computed is part of the identifier, so a future algorithm is a DIFFERENT named
  function (e.g. a later `Sha256.of`), never a silent change to a generic `Hash`. This also matches the
  runtime op's name (`hash-blake3`). Result is a plain 32-byte `Bytes` — NOT a distinct nominal `Hash` type
  — so it composes with the existing opaque-`Bytes` `msg.contract` (`world.wit` `type contract-id =
  list<u8>`) by ordinary `Bytes.eq`, no wrapping/unwrapping (v-platform endorsed this on the PR). (D5 alts:
  a generic `Hash.of` — rejected, hides the algorithm; a nominal `Hash` result type — rejected, forces
  adapters at every `msg.contract` comparison.)
- **Compile-time fold:** `Blake3.of` over a compile-time-visible `Bytes` (the output of 3b) **const-folds**
  to a `ConstBytes` (§3b P0) via `blake3::hash` in `rcdzc` — blake3 is already vendored in the workspace
  (`cdz-platform`, `cdz-run`; rcdzc does not blake3 at compile time today, so this adds the call). This is
  the compile-time half → the baked contract-id constant.
- **Runtime op (chosen default, D6): an APPENDED value-heap runtime op** `hash-blake3(bytes: u32) -> u32`
  at **index 91** (next free after `value-decode` at 90 — `runtime.wit:474`), a `Bytes` handle → a fresh
  32-byte `Bytes` handle, computed with the SAME `blake3` crate. Append-only is the frozen rule
  (`value-heap-runtime.md`); this bumps `REQUIRED_RUNTIME_HASH` (a one-time re-derivation via `xtask
  codegen`, the sanctioned cost). `Blake3.of` lowers to `Core::Blake3Of` → this op at runtime, and const-folds
  at compile time — one surface, two lowerings, identical bytes (both call `blake3::hash`).
  - **D6 alt — an imported `cadenza:blake3` component** (mirroring the `cadenza:nfc/normalize` self-
    describing runtime dep, `runtime.wit:485`): rejected as default because blake3 is a small pure
    function already vendored, not a heavy table like NFC — an appended heap op is simpler and keeps the
    "same code both places" guarantee trivially (both paths are `blake3::hash`). Revisit only if the
    runtime-component-minimization directive argues for pulling it into a separate composed component.
- **Byte-identity is the load-bearing invariant:** the compile-time fold and the runtime op MUST produce
  identical digests for identical input bytes. Guaranteed structurally by both calling the one `blake3`
  crate over the same canonical input bytes; pinned by a gate (§9) that hashes a fixture both ways and
  asserts equality.
- **Domain separation is USERSPACE's job (operator-confirmed on the PR).** The compiler's `Blake3.of` is
  entirely generic — raw `bytes → digest`, no tag, no prefix, no notion of "contract." A contract-id per
  `platform.md §1` is `hash(<the bytes userspace assembled for the declaration>)`; the declaration's
  canonical `cadenza-ast` form is already structurally distinct from a raw payload, and **any** prefix/tag a
  scheme wants is prepended in userspace before `Blake3.of`. So **no `HashTag` byte comes from the
  compiler** — see D7. This keeps the primitive maximally general (it hashes anything, for any purpose).

---

## 4. How the three compose into a contract-id (the userspace side — informative)

This section is NOT compiler work; it shows the primitives suffice, so the build can be validated against a
real target. It lands as a `.cdz` library in the platform lane (v-platform), NOT in `rcdzc`.

1. `import { __ast__ as decl } from "temp-celsius.cdz"` → `decl : Ast` (primitive 1).
2. A userspace `canonicalize-contract : Ast → Ast` walks `decl`, extracts the `@!contract` declaration's
   name + input/output type sub-ASTs, and builds the canonical `(contract name input-type output-type)`
   value form `platform.md §1` pins (primitive 2 folds this).
3. `Ast.encode` that canonical value → its canonical `cadenza-ast` bytes (const-folds).
4. `Blake3.of` those bytes → the 32-byte contract-id constant (primitive 3, compile-time fold).
5. The reducer compares `msg.contract` against that constant with `Bytes.eq` — true distinct-contract
   dispatch, exact hash equality, compiler contract-agnostic throughout.

The runtime `hash-blake3` op covers the symmetric case: a program that receives a declaration at runtime
(an adapter, a registry) hashes it and gets the same id.

---

## 5. Increments (each its own commit + gate; top-to-bottom, the way a vertical lands them)

Ordered so trunk stays green and each slice is independently useful. P3 lands first (self-contained,
unblocks the compile-time fold target); P1 and P2 follow; the userspace validation library lands last.

**P3a — blake3 runtime op (`v-runtime` + `v-hash-encoding`).** Append `hash-blake3` (idx 91) to
`runtime.wit` + its `cdz-runtime` impl (`blake3::hash` over a `Bytes` handle → fresh 32-byte `Bytes`).
Gate: `xtask codegen` regenerates `runtime_abi.rs` with the new op + bumped `REQUIRED_RUNTIME_HASH`
(`codegen --check`); a unit test asserting the op's output equals `blake3::hash` of the input; existing
runtime tests unaffected (append is disc-stable). **Probe increment** — proves the runtime half in
isolation before any guest consumes it.

**P3b — `Blake3.of` guest prelude + runtime lowering (`v-inference` + `v-rust-backend`/`v-compiler-ml`).**
Add the `Blake3` module (`Prim::Blake3Of`, `from_name "blake3-of"`) to `rcdzc/src/prelude.rs`; infer arm
`(Blake3.of e) : Bytes`; lower `Core::Blake3Of` → runtime op 91 (wasm) + the rust backend arm
(new-`Core`-variant rule). Gate: a corpus case where a runtime `Bytes` routes through op 91 and executes
under wasmtime to `blake3::hash` of the input. (The COMPILE-TIME `Blake3.of` fold is deferred to P2, where
`ConstBytes` exists — see the byte-identity gate in §9.) `dev-gate` + scoped `gate --files … --target wasm`.

**P0 — `Core::ConstBytes` compile-time bytes constant (`v-metaprogramming` + `v-rust-backend`).** Add the
`Core::ConstBytes` variant (`core.rs:531` has none today), its wasm + rust backend lowerings (into a data
literal), and a const-fold arm for `Ast.encode`/`Value.encode`/`Blake3.of` over a compile-time-visible input.
Gate: a corpus case where `Ast.encode` of a literal `Ast` value folds to its golden canonical bytes AT
COMPILE TIME (no runtime `value-encode`), executing byte-identical; a `Blake3.of` of a literal `Bytes` folds
to the golden blake3 digest at compile time. This is the shared foundation P2's transform output and P3b's
compile-time `Blake3.of` both need; it is independent of P1 and can land alongside P3.

**P1 — import-reflection `ast` binder (`v-metaprogramming` + `v-inference`).** Parse `(import "path" (ast
name))` (`parser.rs` import clause); in `link.rs`, bind `name` to the imported module's document reified as
an `Ast` value; type it `Ast` (infer). Gate: a corpus case importing a sibling `.cdz`'s AST and reading a
node out of it (e.g. `Ast.encode` of the imported AST folds to the sibling's canonical bytes), verified
byte-identical to the sibling compiled standalone. No runtime cost — `name` is a compile-time constant.

**P2 — const-execution of a transform over an imported AST (`v-metaprogramming` + `v-inference`).** Extend
the one-tier evaluator (`eval.rs` beta-reduce/fold) so a pure, total user function applied to a compile-time
`Ast` value fully evaluates — recursion + `(list …)`/`Ast` compound intermediates — to a constant (landing
in a `ConstBytes` from P0 when the result is bytes), bounded by the existing eval guard; DECLINE (coded)
when it cannot fully fold. Gate: a corpus case where a recursive userspace `Ast → Bytes` transform applied
to an imported AST folds to a constant `Bytes` (compared byte-identical to the same transform run in Rust
over the same input); a companion case where a non-total transform DECLINES with a code (never a
miscompile, never a runtime residue). This is the meatiest slice; it depends on P1 (a compile-time `Ast`
input) and P0 (the `ConstBytes` output), and its terminal `Blake3.of` folds via P0/P3b.

**P4 — userspace contract-id library + the held reducer (`v-platform`, then `v-platform-itest`).** With
P1–P3 on trunk, write the `.cdz` `canonicalize-contract` + `contract-id` helpers (§4) and rework the §7
state reducer to true distinct-contract dispatch; retire the interim payload sentinel. Gate: a conformance
run (v-platform-itest harness + Checker) driving distinct-contract dispatch through the platform — the
operator's headline, and the thing that was HELD. NOT a Rust test (behavioral coverage → conformance
suite).

(P3a, P3b, P0, and P1 are independent and can land in parallel — P3b's runtime lowering needs P3a's op;
P0 introduces `ConstBytes`; P2 depends on P1 + P0; the compile-time `Blake3.of` fold lands in P0; P4 depends
on all of P0–P3. Each is independently green.)

---

## 6. Open decisions (each with a chosen default; escalate only a genuine fork)

- **D1 — reflection surface: a reserved magic name (`__ast__`) in the ordinary import list vs a separate
  `(import … (ast …))` clause vs a standalone `(reflect "path")` prim (REVISED per operator review).**
  Default (operator-preferred): a **reserved magic name** `__ast__` every module implicitly exports,
  imported through the normal `import { … } from "path"` name-list — so a program binds the module's items
  AND its AST in one import, and the reserved (double-underscore) name namespaces it out of collision.
  Reuses the `link.rs` module-load path (the AST is already loaded there); no new clause. Alts: a separate
  `(import … (ast name))` clause — rejected, it is a mutually-exclusive form that cannot also import the
  module's items and risks name collision; a standalone `(reflect "path")` prim — rejected unless a use
  case wants reflection outside an import. (Open sub-decision, settled in-build: whether `__ast__` is
  literally the spelling or a differently-named reserved token — `v-inference`/`v-metaprogramming` pick the
  final reserved name; the operator suggested `__ast__`.)
- **D2 — reflect the FULL module AST vs an exports-only projection.** Default: full canonical AST — keeps
  the compiler contract-agnostic (it exposes syntax, not a curated view); any projection is a userspace
  transform. Alt (exports-only) leaks a notion of "what matters" into the compiler; rejected.
- **D3 — the compile-time bytes constant: add `Core::ConstBytes` vs keep bytes runtime-only.** Default: add
  `Core::ConstBytes` (`core.rs:531` today has none) + a const-fold arm for `Ast.encode`/`Value.encode` over
  a compile-time-visible value, so "the encoded contract in binary" is a real compile-time constant the
  backend lowers into data. Keeping bytes runtime-only (the status quo) makes const-execution to bytes
  impossible; rejected. This is a real new `Core` variant → needs a rust-backend arm (the standing
  new-`Core`-variant rule).
- **D4 — extend the one-tier evaluator vs a separate compile-time interpreter.** Default: extend the
  existing one-tier evaluator (`eval.rs` beta-reduce/fold) to fully evaluate a total application over
  compile-time `Ast`/compound data — `metaprogramming.md` mandates ONE tier, so a second interpreter would
  violate the spec. The open sub-question (how far the fold bound reaches, incl. recursion — declined today
  at `eval.rs:1082`) is settled IN-BUILD by `v-metaprogramming` against the existing eval step/depth guard;
  a transform exceeding the bound DECLINES (never miscompiles).
- **D5 — `Blake3.of` result type: plain 32-byte `Bytes` vs a nominal `Hash` type.** Default: plain `Bytes`,
  so it composes with the opaque-`Bytes` `msg.contract` by `Bytes.eq` with no adapter. Alt (nominal `Hash`)
  is tidier in the type system but forces unwrap at every comparison against `contract-id = list<u8>`;
  rejected as default. Revisit if the platform later makes `contract-id` a nominal type end-to-end.
- **D6 — blake3 in the runtime: appended heap op (idx 91) vs an imported `cadenza:blake3` component.**
  Default: appended heap op — blake3 is a small pure function already vendored (unlike the heavy NFC
  tables that justified a separate component), and one `blake3::hash` code path in both places makes the
  byte-identity invariant trivial. Escalate to an `ask` ONLY if the runtime-component-minimization
  directive is read to require pulling blake3 into a composed component.
- **D7 — domain-separation tag / prefixes (CONFIRMED by operator review).** The compiler provides
  **entirely generic** hashing — `Blake3.of` is `bytes → digest` and nothing more; it has NO notion of a
  contract, a tag, or a prefix. **Userspace owns ALL prefixing/domain-separation** (operator, verbatim on
  the PR: "the userspace should be in charge of prefixes; the compiler should be entirely generic and just
  provide hashing functionality"): a userspace scheme that wants a domain tag prepends it to the bytes
  before `Blake3.of`. So the id is `blake3(<whatever bytes userspace assembled>)` with no hidden tag byte
  from the compiler. (The old nuked platform's `HashTag::Contract` is NOT reintroduced — the new
  `cdz-platform` `Hash::of` takes raw bytes; any tag discipline lives in the P4 userspace library.)

---

## 7. Watch-outs (for the implementing verticals)

- **Append-only is sacred (P3a).** `hash-blake3` MUST be idx 91 (next free) — never inserted mid-list; a
  reorder breaks every deployed program's baked import indices. `xtask codegen --check` is the guard; the
  `REQUIRED_RUNTIME_HASH` bump is expected and correct.
- **Byte-identity is the whole point of primitive 3.** The compile-time fold and the runtime op MUST agree
  bit-for-bit. Both call the one `blake3` crate over the same canonical bytes; the gate that hashes a
  fixture both ways and asserts equality is non-negotiable. Do NOT let the two paths drift (e.g. one over a
  `Bytes` handle's raw contents, the other over a re-encoded form).
- **Decline, don't miscompile (P2).** A transform that cannot fully const-fold DECLINES (coded / `todo`) —
  it MUST NOT emit a partially-evaluated body or silently push the transform to runtime. This is the same
  discipline `eval` already holds; the const-execution extension must preserve it. A `Todo→Fail` corpus
  flip here would be a genuine miscompile.
- **The compiler stays contract-agnostic (all increments).** None of P1–P3 may reference "contract",
  `@!contract`, or a contract-id shape. If an increment finds itself special-casing a `(contract …)` node
  or minting a per-`@!contract` constant, it has drifted from the mandate — the contract meaning lives ONLY
  in the P4 userspace library.
- **`cadenza-ast` byte-stability is the structural gate.** Reflection (P1) and `Ast.encode` (P2) both
  depend on the frozen canonical form; the `cdzast\x00\x01` round-trip corpus must stay green. Coordinate
  with the `cadenza-ast` owner before touching the codec — this design REQUIRES no codec change, only its
  use in a new place (binding a loaded module document as an `Ast` value).
- **Do not touch the in-flight WIT work.** The generic world-import/export call surface must finish
  independently; this design is doc-only and its P-increments are additive to it. In particular P3a appends
  a runtime op — it does not alter the reducer/kernel boundary the WIT lane owns.
- **`Type.of`/`Type.eq` precedent (P1/P3b).** The reflection + prelude-module machinery mirrors the
  existing compile-time reflection family (`[[type-of-compile-time-reflection]]`): a `Prim` + a prelude
  namespace record + an infer arm + a lower arm, and the two exhaustive lower tables (fold-decline +
  intrinsic-name) each need the new arm. Reuse that shape; do not invent a new one.

---

## 8. Verification (the gate that protects this)

- P3a: `hash-blake3(b)` equals `blake3::hash(b)` for a fixture corpus; `xtask codegen --check` green with
  the new op + bumped hash; existing runtime tests unaffected.
- P3b: a `Blake3.of` of a runtime `Bytes` executes through op 91 under wasmtime to `blake3::hash` of the
  input; both wasm and rust backends.
- P0: `Ast.encode`/`Blake3.of` of a literal input folds to the golden bytes / blake3 digest AT COMPILE TIME
  (a `ConstBytes`, no runtime `value-encode`/op 91), executing byte-identical; the byte-identity gate (§9)
  ties this to P3a/P3b.
- P1: importing a sibling module's AST and `Ast.encode`-ing it folds to the sibling's canonical bytes
  (byte-identical to the sibling compiled standalone); `name : Ast`; no runtime residue.
- P2: a recursive userspace `Ast → Bytes` transform over an imported AST folds to a constant `Bytes`
  (byte-identical to the same transform run in Rust); a non-total transform DECLINES with a code.
- P4: a conformance run (v-platform-itest harness + Checker) drives true distinct-contract dispatch through
  the platform; the interim payload sentinel is retired. **P4 MUST also cross-check the userspace-built id
  against the platform's existing Rust contract-id** (see §9's second equality) — `userspace
  contract-id(decl) == cdz_platform::Contract::of(decl).id()` for a fixture contract; if these diverge,
  guest dispatch silently never matches a routed contract (v-platform anchor).
- Throughout: `cargo test -p rcdzc --lib` 0 failed; `cargo xtask gate` additive-only (no `Todo→Fail`);
  fmt (pinned) + clippy + `codegen --check` clean; the `cadenza-ast` byte-stability corpus stays green.

---

## 9. The two byte-identity gates — the invariants worth stating twice

Two DISTINCT equalities protect "same hash both places," and both must be pinned:

1. **Compile-time == runtime (primitives P0/P3, the compiler's job):** `Blake3.of` const-folded at compile
   time == `hash-blake3` op executed at runtime, for the same input bytes. The gate MUST assert this
   directly (hash a fixture at compile time via the fold and at runtime via op 91; assert the two 32-byte
   outputs are equal), not merely test each half in isolation. If this diverges, a guest's compiled-in
   contract-id would not match a runtime-hashed declaration. Guaranteed structurally by both paths calling
   the one `blake3` crate over the same canonical input bytes.
2. **Userspace == platform (P4, v-platform's anchor):** the userspace-built contract-id must be
   byte-identical to the platform's EXISTING Rust contract-id, because that is the id the EventRegistry
   registers and dispatch routes on (`platform.md §1` exact-hash equality). Today `cdz_platform::Contract`
   computes `id = Hash::of(<the declaration's canonical cadenza-ast encoding>)`, so P4's
   `canonicalize-contract` + `Ast.encode` MUST reproduce that exact declaration byte-form. P4's gate pins
   `userspace contract-id(decl) == cdz_platform::Contract::of(decl).id()` for a fixture contract. This is a
   DIFFERENT equality than (1) — compile-time-vs-runtime blake3 identity vs userspace-vs-platform
   declaration-encoding identity. If either diverges, guest dispatch silently never matches a routed
   contract. Together they are the load-bearing correctness property of the whole design.

---

## 10. Ownership / cross-lane (for the PM — do not assign from here)

- **Primitive 3 — blake3:** runtime op = `v-runtime` + `v-hash-encoding`; guest prelude + compile-time fold
  = `v-inference` (prelude/infer) + `v-rust-backend`/`v-compiler-ml` (lower/backend arms).
- **Primitive 1 — import reflection:** parse + link binding = `v-metaprogramming` (owns `Ast`/quote/eval)
  + `v-inference` (import resolution, typing).
- **Primitive 2 — const execution:** one-tier evaluator extension = `v-metaprogramming` + `v-inference`.
- **P4 — userspace contract-id library + held-reducer rework:** `v-platform` (owns the `.cdz`
  `canonicalize-contract` + `contract-id` library, the §7 dispatch + generic-event-reducer rework, and the
  platform contracts adopting the primitives), then `v-platform-itest` (the conformance run). v-platform
  confirmed P4 ownership on the PR and takes it once P0–P3 are on trunk.
- **Scope clarification (v-platform, on the PR):** the compiler-internal primitive work (P0–P3) is
  `v-metaprogramming`/`v-inference`/`v-runtime`/`v-rust-backend` — **NOT** v-platform-owned codegen.
  v-platform owns P4 only. The existing `@!contract` codegen path and the new import-reflection path are
  independent (§3a), so no platform-codegen interaction is expected; if one surfaces (e.g. the `@!contract`
  glob vs the `__ast__` import path), flag v-platform.

Route peer questions via `cargo xtask fleet send`. This doc is the coordination surface; the PM mints the
verticals (or the concierge does on the operator's say-so).
