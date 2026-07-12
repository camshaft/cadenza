# Asks index

One line per ask across all three lifecycle directories. See `README.md` for the process.
Regenerate after moving files. Sorted: open (by priority) → pending-validation → done (by ID).

## 🔧 Open (priority-ordered)

- `P021` **ask-81** — 🧭 DESIGN + HANDOFF (rcdzc): CLOSURES as compile-time lambda β-reduction — the SAME `eval` fold tier that will do type-functions/monomorphization (#150). A `(fn …)` is one more TRANSIENT compile-time value (FuncRef/Ctor/Intrinsic family): new IR leaf `Lambda` (Hir/Typed/Mir), one fold rule `Apply(Lambda,args)`→β-reduce, one `select` decline for a survivor. **Increment A (compile-time lambdas)** closes the ENTIRE core `09-functions.sexp` witness (immediate/let-bound/named-HOF/returned/in-place-capture) + **CURRIED application that const-folds back into a regular `Call`** (`((add 3) 4)`→`Call{add,[3,4]}` via fold SPINE-COLLAPSE; the eager under-arity decline at `infer.rs:187` MOVES to the reduction-aware fold) — needs α-renaming on inline/β-reduce (mandatory; hardens the const-inline path too) + `is_transient`/`try_inline` guard widening. **Increment B (runtime closures = `call_indirect`+heap env+`Ty::Fn`→I32)** DEFERRED, declined honestly. Type functions fall out: a type fn is a lambda over type values, always reduces at compile time (pure A subset) — build the β-reduction tier once  
  `open/P021-ask-81-closures-design-compile-time-lambda-beta-reduction-shared-with-type-functions.md`
- `P005` **ask-68** — 🟢 READY TO IMPLEMENT — Perceus precise drop insertion (M2 Phase D / "task #9"), the FULL spelled-out plan. Reframe: leak-freedom (`live-objects → 0`, the actual ask) needs NO liveness analysis — it's mechanical, lands at end of Phase 2 via an Owned/Borrowed context flag threaded through `emit` + scope-end drops. Phase 1 = drop materialized heap locals at fn-end (balances the dup-on-read) + fix `gen_do` `op::DROP`→`himport::DROP` leak. Phase 2 = stop dup'ing in borrow positions (accessors: arr-get/sum-payload/vec-get/map-lookup all return BORROWED handles — the UAF class), dup escaping accessor results. Phases 3 (last-use, the hard part, needs ask-64 UAF trap) + 4 (FBIP reset/reuse — runtime ALREADY built these, WIT 26–28, just not in HEAP_ALLOWLIST) are OPTIONAL optimization. Verified against tree; runtime's own tests prescribe `dup(kept);drop(parent)`  
  `open/P005-ask-68-perceus-precise-drop-insertion-the-full-plan.md`
- `P015` **ask-30** — 🟢 CLOSED (rejection seam exhausted) The self-hosted compiler's CODED effect-diagnostics `compile` keeps the **byte-gate GREEN (component-check 123 agree / 0 disagree — PASS)** and catches every PROVABLE rejection reachable without compound types: arith/cmp/`if`-branch kind mismatch → CDZ0201, non-exhaustive BOOL match → CDZ0210, out-of-range int literal → CDZ0201, dup record/map field → CDZ0201, malformed `let` → CDZ0201, int-vs-float → CDZ0301/CDZ0201, scalar-receiver `.`/`tuple.N` → CDZ0201. The gap-independent reader-REJECTION seam is now EXHAUSTED (analysis in ask-57); every remaining native-rejects/mine-declines is SHAPE-DEPENDENT compound work (needs the seed heap — ask-13/ask-57), all honest `decline`  
  `open/P015-ask-30-the-self-hosted-compiler-has-no-type-checker-compiles-ill-typed-programs-native-rejects.md`
- `P016` **ask-58** — 🟢 DESIGN (operator direction): make ALL built-in modules (`Bytes`/`Int64`/`String`/`List`/`Set`/`Map`/`Char`/… — every one, not just `Bytes`) REAL prelude RECORDS of builtin-function refs, so `Bytes.len` = `(. Bytes len)` const-folded — delete per-module name-specialization from the reader/resolver; realizes the `member-access-and-modules-as-records` decision. Key new primitive = a first-class BUILTIN-FUNCTION REF value (folds from projection, lowers on apply, declines elsewhere). SEED+SPEC first; compiler.cdz's projection-fold (ask-57) is ready, just needs apply-a-builtin-ref lowering  
  `open/P016-ask-58-builtin-modules-as-records-of-builtin-function-refs-const-folded.md`
- `P028` **ask-48** — 🟡 The new `diagnostics.md` capability sets the diagnostics bar well above the seed — error recovery (maximal independent set), primary/derived, and a machine-branchable rejection/decline/trap kind  
  `open/P028-ask-48-diagnostics-capability-sets-bar-above-seed-error-recovery-maximal-set-primary-derived-kind.md`
- `P030` **ask-20** — ⚪ The self-inclusion frontier: what the compiler's emit path must grow to compile its own source  
  `open/P030-ask-20-the-self-inclusion-frontier-what-the-compiler-s-emit-path-mu.md`
- `P050` **ask-02** — 🔴 M-ordering tension: effects are the #1 self-host blocker but scheduled M6  
  `open/P050-ask-02-m-ordering-tension-effects-are-the-1-self-host-blocker-but-s.md`
- `P070` **ask-09** — 🔴 Should a provable-certain trap be a compile-time rejection? (fold stays meaning-preserving either way)  
  `open/P070-ask-09-should-a-provable-certain-trap-be-a-compile-time-rejection-f.md`
- `P100` **ask-39** — 🟡 Runtime AST construction / `Ast.encode` / `Ast.decode` decline — `Ast` is a compile-time-only value, not a registered RUNTIME sum type  
  `open/P100-ask-39-runtime-ast-construction-and-encode-decode-not-supported-ast-not-a-runtime-sum-type.md`
- `P110` **ask-06** — ⚪ Byte-identity target must account for optimization depth (cdz-rustc needs DCE)  
  `open/P110-ask-06-byte-identity-target-must-account-for-optimization-depth-cdz.md`
- `P120` **ask-07** — ⚪ Runtime `String` is the keystone front-end blocker (spec is fine; seed + realized-set work)  
  `open/P120-ask-07-runtime-string-is-the-keystone-front-end-blocker-spec-is-fin.md`
- `P110` **ask-59** — 🟡 (compiler.cdz) Bool-typed PARAMETERS decline — the i64-parameter model can't compile a function whose param is used as a Bool (`if`-cond / bool-`match` / `not`) nor a call passing a Bool arg; native infers per-param valtype. Fix = per-parameter kind inference + Bool-aware calling convention (a Bool param = i32, a Bool arg passes direct). LARGEST remaining compiler.cdz-ownable scalar cluster; NOT seed-gated. SUBSUMES ask-35 (return-kind is the pass-through case). Deferred — a real inference+ABI subsystem, not a loop patch  
  `open/P110-ask-59-bool-parameter-kind-inference-and-calling-convention.md`
- `P120` **ask-35** — ⚪ Polymorphic return-kind specialization (a function whose return kind is its argument's) — the `agree` follow-on to ask-34's decline; SUBSUMED by ask-59 (per-parameter kind inference)  
  `open/P120-ask-35-polymorphic-return-kind-specialization-for-byte-identity-follow-on-to-ask-34.md`
- `P130` **ask-08** — ⚪ No tail-call optimization / bounded wasm stack (self-host ceiling, not a blocker)  
  `open/P130-ask-08-no-tail-call-optimization-bounded-wasm-stack-self-host-ceili.md`
- `P130` **ask-43** — ⚪ Right-shift over-declares one scratch local (3 vs native's 2) — a byte-fidelity gap keeping `>>` `soft` not `agree`  
  `open/P130-ask-43-right-shift-over-declares-a-scratch-local-soft-not-agree.md`
- `P140` **ask-10** — 🟡 A spike's "verified byte-correct" claim must become a corpus case, not stay a probe  
  `open/P140-ask-10-a-spike-s-verified-byte-correct-claim-must-become-a-corpus-c.md`

## ⏳ Pending validation

- **ask-55** — ✅ float crash FIXED (compiler.cdz 19:03) — loop-verified: bare `4.5` → decline (was trap), 0 `run error` traps in the byte gate (was 22), int/float mix now rejects. Awaiting stable refresh + four-gate confirm. Follow-on ask-56 (wrong code on the mix).  
  `pending-validation/ask-55-shape-check-regressed-float-from-decline-to-crash.md`
- **ask-88** — ✅ codemod `rewrite` MULTI-SPLICE landed: a pattern list allows several `,@` splices if none are adjacent, so `(case ,@before (needs ,_) ,@after)` deletes a clause at any position (backtracking matcher). Re-probe = the `(needs)`-strip with the two-splice pattern (no fixed-position fragility).  
  `pending-validation/ask-88-codemod-rewrite-cannot-delete-a-clause-at-an-arbitrary-position-one-splice-limit.md`
- **ask-89** — ✅ codemod FORMATTING-PRESERVING edit landed (the real blocker): the s-expr reader now records spans (`read_spanned`), and `--write`/`--diff` splice only changed subtrees at their spans — layout/comments kept verbatim — instead of reprinting. `--reprint` forces the old reflow. Re-probe = the `(needs)`-strip across the corpus yields a minimal diff + clean roundtrip.  
  `pending-validation/ask-89-codemod-write-reformats-the-whole-file-does-not-preserve-source-line-layout.md`

## ✅ Done

Resolved asks (ask-01 … ask-56, the P0xx-prefixed batch) have been pruned from the
tree — they are closed history and remain in git. Recover any with
`git log --all --diff-filter=D -- 'implementation/asks/done/*'` then `git show`.
