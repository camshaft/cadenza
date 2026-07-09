## 32. 🔴 `Option.expect` declines on a RUNTIME Option (const-only) — while `match` on the same runtime Option works

**Finding.** `Option.expect : (Option<T>, String) -> T` (unwrap-or-trap) is realized only for a
**compile-time-constant** Option. On a **runtime** Option it declines *"unsupported dotted-application"*.
`match` on the very same runtime Option works. Minimal boundary:
```
(def (g o) (Option.expect o "m"))            (g (Some 7))      ; → declined: unsupported dotted-application
(def (g o) (match o ((Some v) v) (None -1))) (g (Some 7))      ; → 7  (match on a runtime Option: OK)
(Option.expect (Some 42) "m")                                  ; → 42 (const Option: OK)
(Option.expect (Int64.checked-add 20 22) "m")                  ; → 42 (const-foldable arg: OK)
(def (f a b) (Option.expect (Int64.checked-add a b) "m")) (f 20 22)  ; → declined (runtime arg)
```
So the gap is precisely: `Option.expect` where the Option value is not a compile-time constant. The
corpus only exercises it on const-foldable args (`(Option.expect (Bytes.slice (Bytes.of …) 1 2) …)`,
`(Option.expect (String.slice "hello" 0 2) …)` — all literal), so the runtime path was never pinned.

**Why it matters — it (with the `trap`/`never` gap) blocks overflow-TRAPPING arithmetic lowering.**
`Int64.checked-add/sub/mul → Option` just landed (ask-31), so `compiler.cdz` can now DETECT overflow.
To make a runtime `+`/`-`/`*` TRAP on overflow (numeric-model §Overflow Is Defined), the natural helper is
```
(def (add-ck a b) (Option.expect (Int64.checked-add a b) "integer overflow"))
```
— one expression: `Some` → the value, `None` → trap. That is exactly `Option.expect` on a RUNTIME
Option (`a`/`b` are params), which declines. The alternative — `(match (Int64.checked-add a b) ((Some v)
v) (None <diverge>))` — needs a realized DIVERGING expression for the overflow arm, but `(trap "…")` is
unrealized (`undeclared capability: trap`; the `never` capability is `(needs never)`-skipped). So either
**runtime `Option.expect`** OR **a realized `trap`/`never`** unblocks trapping arithmetic; runtime
`Option.expect` is the cleaner single primitive (and generally useful — unwrap-or-trap on any runtime
Option, e.g. a `List.at`/`Bytes.at`/`checked-*` result the program asserts is present).

**Proposed resolution (seed).** Lower `Option.expect` on a runtime Option: it's the `match ((Some v) v)
(None <trap with the message>)` the language already compiles, just as a built-in — the runtime Option
is a heap sum the seed already `match`es (the decline is specific to the `.expect` accessor path, not
the value). Align the `Option.expect` accessor lowering with the runtime `match` lowering. Add a corpus
case: `(def (g o) (Option.expect o "m")) (g (Some 7))` → 7, and `(g (None …))` → trap.

**Status.** 🔴 **Seed (accessor lowering).** Sibling of ask-31 (checked arithmetic, just landed) — together
they unblock overflow-trapping lowering in `compiler.cdz` (currently raw `i64.add` that WRAPS, a known
miscompile). Pin with the runtime `Option.expect` corpus case above.

**🟢 LANDED + LOOP-CONFIRMED 2026-07-07 (seed).** Fixed in the seed's `gen_dotted_apply`: NEW
`gen_option_expect` lowers `Option.expect`/`Result.expect` on a runtime optional as the `match ((Some v)
v) ((None _) <trap>)` it desugars to — read `sum-disc(handle)`, if the PRESENT variant (`Some`/`Ok`) yield
`sum-payload` (unboxed per payload kind), else `unreachable` (a defined trap). Payload KIND comes from a
NEW `expect_payload_kind` — a purely syntactic classifier SHARED by codegen AND `infer_list` so the emitted
fn's return kind never disagrees with what its caller reads (`Int64.checked-*`/`Bytes.at` → unbox Int64;
else the raw payload handle, rendered via the scrutinee's `Some`-payload shape). Declines on the scalar path
⇒ runtime-mode retry, like every value-heap producer. Re-probed on the live seed:
```
(def (g o) (Option.expect o "m")) (g (Some 7))                      → 7
(def (g o) (Option.expect o "m")) (g (None unit))                   → trap
(def (add-ck a b) (Option.expect (Int64.checked-add a b) "overflow")) (add-ck 20 22)         → 42
                                                                     (add-ck Int64.max 1)    → trap  ← the overflow-TRAPPING primitive ask-37 wants
(def (g r) (Result.expect r "m")) (g (Ok 99))                       → 99
```
5 corpus cases pinned in `02-binding-and-control.sexp` (runtime Some/None, checked-add feeds `+`, checked
overflow trap, Result Ok). All 4 gates green (behavior 567/0, ignition byte-identical, component-check-vs-Rust
567 agree/0 disagree/0 decline, cargo test). Learning: [[runtime-option-expect-unwrap-or-trap]]. Moved
open → done. **→ This is the clean fix for ask-37**: `compiler.cdz` can now lower a trapping `+` as
`(Option.expect (Int64.checked-add a b) "overflow")` — one expression, Some→value / None→trap — instead of
the reverted inline-guard approach that self-trapped. See ask-37's next-step note.
