## 31. 🟢 The language needs non-trapping `checked` arithmetic — LANDED (seed) — awaiting loop re-probe

> **⏳ PENDING VALIDATION 2026-07-07 (filed and fixed same day).** `Int64.checked-add/sub/mul :
> (Int64,Int64) -> Option<Int64>` are realized. Verified: ok → `Some` (`checked-add 20 22` → 42,
> `checked-mul 6 7` → 42, `checked-sub 50 8` → 42); overflow → `None` (`checked-add max 1`,
> `checked-sub min 1`, `checked-mul max 2`, `checked-mul min -1` all → the `None` arm); and it works on
> RUNTIME params (`(def (chk a b) (match (Int64.checked-add a b) ((Some v) v) (None -1)))`). **To confirm
> → done:** pin the corpus cases (`(Int64.checked-add Int64.max 1)` → `(None …)`; `(Int64.checked-mul 6
> 7)` → `(Some 42)`).
>
> ⚠️ **BUT the overflow-trapping LOWERING it was meant to unblock is STILL blocked — on a different gap.**
> To make `compiler.cdz`'s `+`/`-`/`*` TRAP on overflow, the trapping helper is
> `(match (Int64.checked-add a b) ((Some v) v) (None <diverge>))` — but the `None` arm needs a realized
> DIVERGING expression, and there is none usable at runtime: `(trap "…")` is unrealized (`undeclared
> capability: trap` — the `never` capability, `(needs never)`-skipped in the corpus), and
> `Option.expect` on a RUNTIME Option DECLINES (new gap — see the sibling ask). `match` on the runtime
> checked-result works, so the ok path is fine; only the divergent overflow arm is unwritable. So
> overflow-trapping lowering now waits on a realized `trap`/`never` OR runtime `Option.expect`.

## 31. 🔴 The language needs non-trapping `checked` arithmetic (`Int64.checked-add/sub/mul : (Int64, Int64) -> Option<Int64>`)

**Finding.** Cadenza's `+`/`-`/`*` **trap** on signed overflow (numeric-model.md §Overflow Is Defined —
the seed emits `checked_add`/`checked_sub`/`checked_mul` helper functions that `unreachable` on
overflow). There is **no non-trapping way to add/sub/mul and observe overflow** — a Rust-style
`checked_add(a, b) -> Option<Int64>` that returns `None` instead of trapping. Every spelling declines:
```
(Int64.checked-add 20 22)   → declined: unsupported dotted-application
(checked-add 20 22)         → declined: undeclared capability: checked-add
(Int.checked-add 20 22)     → declined: unsupported dotted-application
```
A spec learning already anticipates it (`2026-07-05-self-hosting-is-gated-on-generics…`: *"Int64
checked arithmetic + shifts"* among the library needs), but it is unrealized.

**Why it matters — it's the clean primitive that lets the trapping `+` be WRITTEN IN CADENZA.** The
Cadenza-authored compiler (`compiler.cdz`) must lower a runtime `+`/`-`/`*` so it TRAPS on overflow
(to match native / the numeric model). Today `compiler.cdz`'s `lower` emits a RAW `i64.add`/`i64.sub`/
`i64.mul`, which **wraps** instead of trapping — a real miscompile on runtime overflow (verified: a
`(sq 4000000000)` helper returns the wrapped value where native traps). The seed does it right by
hand-emitting checked helper *functions* in raw wasm — but `compiler.cdz` cannot express those helpers,
because to write `checked_add` you need a **wrapping** add or an overflow *test*, and the language has
neither: its only `+` is the trapping one (circular — you can't build the trapping `+`'s guard out of
the trapping `+`). Inventing internal-only IR nodes (`KWrapAdd`/`KXor` with "no compile-time op") to
paper over this was rejected as IR pollution / a workaround.

**With `checked-add → Option`, the trapping `+` becomes an ordinary Cadenza function** the compiler can
emit and all passes handle normally:
```
(def (add-or-trap a b)
  (match (Int64.checked-add a b) ((Some v) v) (None (trap "integer overflow"))))
```
`compiler.cdz` then routes each source `+`/`-`/`*` to a `KCall` of the appropriate appended helper — no
raw wasm in the IR, fold/kind-of/lower/count-lets all work unchanged. This is the clean, composable
foundation; `checked_*` is also directly useful in ordinary Cadenza (overflow-aware code without a trap).

**Proposed resolution (seed + realized-set + spec).** Realize `Int64.checked-add`, `Int64.checked-sub`,
`Int64.checked-mul : (Int64, Int64) -> Option<Int64>` — `None` exactly when the true mathematical result
is outside `[Int64.min, Int64.max]`, else `Some result`. (Widths later generalize to `(Int N)`/`(UInt
N)`; Int64 is the self-host need.) The seed already has the overflow-detection logic inside its
`checked_*_body` helpers — this ask exposes that detection as a **value-returning `Option`** rather than
an internal trap. Spec: a numeric-model.md clause *"Overflow-Checked Arithmetic Returns An Option"* +
corpus cases (`(Int64.checked-add Int64.max 1)` → `(None …)`; `(Int64.checked-mul 6 7)` → `(Some 42)`)
+ seed lowering. Related: shifts and the trapping-lowering both want the same guard-emission machinery.

**Status.** 🔴 **Seed + realized-set (and a small spec clause).** Blocks `compiler.cdz` from lowering
runtime `+`/`-`/`*` to trap-on-overflow (currently a wrapping miscompile). Pin with the two corpus
cases above once landed. (NOTE: distinct from the *trapping* `+` semantics, which already exist — this
is the non-trapping `Option`-returning companion, the primitive the trapping form is built on.)

**🟢 LOOP-CONFIRMED 2026-07-07 (Run 63).** Re-probed: `(match (Int64.checked-add 20 22) ((Some v) v) ((None _)
-1))` → 42 (ok arm), overflow → None arm. The 12 sibling corpus cases pass (behavior gate 561). Moved
pending-validation → done.
