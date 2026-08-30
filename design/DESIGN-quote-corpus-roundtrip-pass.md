# Quote binary-AST round-trip corpus pass — quote every corpus input, round-trip the AST across the caller boundary

> **Status:** DESIGN (design-quote-corpus-pass, 2026-08-30, operator-requested + operator-answered).
> Owner (build): a new `vertical` (subsystem `cdz-corpus`/`flake.nix`, coordinating with
> `v-metaprogramming` for the collection-literal quote work (seq74/75), `v-corpus-harness` for the
> grader/baseline, `v-test-shred`/`v-nix` for the shred wiring).
>
> **Operator intent (verbatim, 2026-08-30 via Slack):** "i also want an automated way to go through
> every single corpus input and wrap it in a `quote` and make sure it compiles and returns the
> correct value. so this would be another pass in the shredding tests."
>
> **Operator answer on the "correct value" crux (verbatim, 2026-08-30):** "i want to have two
> functions in the emitted program. one that quotes it and returns the binary ast encoded. and then
> one that decodes the binary ast the caller was returned and it passed it back and then the program
> asserts that the decoded ast matches the quoted ast value. that way we can round trip the entire
> thing and make sure the compiler doesn't fully const fold the encode-decode path."

## 1. What this is

We have 10,141 corpus cases in `spec/semantics/*.sexp`. The language has metaprogramming: `quote`
reifies a source form into an `Ast` **value** (`rcdzc/src/quote.rs`, `reify_quotes`), and the binary
codec (`cadenza-ast/src/codec.rs`, `Ast.encode`/`Ast.decode` prims) serializes an `Ast` value to
bytes and back — the compiler's native, standing **binary-AST exchange format**.

This design adds a **new automated corpus pass** ("another pass in the shredding tests") that, for
every eligible corpus input `E`, emits a program that **round-trips `quote(E)` through the binary
codec across the caller/component boundary** and asserts the decoded AST equals the quoted AST. It is
a 1000×-wider regression net for `quote` reification + the binary codec, exercised at **runtime**
across a real component boundary — with an explicit **anti-const-fold** guarantee.

**Key property — semantics-agnostic, so it covers the *whole* corpus.** Because the pass **quotes
the syntax of `E` and never evaluates it**, it does not matter what `E` *means* — a value case, a
trap case, or even a compile-*error* case are all just quotable syntax. This is why it can honor the
operator's literal "**every single corpus input**", not merely the value-bearing subset. The only
gate is whether `quote` can reify `E`'s syntax (see §3).

**Why now:** `quote` currently **declines on every collection literal** (`#list`/`#map`/`#set`/
`#tuple`/record) — the reifier has no arm for `Leaf::Ctor`/`FieldPair`/`Member` and bails at
`quote.rs:537` (`_ => None`). That is exactly the seq74/75 work `v-metaprogramming` was tasked with
(operator-greenlit again via the seq-286 stable-code direction). This pass becomes the **acceptance
gate** that drives and protects it: each syntactic form flips `Todo → Pass` as its quote support
lands.

## 2. The design — two functions, caller-boundary round-trip, anti-const-fold verified

For each eligible case with input `E`, the pass emits a program with **two exported functions** whose
signatures put the binary AST on the wire as **opaque bytes** (`list<u8>` at the WIT boundary), so the
compiler cannot see through the round-trip:

```
export encode_quoted : func() -> list<u8>
    = Ast.encode (quote E)                 ;; quote E, encode to binary AST, return bytes to the caller

export decode_check   : func(bytes: list<u8>) -> bool
    = match Ast.decode bytes {             ;; decode the bytes the caller passed BACK in
        Ok a  => a == (quote E)            ;;   assert decoded AST == the quoted AST value
        Err _ => false
      }
```

The **grader is the "caller."** Per eligible case the pass's exec harness runs (§4):

1. **positive trial** — call `encode_quoted()` → get bytes `B`; call `decode_check(B)` → **assert
   `true`**. This is the round-trip identity `decode(encode(quote E)) == quote(E)`, with encode and
   decode split across the boundary so the encode→decode path is **not** a single const-foldable
   expression.
2. **negative trial (the anti-const-fold *verification*)** — call `decode_check(corrupt(B))` →
   **assert `false`** (or a decode trap). `corrupt(B)` is a targeted byte flip / truncation that
   forces a decode error or a structural mismatch. **If the compiler had const-folded `decode_check`
   to a constant `true`, this trial would wrongly pass** — so the negative trial is a live witness
   that decode genuinely executes at runtime on caller-supplied bytes. This makes the operator's
   anti-const-fold requirement a first-class, *tested* property, not just a structural hope.

**Why the boundary defeats const-fold.** `encode_quoted`'s body may legitimately const-fold (`quote
E` is a compile-time constant), but it still returns real bytes to an external caller. The decode
happens in a *separate* export, reading bytes the compiler treats as **runtime input from outside the
component** — it cannot fold `Ast.decode(bytes)` to a constant, so the round-trip genuinely runs. The
negative trial proves it.

`Ast` values support structural `=` (corpus `12-metaprogramming.sexp`: `(= (quote 42) (Ast.Int 42))`
→ `true`); `Ast.decode` returns `(Result Ast e)` (`resolved.rs:495`, lowered `lower.rs:1484`).

## 3. Eligibility (which inputs the pass emits a program for)

Include a case iff its input `E` is a **single quotable form**:

1. `E` **parses** as one S-expression form (essentially all corpus inputs do; a rare *parse*-error
   case is excluded — a form that does not parse cannot be quoted). `expect-kind` is otherwise
   **irrelevant** — value / trap / error / declines / warning inputs are all eligible, because the
   pass never evaluates `E` (§1).
2. `quote` can **reify** `E`. Where it currently cannot (any collection literal, until seq74/75
   lands; `unquote-splicing`, `quote.rs:631`), the emitted program declines to compile and the case
   grades **Todo** — honestly recorded, not Fail (§5).
3. Multi-form inputs (a corpus `(input …)` carrying several top-level defs) are wrapped as a single
   quotable form — quote them as one enclosing form (`(do …)` / the module form itself). **v1
   default: single-top-level-form inputs; multi-form wrapping is increment 5** (see §7).

This is a superset of my earlier value-only sketch — the operator's AST-round-trip design (vs an
eval-identity) is what unlocks "every single corpus input".

## 4. Architecture — a new shred→build→exec→aggregate layer

"Another pass in the shredding tests": reuse the per-case corpus shred architecture
(`design/DESIGN-corpus-nix-per-case-caching.md`), modeled on the worked example of a second exec
layer over the shred, `mkCorpusCadenzaBuild`/`mkCorpusCadenzaExec` (`flake.nix:2959`/`3024`,
explicitly "mirrors mkCorpusBuild/Exec, reuses the shred + wasm baseline + cdz-run grader").

```
corpus file ─shred(quote-wrap)─▶ per-case artifacts ─build─▶ two-export wasm ─exec─▶ pass/fail ─▶ aggregate
             (cdz-corpus: emit    (program.ast = the        (cdz-compile)     (bespoke: 2-call
              the 2-export prog     2-fn round-trip prog                        boundary round-trip
              + wit-world)          + wit-world.ast)                           + negative trial)
```

- **shred** — a **distinct** shred `mkQuoteCorpusShred` (unlike the cadenza pass, which reuses
  `mkCorpusShred`'s `program.ast` verbatim, this pass emits a **different** program, so it re-runs the
  parser once per corpus file — cache-keyed on the file + the wrap logic). It runs `cdz-corpus records
  --out-dir --quote-wrap`. For each eligible case it takes the input form `E`
  (`normalize_program_text_and_ast`, `cdz-corpus/src/lib.rs:1265`) and **synthesizes the two-export
  program** (`encode_quoted`/`decode_check` around `quote E`) plus its `wit-world.ast` (the two
  exports, bytes boundary), emitting them as binary AST **before `codec::encode`**. The case's expected
  clause is replaced by the pass's own fixed round-trip assertion (positive + negative trials).
- **build** — reuse the existing per-case build (`cdz-compile`, binary-AST passthrough). A case whose
  `quote E` declines is captured as a decline → graded Todo.
- **exec** — a **bespoke** exec harness (a `cdz-run` mode, or a small pass-specific runner like the
  cadenza exec) that is the *caller*: it instantiates the component, calls `encode_quoted()`, threads
  the returned bytes into `decode_check(bytes)` (asserting `true`), then calls
  `decode_check(corrupt(bytes))` (asserting `false`/trap). This output→input threading + the corrupt
  witness is the one genuinely-new mechanism; localizing it in the pass's own exec keeps the generic
  corpus `(call …)` DSL unchanged.
- **aggregate** — `checks.<sys>.quote-corpus`, per-case × backend verdict + counts, mirroring the
  corpus aggregate (`corpusCadenzaAll`, `flake.nix:3090`).

Everything stays in native **binary-AST** end-to-end (the shred encodes, `cdz-compile` is a
passthrough, and the value on the wire between the two exports *is* binary AST) — the pass is itself a
direct exercise of the "binary AST is THE data-exchange format" directive.

## 5. Baseline & grading

The pass gets its **own** additive-only baseline `spec/semantics/.quote-gate-baseline` (mirroring
`.gate-baseline`; a new `.#quote-corpus-verdicts` regenerator mirroring `.#corpus-verdicts` /
`mkCorpusVerdict`, `flake.nix:3155`). Grading uses the existing `Todo/Pass/Fail` model:

- `quote E` declines (collection literals today) → **Todo** (expected; not Fail).
- `Todo → Pass` = new quote/codec capability landed (progress; regenerate baseline).
- `Todo → Fail` / `Pass → Fail` = a **round-trip regression**: the positive trial mismatched/trapped
  (a quote-reification or codec bug), OR the **negative trial passed** (an anti-const-fold regression —
  decode got folded away). Reported by case tag, same as the corpus grader.

## 6. Increments (top-to-bottom, vertical-owned)

1. **Program synthesis + eligibility in `cdz-corpus`.** `--quote-wrap` mode: filter to single
   quotable-form inputs, synthesize the two-export `encode_quoted`/`decode_check` program + its
   `wit-world.ast` around `quote E`, emit as binary AST. Rust unit tests over a scalar (Pass), an
   arithmetic form (Pass), a collection-literal form (expected decline → Todo).
2. **Bespoke exec harness + shred/build/aggregate wiring in `flake.nix`.** `mkQuoteCorpusShred`/
   `Build`/`Exec` + `checks.<sys>.quote-corpus`; the exec threads `encode_quoted → decode_check` and
   runs the positive trial. Establish `.quote-gate-baseline` + `.#quote-corpus-verdicts` (most
   collection cases start Todo — recorded honestly).
3. **Anti-const-fold negative trial.** Add the `corrupt(bytes)` witness (`decode_check(corrupt) ==
   false`/trap) to the exec; a Fail if it ever passes. This is the operator's first-class
   anti-const-fold *verification*.
4. **CI/`checks` integration + baseline hygiene** with `v-corpus-harness` (trap/verdict parity,
   regression gate reads the new baseline).
5. **Multi-form input wrapping** (widen §3 clause 3 beyond single-top-level-form inputs).
6. **(Drives, does not own) collection-literal quote capability.** As `v-metaprogramming` lands
   seq74/75 (`quote.rs:537` compound-ctor arms → the already-existing `Ast` variants
   `ListCtor`/`MapCtor`/… `ast_reflect.rs:948-955`, codec kinds `codec.rs:128-132`), cases flip
   Todo→Pass; this pass is the acceptance net.

## 7. Open decisions

- **Crux — meaning of "returns the correct value": RESOLVED by the operator** — the two-function
  binary-AST caller-boundary round-trip with anti-const-fold as a first-class goal (§2). Not the
  eval-identity; no `eval`.
- **Anti-const-fold verification mechanism (my proposed default, flag for operator):** the negative
  corrupt-bytes trial (§2 trial 2) as the explicit *verification*. Alternative/complement: assert the
  emitted `decode_check` wasm contains a real `Ast.decode` call (a wasm-shape check). Default: the
  negative trial (behavioral, backend-agnostic).
- **Eligibility breadth (my default, flag for operator):** cover **every** single-quotable-form input
  regardless of `expect-kind` (§3), per "every single corpus input". Narrower alternative: value cases
  only. Default: all.
- **v1 form scope:** single-top-level-form inputs; multi-form wrapping deferred to increment 5.
- **Doc location:** `/design/` (repo-root infra designs) alongside
  `DESIGN-corpus-nix-per-case-caching.md` / `DESIGN-test-shred-per-test-caching.md`, not
  `implementation/design/`, because it is corpus/shred infra.

## 8. Seams / file anchors

- Quote reifier + the collection gap: `rcdzc/src/quote.rs` (`reify_quotes:146`, `reify_inner:462`,
  `_ => None` collection bail `:537`, `unquote-splicing` defer `:631`).
- Binary-AST codec: `cadenza-ast/src/codec.rs` (kinds `:94-132`, incl. compound-ctor `:128-132`);
  `Ast.encode`/`Ast.decode` prims `rcdzc/src/resolved.rs:495`, lowered `lower.rs:1466`/`:1484`;
  variant↔kind map `rcdzc/src/lower/ast_reflect.rs:977`, variants `:948-955`.
- Shred writer: `cdz-corpus/src/cli.rs` (`records --out-dir` `:346`, `program.ast` write `:392`,
  `expect_kind:564`); `Record.program_ast` `cdz-corpus/src/lib.rs:40`; input form seam
  `normalize_program_text_and_ast` — single-input `lib.rs:1265` vs module siblings `:960`/`:978`.
- Grader: `cdz-corpus-grade/src/lib.rs` (`grade_trial:586`); `cdz-run/src/grade.rs`.
- Pass wiring example (a second layer over the shared shred): `flake.nix:2959`/`3024`
  (`mkCorpusCadenzaBuild`/`Exec`); base `mkCorpusShred`/`Build`/`Exec` `:2790`/`2809`/`2915`;
  aggregate `corpusCadenzaAll` `:3090`; verdicts `mkCorpusVerdict` `:3155`.
- Round-trip identity precedent: `spec/semantics/12-metaprogramming.sexp` (`=` over `Ast`, quote
  cases from `:7`).
