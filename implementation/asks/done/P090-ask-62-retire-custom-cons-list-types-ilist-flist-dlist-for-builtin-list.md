## 62. ✅ DONE 2026-07-08 (compiler.cdz — OPERATOR DIRECTION) Retire the custom cons-list sum types (IList / FList / DList / Code) for the built-in `list` — ALL FOUR RETIRED

**✅ COMPLETE — ZERO custom cons-list sequence types remain in compiler.cdz.** value-harness 35 agree /
5 soft / 0 hard / 0 error (full parity, +1 agree vs the pre-migration baseline). Self-compiles VALID
(90686 bytes). MOVE TO done/.

- **Step 1 — `IList` → `list<Int64>` (the param/function env):** `ienv-pos` is now an index-scan
  (`ienv-pos-go`, low→high with overwrite-on-match so the HIGHEST/last match wins — shadowing preserved);
  `ienv-len`=`List.len`; `ienv-snoc`=`List.push`; builders `sig-param-env`/`mod-fenv` push in order
  (position = slot / function index). Verified: sibling lets ⇒ 3, param+call ⇒ 6, 2-param ⇒ 7, shadowing ⇒ 5.
- **Step 2 — `FList` → `list<Func>` and `DList` → `list<Def>`:** all walks (`type-items`/`func-items`/
  `code-items`/`fold-funcs`/`typecheck-funcs`/`ktab-seed`/`ktab-recompute`/`any-func-rejects?`/`check-funcs`/
  `entry-guard`/`flist-head`, `resolve-module`, `read-defs`) rewritten as index-scans / push-accumulators over
  the built-in list (position = func/def index). `flen`=`List.len`.
- **Step 3 — `Code` → `list<Instr>` (the backend instruction sequence):** DONE. `Code` was already fully
  abstracted behind 5 primitives — only they changed, all ~30 `lower` arms untouched. `serialize` = index-walk;
  `code-cat` = concat-via-push (`code-cat-go` appends ys onto xs element-by-element, since the surface has no
  `List.concat`); `one i` = `(List.push (list) i)`; `seq` = identity (Code IS a `list<Instr>` now). Verified on
  codegen-heavy RUNTIME-operand cases (force `lower`/`code-cat`/`serialize`, not const-fold): nested arith ⇒ 11,
  shift with scratch-locals ⇒ 16, if-over-runtime ⇒ 7, call+call ⇒ 20, bitwise ⇒ 5 (byte-identical). The
  concat-via-push preserves instruction order exactly.
- ⚠ COST NOTE (watch, not yet a problem): `code-cat` is now O(len ys) per call (element-by-element push),
  and `lower` nests `code-cat` along the expression spine, so a very large single function body is O(n²) in
  its instruction count. At the current corpus scale it's fine (self-compile stays fast); if a future large
  body regresses, the fix is a `List.concat`/`List.append` surface op (a genuine seed ask) or an
  accumulator-threaded `lower` — NOT a return to a cons type.

⚠ LOOP GOTCHA that cost time here: point the harness/`emit` at the **stable** toolchain
(`stable/cadenza-seed` + `stable/cdz_runtime.wasm`), NOT a live `implementation/seed/` build — a mid-change
live seed transiently emits INVALID components ("failed to parse WebAssembly module") for `List.push`-composed
programs, which looks like a compiler.cdz regression but is a stale/broken seed. The harness now defaults to
stable (`CDZ_SEED`/`CADENZA_RUNTIME` env-overridable).

**Original direction + plan follows.**

---

## 62. 🟡 (compiler.cdz — OPERATOR DIRECTION) Retire the custom cons-list sum types (IList / FList / DList / Code) for the built-in `list`

**Operator's direction (2026-07-07):** *"We still need to refactor all of those things [IList etc.] to use built-in
lists instead of the custom sum types."* Realizes the standing [[migrate-compiler-onto-builtin-list]] direction.

**Current state.** compiler.cdz defines 4 bespoke cons-list types, introduced early when the built-in `list`
couldn't yet carry compound elements / recurse without OOM:
- `IList (INil | ICons (Tuple Int64 IList))` — the param/function-name env (prelude indices). 12 refs, ops
  `ienv-pos`/`ienv-snoc`/`ienv-len`.
- `FList (FNil | FCons (Tuple Func FList))` — the module's function list. 28 refs, ops `flen`/`flist-head`/
  `fold-funcs`/`typecheck-funcs`/`core-module-multi`/`build-ktab` etc.
- `DList (DNil | DCons (Tuple Def DList))` — the reader's def list. 5 refs (`read-defs` builds, `resolve-module`
  consumes — DList→FList).
- `Code` — the instruction-list type (separate; the backend's emitted-instruction sequence).

Meanwhile the compiler ALREADY uses the built-in `list` heavily (73 refs — `List.push`/`List.at`/`(list …)` in
`resolve-args`, the reader arg lists, the `lce` const-prop env, `render-*`). So the built-in list is proven for
the compiler's own use; the custom types are legacy.

**✅ FEASIBILITY CONFIRMED (2026-07-07) — the seed capability that blocked this is RESOLVED.** The reason the
custom types existed was that the built-in list couldn't carry a COMPOUND element (a `Func`/`Def` = a tuple) and
recursively consume it without inference blowup/OOM. Verified that is fixed: `(def (sum-firsts xs i acc) (match
(List.at xs i) ((Some (tuple a b)) (sum-firsts xs (+ i 1) (+ acc a))) ((None _) acc)))` over a `List.push`-built
`list<tuple>` → **30** (a compound-element built-in list, built + recursively summed, with the exact
`((Some (tuple …)) …)` nested-binder pattern). So `list<Func>`/`list<Def>`/`list<Int64>` all work. NO seed
dependency — this is a compiler.cdz-only refactor.

**The mapping (mechanical):**
- `INil`/`FNil`/`DNil` → `(list)`; `ICons`/`FCons`/`DCons` (append via `ienv-snoc`) → `List.push`.
- head/tail recursion (`match xs ((Cons (tuple h t)) …)`) → index walk `(match (List.at xs i) ((Some h) …) ((None _) …))`
  — the idiom already used in `resolve-args`/`lce-at`/`render-ok-elems`.
- `ienv-pos` (position = local slot, LAST match wins for shadowing) → an index-scan returning the highest match;
  `ienv-len` → `List.len`; `flist-head` → `(List.at xs 0)`.
- ⚠ ORDER: `ienv-snoc` appends at the END (slot = old length); `List.push` also appends — semantics preserved.
  Verify the shadowing rule (`ienv-pos` returns the LAST/highest match) is kept in the index-scan version.

**Why it matters (self-hosting).** Fewer bespoke types = a smaller compiler closer to the real language surface,
and it exercises the built-in list (the trie) on the compiler's own hot paths — the [[migrate-compiler-onto-builtin-list]]
critical-path goal. Also removes 4 type decls + ~10 helper fns.

**Plan (INCREMENTAL — gate-green between each; NOT a big-bang edit).** The types are coupled (`resolve-module`:
DList→FList), so migrate in dependency order with the byte gate + value-harness green at each step:
1. `IList` → `list<Int64>` (the env — most isolated; `ienv-*` become list-index helpers). Verify shadowing.
2. `DList` → `list<Def>` and `FList` → `list<Func>` together (they meet at `resolve-module`).
3. `Code` → `list<Instr>` (the backend instruction sequence) — last, touches serialize.
Each step: rewrite the type's ops + refs, self-compile VALID, component-check 0-disagree + value-harness 0-hard.

**⛔ BLOCKED on ask-63 (a confirmed runtime RC use-after-free) — 2026-07-07.** Attempted the IList→built-in-list
migration (env = `list<Int64>`, `ienv-*` as list helpers). Self-compiled VALID, but the value-harness caught a
REGRESSION: sibling lets `(+ (let ((x 2)) x) (let ((y 1)) y))` emit invalid wasm (`local.get 192` — a freed
prelude index leaking into a local slot). Root cause MINIMIZED to a pure-runtime bug (ask-63): a built-in `list`
value consumed by TWO operations across a function boundary under checked arith is FREED TOO EARLY (`op_drop` /
`talc::deallocate` double-free in `vec-push`). The migration logic is correct (`ienv-*` verified in isolation); the
built-in list is unusable for the env until the RC bug is fixed. REVERTED the migration to keep compiler.cdz
gate-green (139/0). Per the operator's discipline: block on the miscompile, don't work around it. Re-attempt after
ask-63 lands.

**Status.** 🟡 OPERATOR-DIRECTED, compiler.cdz-only. ⛔ BLOCKED on ask-63 (runtime RC use-after-free — the env,
being consumed by multiple sibling reads, hits it). Feasibility of the migration itself is proven; the blocker is
a runtime bug the migration EXPOSES. A meaningful multi-part
refactor across ~45 refs + ~10 helpers on load-bearing paths (the env, the function list) — best as a dedicated
focused pass, incremental, not squeezed mid-feature. Related: [[migrate-compiler-onto-builtin-list]], ask-60 (heap
types — the built-in list's compound-element support this rides on is the same machinery). Not a workaround — it
retires legacy scaffolding now that the built-in list subsumes it.
