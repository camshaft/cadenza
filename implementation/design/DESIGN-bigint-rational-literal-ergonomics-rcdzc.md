# DESIGN: BigInt / Rational literal ergonomics — Rust-style explicit suffixes

Status: EXPLORATION (no code changed). Operator's ask, refined:
*"easy bigint/rational literals + math operators just work"* → **"I'd prefer to be explicit.
Support literal suffix annotations like Rust."** So: NO auto-widen. A suffix is an opt-in,
per-literal type tag — `100N` is a BigInt because you *said so*, not because it overflowed.

## The one-sentence finding

**The operators already just work; a suffix is just a compact spelling of an existing
annotation.** `+ - * /` over `BigInt`/`Rational` already infer + lower correctly
(`infer.rs:2607`). And `(: 100000000000000000000 BigInt)` **already grounds today** — a
suffix `100000000000000000000N` is exactly that annotation, carried on the literal instead
of wrapped around it. So the BigInt suffix is nearly free; the Rational suffix needs one new
grounding rule (annotate-a-number-as-Rational), which does NOT exist yet.

## Measured seam (today)

| Spelling | Today | Note |
|---|---|---|
| `(: 100000000000000000000 BigInt)` | ✅ grounds to BigInt | suffix desugars to THIS |
| `(: 5 Rational)` | ❌ CDZ0203 mismatch | **needs a new grounding rule** |
| `(: 0.5 Rational)` | ❌ CDZ0203 mismatch | decimal→exact-rational grounding, new |
| `100N` (a suffixed token) | lexes as `Int` + `Ident` `N` | lexer stops before letters |

## Proposed surface

Rust puts the type on the literal: `100i64`, `1.5f32`. Cadenza analogue — a **single-letter
type suffix** glued to a numeric literal:

- **`N` → BigInt.** `100N`, `100000000000000000000N`, `0xFFN`. (`N` for the unbounded
  natural/integer; uppercase so it never collides with a hex digit `a–f`/`b`/`e`, and reads
  as a "big" marker.)
- **`R` → Rational.** Two forms:
  - on an **integer** literal: `5R` = the exact rational `5/1`.
  - on a **decimal** literal: `0.5R` = the exact rational `1/2` (lossless — `Decimal` is
    already `significand·10^exp`, so `0.5R → 1/2`, `1.25R → 5/4`, `0.1R → 1/10`).
  - `1/2` as a *literal* is deliberately NOT introduced (see Rejected); write `0.5R` or the
    constructor `(Rational.of 1 3)` for a non-decimal fraction.

Suffixes compose with the existing base prefixes (`0xFFN`) and separators (`1_000N`). They
do NOT stack (`100NR` is malformed). Lowercase `n`/`r`? — pick ONE case to keep one canonical
spelling (the round-trip/garbage-render rule). **Recommend uppercase `N`/`R`**; the printer
emits the canonical case and a lowercase input is a lexical defect (or is simply not matched
and falls through to a `Name`, i.e. `100n` → error downstream — safest).

## Where each piece lands

Three layers, each already has the exact hook:

1. **Lexer** (`cadenza-syntax/src/lexer.rs::number`): after the digits/`.`/exponent scan,
   consume ONE trailing suffix letter from a closed set `{N, R}` when it's glued (no space).
   Emit a distinct token kind, OR keep `Kind::Int`/`Kind::Float` and let `literal.rs` read
   the suffix off the text (simpler — the text already includes it, cf. how `parse_int`
   inspects the whole `tok`). Precedent: this lexer already scans the `+.`/`%` operator
   suffixes, so a trailing-char suffix is an established shape here.
2. **Classify** (`cadenza-syntax/src/literal.rs::classify_word_nonname`): before the
   int/float parse, peel a trailing `N`/`R`; parse the numeric body as today; produce a
   leaf that carries the requested type. Cleanest: a NEW `Leaf` need NOT be added if the
   suffix desugars in the *reader* to an annotation node `(: <lit> BigInt)` / `(: <lit>
   Rational)` — then NO leaf/codec/round-trip surface changes and the value flows through
   the already-working annotation-grounding path. **This is the key simplification: a suffix
   is reader sugar for an annotation, exactly like `#name` is sugar for `#"name"`.**
3. **Typing** (`infer.rs`, the grounding site ~line where an int-lit-annotated-`BigInt`
   grounds): BigInt side already works. **Add the Rational-annotation grounding**: a numeric
   literal annotated `Rational` grounds to the exact rational — an integer `k` → `k/1`; a
   decimal `significand·10^exp` → `significand / 10^|exp|` normalized (reuse the existing
   `IntValue` gcd/normalize that `Core::ConstRational` folding already uses in `lower`).
   This is the same "Annotations Constrain" rule as the BigInt case, extended to Rational,
   and lossless because the decimal is exact.

## Why this fits the operator's rules

- **No keys outside the prelude.** The suffix is a *reader desugar* + a *typing default at
  the annotation site* — no `if name=="…"`, no per-name branch in infer. `N`/`R` are a
  closed lexical set in the lexer, the same category as `0x`/`0b` radix prefixes and the
  `+.`/`%` operator suffixes.
- **One value ⇒ one spelling (garbage-render).** The printer emits the canonical suffix
  (`100N`, `0.5R`); a suffixed literal round-trips. Because the suffix desugars to an
  annotation the compiler already understands, there is no second internal representation.
- **Explicit, not implicit.** A bare over-wide literal still errors (CDZ0201) — you opt in
  with `N`. No silent widening, no cross-operand coercion surprise.

## Rejected

- **Auto-widen an over-wide literal to BigInt** — operator explicitly prefers explicit. Also
  raised a "does `huge + 1` silently promote the `1`?" coercion question that the suffix
  sidesteps entirely (you write `1N`).
- **`1/2` fused rational literal** — needs a new `Leaf` variant (touches every leaf match,
  codec, both printers) and introduces `1/2` vs `1 / 2` ambiguity. `0.5R` covers the decimal
  cases losslessly; `Rational.of` covers `1/3`. Deferred as its own vertical if ever wanted.
- **`/` returns Rational for integer literals** — silently changes integer division; can't
  distinguish `(/ a b)` from `(/ 1 2)` without a leaky literal special-case. Conflicts with
  the pinned "no silent promotion" corpus case.

## Build order (smallest shippable first)

1. **Rational-annotation grounding in `infer.rs`** (unblocks `(: 5 Rational)` / `(: 0.5
   Rational)`) + corpus cases. Pure typing, no syntax — valuable on its own, and the suffix
   rides it. *This is the one genuinely-missing piece; everything else is sugar over it.*
2. **`N`/`R` suffix in the lexer + reader desugar to an annotation** + round-trip printer.
3. Corpus: `100N` is a BigInt; `100000000000000000000N` needs no annotation; `0.5R` is
   `1/2`; `5R` is `5/1`; `(+ 100N 1N)` computes; a lowercase/stacked suffix is rejected.

### Note on scope vs. the Rational vertical
The runtime-valued Rational vertical (param / computed / boundary-crossing / collection
element) is COMPLETE and verified. This literal-ergonomics work is a *separate* front-end
vertical — it adds no runtime/backend surface (the types already lower); it only makes the
values easier to write.
