# PR #1792 review comment — rcdzc/src/backend/wasm/mod.rs (v-wasm-opt) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1792 (MERGED — hoisted CDZ0406 closure-escape scan).

## CDZ0406 scan walks ALL lifted lambdas incl unreached/inert-stub slots → spurious reject of unreachable closures (Copilot, mod.rs:194) — correctness [VERIFIED]
> The hoisted CDZ0406 scan walks all lifted lambdas, including slots marked unreached + emitted as inert
> stubs (see `append_lifted_bodies`). That can spuriously reject programs where an effectful closure was
> lambda-lifted but later proven unreachable. It also does unnecessary work scanning every body even after
> the first escaping host op. Limit to `lifted_reached` slots and break on the first host import.

VERIFIED against trunk: the scan is `for l in &layout.lifted { host::collect_host_imports(db, l.body,
&mut escaping) }` (mod.rs:192-194) — iterates ALL `layout.lifted` with NO reached-filter. But
`layout.lifted` DOES include unreached slots: `append_lifted_bodies` (mod.rs:118-122) iterates the same
`layout.lifted` and gates on `layout.lifted_reached.get(code)` to emit a real body vs an INERT STUB
(:111 "never called"). So an UNREACHED lifted lambda whose (dead, stub-emitted) body contains a
`Core::HostCall` is still collected → a spurious CDZ0406 reject of a program where that closure is
provably unreachable. MED (false-reject of valid code). Fix per Copilot: filter the scan to
`lifted_reached` slots (mirror the :122 gate) + break on `escaping.first()`. RECOMMEND v-wasm-opt confirm
+ add a witness (an effectful closure lifted-but-unreachable that should COMPILE). Fix-forward.
