## 44. 🟢 Stray `DBG` `eprintln!` left in the seed's ctor-arm-match codegen (debug noise on the self-hosting path)

**Finding.** `implementation/seed/crates/cdz-compiler/src/codegen.rs:4296` has a leftover debug trace:
```rust
if scrut_kind != Kind::Heap && arms.iter().any(|a| matches!(a, Node::List(x) if x.first().map_or(false, is_constructor_pattern))) {
    eprintln!("DBG ctor-arm match, scrut_kind={:?}, scrutinee={:?}", scrut_kind, scrutinee);
}
```
It fires (on stderr) whenever a `match` has a constructor-pattern arm but a **non-Heap scrutinee** — an
inference edge in the ctor-arm-match path. In practice it prints once while the seed compiles `compiler.cdz`
itself (`DBG ctor-arm match, scrut_kind=Int64, scrutinee=Name("node")`), surfacing in `compile-run` output.

**Severity — LOW (hygiene, not correctness).** It is on stderr, so it does not corrupt emitted bytes or the
gate's stdout parsing: the behavior gate is GREEN (569), a plain `emit` prints 0 DBG lines, and a full
behavior-gate run prints 0 — it only appears on the self-hosting `compile-run` path (the seed compiling
compiler.cdz). WRONG sweep = 0. So it is debug noise, not a bug — but it is a debug `eprintln!` that should not
ship in the seed's normal codegen.

**Two things it's worth as a signal:**
1. **Remove the `eprintln!`** (or gate it behind a debug flag). Trivial cleanup.
2. It **marks a spot the type-inference work is actively probing**: the guard condition — a ctor-pattern arm with
   a `scrut_kind != Heap` (here `Int64`, scrutinee `node`) — is the diagnostic tripwire for the ctor-arm-match
   kind-inference edge the agent is working (a `match` on a constructor whose scrutinee kind hasn't resolved to
   Heap). Whatever that trace was diagnosing is the live inference case; when it's resolved, the trace goes.

**Acceptance signal.** `cadenza-seed compile-run <compiler.cdz> <any>` prints no `DBG` line on stderr; the
ctor-arm-match kind inference for a non-Heap scrutinee (`node`) either resolves or declines cleanly without the
trace. No corpus case (it's a stderr debug print, not a value-behavior). Found by the loop's Run-81 probe of the
self-hosting `compile-run` output.

**🟢 LOOP-CONFIRMED FIXED 2026-07-07 (Run 82).** The 14:02 seed rebuild removed the `eprintln!` — `grep -c "DBG
ctor-arm"` in codegen.rs = 0, and `compile-run <compiler.cdz>` prints 0 DBG lines on stderr. Clean removal, no
gate impact (was always stderr-only). Moved open → done.
