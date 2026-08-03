# PR#989 review comments — ty-bridge over-export + stale header + unused import (v-compiler-ml)

Mirrored from GitHub PR#989 review comments (Copilot), ids `3695265421` (ty-bridge.cdz:151),
`3695265440` (ty-bridge.cdz:16), `3695265447` (infer-db.cdz:22). Compiler-ml port source →
v-compiler-ml. Blame `b658e8e76` "compiler-ml cleanup: extract Typed to a leaf module — break the
infer-db↔ty-bridge cycle, dedup typed-to-ty".

## Comments (verbatim)

- (id 3695265421, ty-bridge.cdz:151) "`typed-list-to-ty` is now exported, but there are no call sites
  outside this module (it's only used internally by `typed-to-ty`). Keeping it unexported reduces API
  surface and avoids other modules depending on an internal helper."
- (id 3695265440, ty-bridge.cdz:16) "The module header comment says infer-db has no deferred-literal
  state yet, but infer-db uses a deferred-int sentinel (`TIntW(_, 0)`). This comment is now misleading
  about what `Typed` values can exist and when it's safe to call `typed-to-ty`."
- (id 3695265447, infer-db.cdz:22) "`typed-list-to-ty` is imported from `ty-bridge` but never referenced
  in this module (only `typed-to-ty` is used). If the compiler enforces unused-import hygiene, this can
  break the build; even if not, it's dead weight."

### Liaison verification (confirmed on trunk a2875840b)

Grep confirms all three:
- ty-bridge.cdz:151 `export { typed-to-ty, typed-list-to-ty, ty-to-typed }` — but `typed-list-to-ty` is
  referenced only INSIDE ty-bridge (line 25 by `typed-to-ty`'s TFn arm, line 32/33 self-recursion). No
  external caller (infer-db.cdz imports it but doesn't use it — see below). Unexport it. (comment 1)
- ty-bridge.cdz:16 header talks about deferred-literal state — but the LIVE infer-db pipeline uses the
  `TIntW(_, 0)` deferred-int sentinel (the monomorphic-HM deferred-int the port's whole width-lattice
  rests on). If the header says "no deferred-literal state yet", it's stale/misleading about which
  `Typed` values reach `typed-to-ty`. Owner confirms the exact current header wording + corrects.
  (comment 2)
- infer-db.cdz:22 `import { typed-to-ty, typed-list-to-ty } from "ty-bridge"` — but only `typed-to-ty`
  is used in infer-db (the :689 mention is a comment). `typed-list-to-ty` is an UNUSED import → dead
  weight, and a build break if the port enforces unused-import hygiene. Drop it from the import.
  (comment 3) — pairs with comment 1 (unexport + drop-import together).

All API-hygiene / doc, behavior-neutral. Comments 1+3 are the same over-export→unused-import pair.

Owner: **v-compiler-ml** (compiler-ml port `ty-bridge.cdz`/`infer-db.cdz`; their `b658e8e76` extract).
Unexport `typed-list-to-ty` + drop its unused infer-db import; correct the stale deferred-literal header.
