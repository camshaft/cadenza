## 16. 🟢 The real `resolve` on a runtime-built `Node` declined "cannot box" — RESOLVED (was MIS-FRAMED as a seed scale limit) 2026-07-07

**⚠️ Correction (2026-07-07):** this item was **mis-framed**. It was NOT a seed scale limit in the
runtime heap-boxer. The "cannot box" decline was **self-inflicted**: `resolve`'s `PUnknown` arm used an
out-of-range `Bytes.of (list 256)` as a placeholder trap (item 11's stub), a `Never` value that poisoned
the whole runtime `resolve`. Replacing it with an honest `Core.KError → unreachable` fixed it — see item
11. My "scale limit, no minimal witness" bisection was wrong because it rebuilt a clean structural
analogue and dropped the culprit (the Bytes hack); the correction and its meta-lesson (reduce the failing
program by deleting its arms, not by rebuilding a clean one) are in
`spec/learnings/2026-07-07-the-workaround-was-the-bug-correcting-the-scale-limit-diagnosis.md`. A real but
differently-shaped seed invariant DID exist underneath (a `Never` value on the runtime-heap path emitted
invalid code), now hardened and pinned ([[never-typed-value-on-the-runtime-heap-path]]). Net: `resolve` on
a runtime `Node` compiles, the reader→pipeline connects, `bytes → component` works end-to-end. The
original (mis-framed) analysis is kept below as the historical record.

---


**Finding.** `compiler.cdz`'s real `resolve : Node → Core` declines **"runtime compound element of a
kind the runtime cannot box yet"** when applied to a `Node` built at runtime (what the reader produces)
and forced to a runtime value. This is the last link: `read-node : Bytes → Node` is built and verified
(`read (quote (+ 1 2))` builds the right Node), but `read → resolve → fold → lower → serialize → frame`
cannot connect because `resolve` on a runtime `Node` declines.

**It is a SCALE limit, not a shape gap.** It does not reduce to a minimal case — every structural
feature works at runtime in isolation: a 3-variant resolver runs, a 6-variant heterogeneous
`KConst`/`KBoolC`/`KAdd`/`KLt`/`KNot`/`KIf` resolver runs (verified → 4), runtime `(Tuple String Node
Node)` build+match works, `head-prim` on a runtime String works. Only the **full 18-variant `Core`
returned by the full `resolve`** declines, and even `resolve` on a runtime `(NInt 42)` (a scalar arm)
declines — so it is a full-FUNCTION property (some arm's Core construction poisons every call), a
specific element-kind combination in the 18-variant union the runtime heap-boxer rejects on this path.

**Why it touches the seed (not the spec).** The language clearly permits a recursive `Node → Core`
resolver over runtime input — every sub-shape compiles. It is a runtime heap-boxer limitation at the
union/scale of the full variant set. Seed fix: trace which `gen_runtime_*` / heap-box path
`resolve`-of-a-runtime-`NPrim` hits and reports "cannot box", and admit that element-kind combination.

**Status.** ⚪ Seed work (SEED-GAPS Tier 2f). **No corpus case** — deliberately: a scale limit has no
minimal witness (every tractable resolver passes; the failing one is the full 18-variant `resolve`, too
large and threshold-specific to pin durably). Its regression guard, when fixed, is the whole
`compiler.cdz` connecting `read → resolve → … → frame` and compiling — the two-compilers gate on the
whole compiler. **This is the single remaining hard blocker on `bytes → bytes` self-hosting** (items 12
and 13 remain, but the reader routes around 12 for structure and 13 is ergonomic). Learning:
`spec/learnings/2026-07-07-the-final-self-host-blocker-is-a-scale-limit-not-a-shape-gap.md`.

**Update (2026-07-07) — 🟢 FIXED.** The runtime heap-boxer now admits the full 18-variant `Core` union
on the `resolve` path: the full-shape `resolve` on a runtime-built `Node`, scalar-consumed, runs (→ 1).
Now that it is fixed, a **representative** corpus case IS pinnable (the scale-limit rule flips: no minimal
witness while broken, but a natural-size representative guards it once fixed): `05-compound-types.sexp`
*"a recursive resolver transforms one runtime sum tree into another, then consumes it"* (`resolve : Node →
Core` then `eval : Core → Int64`, → 42, **PASS**). With this, **every self-host seed blocker
(Tier 00/0/2b/2c/2d/2e/3a/2f) is cleared** — the remaining work is WIRING the `read-node → resolve` join
in `compiler.cdz` (kept uncommitted until 2f landed) plus non-blocking items 12/13. Learning:
`spec/learnings/2026-07-07-the-final-self-host-blocker-is-fixed-the-reader-can-join-the-pipeline.md`.

---
