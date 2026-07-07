# A `bytes → bytes` compile entry unblocks the real differential harness — the seam is landed, the self-hosted compiler just hasn't moved onto it yet

*2026-07-07*

**What happened.** The interim corpus harness (patch AST bytes into `compiler.cdz`'s nullary `main`, emit, read
the runtime `Value` it built) was always a stopgap — slow (one full `compiler.cdz` compile per case),
`quote`-limited, and driving the compiler through a channel it will never ship. The *real* harness the seed
already has — `cadenza-seed component-check <compiler-component> spec/semantics`, which feeds each case's AST
bytes to a `compile : list<u8> → list<u8>` component and diffs against native — was blocked on one thing: the
seed could only lift a **nullary** entry (`run : () → output`), so it could not build a Cadenza-authored
compiler as a `bytes → bytes` component (SEED-GAPS **gap 3l**). That gap is now **RESOLVED on the seed side**,
and probing the rebuilt seed directly confirmed it works end-to-end:

- The seed's entry selection now accepts a def named `compile` with one `Bytes`/`list<u8>` param → `Bytes`, and
  lifts it as `cadenza:compiler/compile : func(list<u8>) -> list<u8>` (codegen picks it over `run` by shape).
- A new dev subcommand `cadenza-seed compile-run <compiler.cdz> <input.cdz>` builds the compiler as a compile
  component and drives it over the second program's canonical AST bytes.
- **Verified:** an identity compiler `(def (compile b) b)` builds a **valid 3,059-byte** compile component;
  driven over `(module m (def (main) 42))` it returns the input's **32 canonical AST bytes** unchanged
  (`83 01 84 63 64 65 66 …`, the exact CBOR prefix `compiler.cdz`'s `main` hardcodes). The list ABI round-trips
  through linear memory: input bytes → runtime `Bytes` handle → user `compile` → result bytes → retptr.

So the machinery is real. But a second probe caught the seam that is NOT yet crossed: **`compile-run` on the
actual `compiler.cdz` fails** — `VALID compile component (29728 bytes)` then `compile run error: expected 0
argument(s), got 1`. The committed `compiler.cdz` still has a **nullary `(def (main) …)` entry with the target
program's bytes hardcoded in the body**, so the seed lifts it as `run`, the host falls back to the top-level
`run` export, and then drives it with the 1-byte-list argument a `compile` entry would take — arity mismatch.
The compiler hasn't been rewired from "nullary `main` that compiles a baked-in program" to "`(def (compile b)
(compile-bytes b))` that compiles its argument."

**Why.** This is the ordinary shape of a capability landing **one seam at a time**: the seed grew the ability to
*host* a `bytes → bytes` compiler before the Cadenza compiler moved its entry onto that ABI. The two are
independently correct right now — the seed's lift + list-ABI marshalling round-trips an identity compiler, and
`compiler.cdz` compiles its baked-in module to the right 89-byte component through the old nullary path — they
are just not yet *connected*. The rewire is a one-line entry change (`(def (main) …hardcoded bytes…)` →
`(def (compile b) (compile-bytes b))`), and `compile-bytes` — the whole `read-module → resolve-module → fold →
lower → serialize → frame` pipeline — already exists and takes exactly a `Bytes` argument. The reason it hasn't
happened is worth recording: the *interim harness still works on the nullary form*, so there's no forcing
function until the interim harness is retired, and the full `component-check` run additionally needs the value-
heap **runtime component** to build (it currently doesn't — CHAMP set ops are mid-implementation), so the
end-to-end corpus diff over the compile component is blocked on a *different*, unrelated in-flight change. The
honest status: **gap 3l is resolved and the compile ABI works; the real differential harness is unblocked in
principle; two mechanical steps remain — rewire `compiler.cdz`'s entry to `compile`, and get the runtime
component building again — neither of which is a language or compiler-correctness gap.**

**The requirement it drove.** No corpus case: a `bytes → bytes` entry is an **ABI/entry-shape** contract, not a
scalar-value behavior, so it is not expressible in the `(output (: v T))` corpus oracle model — forcing one
would be the anti-pattern this loop warns against. The durable outputs are this learning and SPEC-BACKLOG
updates: the harness-blocker item (the interim harness exists ONLY because 3l was open) is marked resolved on
the seed side with the two remaining mechanical steps named, and the component-ABI concern — that a module may
export a single `compile : list<u8> → list<u8>` entry selected by the def name `compile` (as `run` is selected
by the name `main`), lifted through the canonical list ABI — is recorded for the operator to fold into
`contracts/component-abi.md` (the spec carries no proper names, so the requirement is "a bytes-to-bytes entry
export," not "the `compile` function"). General lesson, and a companion to the entry-selection finding of the
previous cycle ([[the-self-hosted-reader-compiles-a-multi-def-call-but-picks-the-entry-by-position]]): **a
resolved gap is a *capability*, not a *connection* — probing "is 3l fixed?" answers yes for the seed and no for
the end-to-end loop, and only running the actual artifact (not reading the handoff banner's "VERIFIED
end-to-end") distinguishes the two.** The banner described a compiler.cdz that had been rewired for the
verification and then reverted; the committed file tells the real current state.
