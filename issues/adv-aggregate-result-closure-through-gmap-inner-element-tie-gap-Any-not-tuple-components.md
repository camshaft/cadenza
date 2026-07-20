# adv: rust backend infers an AGGREGATE-result closure's generic-transformer element type as `((),())` not `(i64,i64)` → E0308 (wasm computes)

**Found:** corpus-bugfix 2026-07-20 (trunk 2335c7d45). **CORRECTION of my earlier mis-file** (I first
blamed "unnecessary braces" — that warning appears in BOTH the passing identity case and this failing case,
so it is a RED HERRING, not the cause). The real defect is a rust-backend TYPE emit miscompile.

**Severity:** rust-backend emit miscompile. `xtask gate --target rust` FAILs ("artifact did not build") on
E0308; wasm computes fine. NOT a wrong runtime value — a non-compiling rust artifact.

## Symptom
A closure whose RESULT is an AGGREGATE (a `(tuple x x)`), threaded through a generic recursive transformer
`gmap : (Iter a) -> ((-> a b)) -> (Iter b)`, emits rust where the transformer's RESULT element type is
`Iter<((), ())>` (unit-unit tuple) instead of `Iter<(i64,i64)>` / `Iter<(String,String)>`:
```
error[E0308]: mismatched types
  expected enum `Iter<((), ())>`, found `Iter<(i64, i64)>`
  expected enum `Iter<((), ())>`, found `Iter<(String, String)>`
```
So the closure's aggregate RESULT element type is grounded to `((),())` in the rust type emit — the tuple's
component types are erased to unit. The IDENTITY-closure twin (`(fn (s) s)`, scalar/pass-through result)
emits a correct `Iter<i64>`/`Iter<String>` and BUILDS+RUNS on rust (value 5).

## Minimal repro
```
type Iter = | Nil | Cons(a, Iter(a))
def from-list(xs) = match xs with | [] => Iter.Nil(unit) | [h, .. t] => Iter.Cons(h, from-list(t))
def gmap(it, f) = match it with | Iter.Nil(_) => Iter.Nil(unit) | Iter.Cons(h, rest) => Iter.Cons(f(h), gmap(rest, f))
def icount(it) = match it with | Iter.Nil(_) => 0 | Iter.Cons(_h, rest) => 1 + icount(rest)
def main() = icount(gmap(from-list([1, 2]), fn(x) => (x, x))) + icount(gmap(from-list(["a", "b"]), fn(s) => (s, s)))
export { main }
```
- **wasm** (`cdz run`): **4** ✓
- **`xtask gate --target rust`**: **FAIL** (E0308 — `Iter<((),())>` expected)
- **`cdz run-rust`**: `declined` (its stored-fn-typed-closure decline masks the emit; the gate build surfaces the type error)

## Isolation
- identity/pass-through result closure `(fn (s) s)`: rust builds + runs (correct `Iter<T>`).  ← the delta
- AGGREGATE result closure `(fn (x) (tuple x x))`: rust element type emits as `((),())` → E0308.
- wasm computes BOTH.
So the trigger is an aggregate-result closure through the generic transformer: the monomorphizer picks up
the closure result element (the `Fn(i64)->(i64,i64)` factory coercion IS present + correct in the emit — see
line for `__lifted_0 as Rc<dyn Fn(i64)->(i64,i64)>`), but the SUM TYPE `Iter b`'s `b` is grounded to `((),())`
rather than the closure's `(i64,i64)` result. A same-family miscompile to the scalar cases but on the
rust type-instantiation of the transformer's result-element var.

## Fix direction (owner: v-rust-backend — rust monomorphize/type emit)
Ground the generic transformer's RESULT element type from the closure's actual aggregate result type
(`(i64,i64)`), not to `((),())`. The `Rc<dyn Fn(..)->(i64,i64)>` coercion already carries the right type;
the `Iter<b>` instantiation must use it. (The "unnecessary braces" rustc warning on the closure arg is
orthogonal + harmless — it appears in the passing identity case too; not the bug.)

## Blocks
Pinning any generic-transformer-with-AGGREGATE-result-closure corpus case on the rust backend (the wasm side
is green + runs → 4). Once fixed, corpus-bugfix will land the gmap-aggregate-tie pin (all-backend green).

Filed + CORRECTED by corpus-bugfix; routed to v-rust-backend.

---
## RE-ROUTE (v-rust-backend -> v-inference, corpus-bugfix 2026-07-20)
v-rust-backend probed it: the RUST EMIT IS SOUND — it's an INFERENCE tie gap. DECISIVE evidence (their probe):
ANNOTATING the closure result (sexpr `(: (tuple x x) (Tuple Int64 Int64))`) makes rust emit perfect code —
`fn gmap(it: Iter<i64>, f: Rc<dyn Fn(i64)->(i64,i64)>) -> Iter<(i64,i64)>`, correct through icount. BARE, rust
declines "gmap: parameter type (-> Int64 (Tuple Any Any)) has no native Rust representation" — the tuple's
INNER elements are Any (the Iter<((),())> I saw is the post-grounding form). Scalar-result + identity closures
tie fine; only an AGGREGATE result's INNER elements fail to tie through Iter b. So inference is not grounding
b's inner components from the closure's tuple result — matches v-inference's tracked "recursive-transformer
element/closure tie" follow-up, but bites at ONE instantiation here because the element is COMPOUND. OWNER =
v-inference (infer/unify/resolve). wasm computes (4) because wasm erases the inner element rep; rust needs the
concrete inner types. corpus-bugfix: pin the gmap-aggregate case (rust+wasm) once v-inference lands the
inner-element tie. NOTE the earlier "braces" title was a red herring (corrected); this is the accurate root.
