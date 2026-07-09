## 38. 🟢 `Ast.decode` must be TOTAL (return a Result/Option), not trap — input can come from an external source; and it must reject trailing bytes into the error case

**Operator direction (2026-07-07):** *"I don't want to trap on AST decoding — this should return a Result… we
can't fail on this because it could come in from an external source."* So `Ast.decode` decodes UNTRUSTED bytes
and MUST be **total**: malformed input yields an error VALUE the program handles, never a trap/decline. This
supersedes any reading of `deterministic-value-form.md` #Decoding Refuses… as a hard failure — "refuse" means
*the error case of a fallible decode*, not a trap.

**Current seed behavior (both wrong under this direction):**

| input | current | should be |
|---|---|---|
| `(Ast.decode (Bytes.of (list 255 255 255)))` — invalid bytes | **TRAPS** (declines) | `Err`/`None` (an error value) |
| `(Ast.decode (encode(Ast.Int 7) ++ [99]))` — valid prefix + trailing | returns `Ast.Int 7` (silently drops trailing) | `Err`/`None` (trailing is an error) |
| `(Ast.decode (Ast.encode (Ast.Int 7)))` — exactly canonical | `Ast.Int 7` | `Ok (Ast.Int 7)` / `Some (Ast.Int 7)` |

So TWO fixes: invalid bytes must not TRAP (→ error value), and trailing bytes must be detected (→ error value,
not silently dropped). Both are the total-decode discipline: a decoder over external bytes returns success-or-
error, and "consumed the whole input exactly" is part of success.

**⚠️ SIGNATURE DECISION FOR THE OPERATOR.** Making `Ast.decode` total changes its type from `Bytes → Ast` to a
fallible form, which ripples to the 9 existing `Ast.decode` corpus cases (they currently write `(= (Ast.decode
…) (quote …))` assuming a bare `Ast`). Options:
- **(A) `Ast.decode : Bytes → Option<Ast>`** — mirrors `String.from-bytes : Bytes → Option<String>` (the
  existing total-decode precedent). Simplest; the error carries no detail. Round-trip cases become `(= (Ast.decode
  …) (Some (quote …)))` or match the `Some` arm.
- **(B) `Ast.decode : Bytes → Result<Ast, <decode-error>>`** — richer; the error case can say why (bad head vs
  trailing bytes vs truncated). Needs a decode-error type. Round-trip cases match the `Ok` arm.
- Recommend **(A)** unless the error detail is wanted — it matches the existing fallible-decode surface
  (`from-bytes`), and a program that cares only "did it parse" is the common case; (B) if diagnostics on
  malformed external input matter.

**Acceptance signal.** `(Ast.decode <garbage>)` and `(Ast.decode <valid++trailing>)` both return the error case
(`None`/`Err`) and a program can `match` on it without trapping; `(Ast.decode <exactly-canonical>)` returns the
success case. Corpus: the round-trip cases update to the chosen signature; two new cases pin the error case for
invalid bytes and for trailing bytes (both value cases now — no trap oracle). **Withheld until the signature is
decided** (operator call) — adding cases in the wrong signature would enshrine it.
Learning: `spec/learnings/2026-07-07-a-new-decode-contract-landed-the-refuse-invalid-half-holds-the-no-trailing-bytes-half-does-not.md`.
Related: `deterministic-value-form.md` decode contract; `String.from-bytes` (the `Option` total-decode
precedent); the reader's `ast::decode` at the `compile` entry (which likewise must not trap on external input).

**🟢 SIGNATURE RESOLVED 2026-07-07 (Run 69) — the spec now mandates OPTION (absence of a value).** A sibling
landed a new capability spec `spec/capabilities/value-interchange.md` that makes the total-decode direction
normative and GENERAL (all values, not just Ast): §"Decode Inverts Serialize And Refuses Otherwise" — *"Decoding
a byte sequence that is not the serialization of any value of the expected type MUST yield the ABSENCE OF A VALUE
rather than a value, consistent with the language's fallible readers that yield an optional result rather than
trapping."* So the signature question is answered: **Option** ("absence of a value" = `None`), matching
`String.from-bytes : Bytes → Option<String>`. `Ast.decode` should be `Bytes → Option<Ast>`.

**Seed still unmet (re-probed Run 69).** `Ast.decode` still returns a bare `Ast` and **TRAPS** on garbage
(`(Ast.decode (Bytes.of (list 255 255 255)))` → trap; a `match … ((Some a) …)((None _) …)` declines "constructor
pattern against unresolved scrutinee" / "match does not cover the scrutinee" — i.e. decode is not Option-typed).
Both clauses remain: (1) invalid bytes → `None` (not trap); (2) trailing bytes → `None` (not silent drop, per
`deterministic-value-form.md`). The trailing-bytes clause lives in deterministic-value-form.md; the
total/Option requirement now lives in value-interchange.md — both unmet.

**Now unblocked for corpus once the seed conforms:** the error-case cases become ordinary VALUE cases (no trap
oracle) — `(match (Ast.decode <garbage>) ((Some _) 1) ((None _) 0))` → 0, `(match (Ast.decode <valid++trailing>)
…)` → 0, `(match (Ast.decode <valid>) ((Some _) 1) …)` → 1 — plus the 9 existing round-trip cases migrate to the
`Some`/match form. Still WITHHELD until the seed makes `Ast.decode` Option-returning (adding them now would FAIL
the gate — decode isn't Option-typed yet). Fix is seed-side: change `Ast.decode`'s signature to `Bytes →
Option<Ast>`, return `None` on invalid bytes AND on trailing bytes (require EOF after the value), migrate the
existing round-trip corpus to `Some`.

**🟢 LOOP-CONFIRMED FIXED 2026-07-07 (Run 71) — landed as `Result<Ast, e>`, both clauses met.** The seed made
`Ast.decode : Bytes → Result<Ast, e>` — TOTAL, never traps. Re-probed: valid → `(Ok ast)` (round-trip via `match
((Ok a) (= a …))` → true), garbage → `(Err reason)` (match → error arm), trailing bytes → `(Err reason)` (`(Ast.
encode (Ast.Int 7)) ++ [99]` → Err, not `Ok (Ast.Int 7)`). Both ask-38 clauses met: invalid → error value (not
trap), trailing → error value (not silent drop). Migrated the 4 existing round-trip corpus cases to the `Ok`
form + added 2 error-case cases (garbage → Err, trailing → Err); gate green 569.

**⚠️ One reconciliation for the operator (spec wording vs implementation choice):** `value-interchange.md` §"Decode
Inverts Serialize And Refuses Otherwise" says decode yields "the **absence of a value**" — which reads as OPTION
(`None`). The seed implemented **`Result<Ast, e>`** (`Err <reason-string>`), carrying the decoder's reason. Both
are total and satisfy "not trapping," and Result is arguably richer (it explains *why* the bytes were rejected).
But the literal spec wording ("absence of a value") is Option-shaped, not Result-shaped. **Operator call:** either
bless Result in the spec (update value-interchange.md to allow an error payload / say "a fallible result", which
matches the seed and is more useful), OR the seed should return Option to match the current wording. Not blocking
— the behavior is correct and total either way — but the spec and implementation should agree on the shape.

**⚠️ Minor seed limitation found while migrating:** a `match` on the decode result with an explicit `((Err _) …)`
arm is REJECTED "CDZ0201: comparison between values of different types" / the compound one hit "CDZ0401:
undeclared capability: g"; the `(else …)` catch-all form works. So the corpus cases use `((Ok …) …) (else …)`.
Worth a look (an explicit Err arm should type like the else), but low priority — filed as a note here, not a
separate ask, since the else form is a clean workaround.

**🟢 LANDED + LOOP-CONFIRMED 2026-07-07 (seed). Signature = Result (operator's call, B).** `Ast.decode`
is now TOTAL `Bytes → Result<Ast, e>`, never traps. TWO seed fixes: (1) `ast::decode` decodes over a
`std::io::Cursor` and rejects TRAILING bytes (`cursor.position() != bytes.len()` ⇒ `Err`); (2) the
`eval_const_dotted` `Ast.decode` arm returns `(Ok ast)` / `(Err <reason-string>)` (was
`Ast`/`ConstTrap`). PLUS a latent bug the Result binder surfaced: `cval_to_node(CVal::Ast)` returned the
BARE node (re-folds to Int/Str, losing Ast-ness), so a decoded `(Ok a)` binder compared Int-vs-Ast and a
nested `(Ok (Ast.Int n))` pattern declined — now wraps `(quote n)` which re-folds to `CVal::Ast` exactly.
This ALSO fixes the note above: an explicit `((Err e) …)` arm now type-checks (no `else` workaround
needed) — verified `(match (Ast.decode <garbage>) ((Ok a) 1) ((Err e) 0))` → 0. Re-probed:
```
(Ast.decode <garbage 255,255,255>)                    → (Err "binary AST decode: …")   [no trap]
(Ast.decode (encode (Ast.Int 7)) ++ [99])             → Err                             [trailing detected]
(Ast.decode (Ast.encode (quote 7)))                   → (Ok (Ast.Int 7))
match … ((Ok a) (= a (quote 7))) …                    → true
match … ((Ok (Ast.Int n)) n) …                        → 7
```
4 corpus cases in `12-metaprogramming.sexp` (2 round-trip → `Ok` arm, 2 error cases: invalid + trailing);
stale "seed declines a constructor-built AST" doc-comments corrected (that const path already works).
All 4 gates green (behavior 569/0, ignition byte-identical, cc-vs-Rust 574/0, cargo test). Learning:
[[ast-decode-total-result-and-cval-ast-roundtrip]]. Moved open → done.

**Still open (NOT this ask, NOT blocking — noted for a future ask):** the RUNTIME AST path — a
runtime-built `(Ast.Int n)` (n a param) renders "unknown sum variant: Ast.Int", and `Ast.decode`/`Ast.encode`
on a RUNTIME `Bytes` declines "unsupported dotted-application". `Ast` is a compile-time-only `CVal::Ast`,
not a registered runtime heap sum type (making it one is M2-scale). NOT on the self-hosting critical path:
`compiler.cdz`'s `compile-bytes b` decodes its runtime input with its OWN `read-module` reader, not the
built-in `Ast.decode`.
