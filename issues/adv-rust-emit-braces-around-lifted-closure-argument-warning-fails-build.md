# adv: rust backend emits BRACES around a lifted-closure function argument → `unnecessary braces` warning → gate build FAILS (wasm computes)

**Found:** corpus-bugfix 2026-07-20 (trunk 0e1dbae71), while probing a gmap aggregate-result tie pin.
**Severity:** rust-backend emit-style defect. `xtask gate --target rust` grades the case FAIL ("artifact did
not build: warning: unnecessary braces around function argument") because the gate builds emitted rust under
`-D warnings`. wasm computes the value fine (4). NOT a wrong-value miscompile — a non-building rust artifact
due to a superfluous-braces emit on a closure passed in argument position.

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
- **`xtask gate --target rust`**: **FAIL** — "artifact did not build: warning: unnecessary braces around function argument"
- **`cdz run-rust`**: `declined` (the run-rust path tolerates it / declines the stored-fn-typed-closure separately; the GATE's stricter build is what surfaces the warning-as-error)

## Precise emit locus
Emitted rust (`cdz compile -t rust`) line for `main`:
```
gmap_mono5(from_list_mono6(vec![...]), { std::rc::Rc::new(move |__a0| __lifted_0(__a0)) as std::rc::Rc<dyn Fn(i64) -> (i64, i64)> })
                                        ^                                                                                          ^
                                        superfluous BRACES around the closure argument
```
The lifted-closure argument is emitted as a braced block expression `{ Rc::new(...) as Rc<dyn Fn...> }` in
FUNCTION-ARGUMENT position. rustc: `warning: unnecessary braces around function argument`. Under the gate's
`-D warnings` build this fails the artifact.

## Isolation
- The EXISTING corpus case "a recursive-generic transformer threading an IDENTITY closure composes at TWO
  element types" (09-functions.sexp, closure `(fn (s) s)` — no cast needed) BUILDS + PASSES on rust (value 5).
  So the braces appear specifically when the closure argument is emitted WITH the `as Rc<dyn Fn...>` coercion
  wrapper (a tuple/aggregate-result or otherwise-coerced closure), which the emitter wraps in `{ ... }`.
- So the trigger is a closure argument emitted with the Rc-coercion block; the fix is to drop the braces
  (emit `f_arg` or `(expr)` not `{ expr }` in argument position), or only brace when a statement is needed.

## Fix direction (owner: v-rust-backend — the rust closure-argument emit)
Emit the coerced-closure argument without the wrapping block braces (or parenthesize instead of brace) so the
generated rust is `-D warnings` clean. Blocks pinning any generic-transformer-with-aggregate-closure corpus
case on the rust backend (the wasm side is green). NOTE: the gate builds emitted rust under -D warnings, so an
emit that produces ANY rustc warning fails the rust gate even when semantically correct.

Filed by corpus-bugfix; routed to v-rust-backend.
