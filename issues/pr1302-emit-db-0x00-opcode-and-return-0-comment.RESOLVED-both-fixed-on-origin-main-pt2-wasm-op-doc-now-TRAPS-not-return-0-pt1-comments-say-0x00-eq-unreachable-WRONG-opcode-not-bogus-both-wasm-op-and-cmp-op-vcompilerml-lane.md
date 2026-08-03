# PR #1302 review comments — implementation/compiler-ml/src/emit-db.cdz (v-compiler-ml)

Mirrored from https://github.com/camshaft/cadenza/pull/1302 (PR: "cand: v-compiler-ml — d7482375a").
Continuation of the #1213/#1287 emit-db trap-message thread.

## 1. "bogus 0x00 opcode" comment — 0x00 is valid wasm (`unreachable`) (Copilot, emit-db.cdz:152, also :165) — doc
> Inline comment says a miss would emit a "bogus 0x00 opcode", but 0x00 is a valid wasm opcode
> (`unreachable`). Clarifying this avoids confusion about what would be emitted if the gate is
> violated.

0x00 is `unreachable`, not a bogus/invalid byte — reword so the comment doesn't mislead about what a
gate violation would emit.

## 2. `wasm-op` doc still says non-arith ops "return 0" but impl now traps (Copilot, emit-db.cdz:149) — doc
> The doc comment for `wasm-op` still says non-arith ops "return 0", but the implementation now
> traps. Updating that comment keeps the docs consistent with the new behavior.

Same theme as #1213 (the trap that now names the op): the `wasm-op` doc still describes the old
return-0 behavior — update it to the trap behavior.
