# Perceus OccTable — the effects RESUME-SEAM occurrences (v-effects slice)

Status: DESIGN (v-effects, 2026-08-30; rev.2 keyed to POST-SPLICE Core per v-memory-safety's
accept). The resume-seam slice of the uniform per-occurrence reclaim architecture — read
[`DESIGN-perceus-per-occurrence-dup-placement-uniform.md`](DESIGN-perceus-per-occurrence-dup-placement-uniform.md)
first; that doc lists v-effects as CO on the boundary / **resume seams**. This enumerates the
resume-seam heap values *as the POST-SPLICE Core `select.rs` actually sees them*, the `ValueKey`
each maps to, and the borrow/consume class — so `classify_occurrences` can pick them up uniformly.

Owner (soundness/acceptance): v-memory-safety. Executor (select.rs emit + placement): v-core-opt.
This slice (resume-seam classification + witnesses): v-effects. Slots into #5857 **Increment C**
(unify), behind v-core-opt's Increment A (hczm) + B (dup_at repr).

Leverage (concierge, corrected): this unblocks v-effects' OWN cross-function / non-tail resume
increment (the "later increment" the specializer-floor declines cite) — NOT the compiler-ml shred
(that lever moved to v-cadenza-backend solo).

---

## 0. The framing: `resume` is spliced away BEFORE Core

`resume` is SPLICED AWAY in `reduce_handle`/`splice_context` (effects.rs:11762) before Core — there
is NO resume node in `select.rs`. `classify_occurrences` sees the POST-SPLICE Core: the continuation
context inlined at the perform hole. Today it can't tell a resume-thread from a pure consume (both
route via `Core::Call`/`CallClosure`), so it conservatively HOLDS (leak — correct). This slice's job
is to make the resume-seam occurrences CLASSIFIABLE in that post-splice Core. So the enumeration
below is keyed to the Core node each value BECOMES, not to a "resume site" that no longer exists.

## 1. The two splice shapes (what the fold emits)

The fold reifies the continuation two ways (effects.rs ~2686–2758):

- **TAIL-RESUMPTIVE** (`rewrite_resume_to_context`, effects.rs:11455): `(resume v s)` → `C[v]` — the
  delimited continuation `C` (handle body, perform-hole→v) is inlined DIRECTLY at the site. No
  closure. The threaded state `s` and value `v` become ordinary Core in the inlined body.
- **ESCAPING** (`k = (fn (#kv) C)`, effects.rs:2701): when the continuation escapes, it is reified as
  a lambda `(fn (#kv) splice_context(body, perform, #kv))`, lowered to a `Core::Closure` and applied
  at each `(k v)` via `Core::CallClosure`. `C`'s free names (incl. the threaded state) become the
  closure ENV.

## 2. POST-SPLICE occurrence → Core node → ValueKey

| resume-seam value | POST-SPLICE Core node (`select.rs` sees) | ValueKey | note |
|---|---|---|---|
| threaded state `s`, tail-resumptive | `Core::Param{s}` / `Core::LocalRef{s}` — the dispatch-loop state param, read in the inlined `C` | `Param(s)` (or `Let` when the loop names it) | it IS a synthesized fn param of the specialized dispatch loop (effects.rs:512 threading) |
| threaded state `s`, escaping continuation | a slot of the reified `Core::Closure` env; read inside the closure body | `Captured(i)` | closure-env-owned ⇒ `init = 0` (borrowed) — the borrowed-N rule §2.3 |
| the `#st` value/state tuple | a materialized Core tuple from `build_value_state_tuple` (effects.rs:5588), unpacked by tuple-`Proj` per dispatch | `PayloadNode(proj)` | same shape as a sum-payload extraction the reference doc already models — "re-projected per dispatch" |
| resume value `v` | an operand occurrence spliced into `C` (tail), or a `CallClosure` arg (escaping) | (the enclosing value's occurrence) | NOT a new key — an occurrence of an existing value |

NO new `ValueKey` variant. The resume seam is three existing entry points: **param-keyed** (tail
threaded state), **capture-keyed** (escaping-continuation closure env), **payload-node-keyed** (the
`#st` tuple unpack) — each seeding the same `classify_occurrences` walk.

## 3. Borrow vs Consume at each post-splice node (the "reuse, don't re-derive" gate)

The judgments `binding_escapes_dup_aware` must produce per post-splice node; v-mem's gate (2) is to
VERIFY the existing arm logic already yields these, so NO new predicate is added:

- **threaded state, tail-resumptive** — a `Core::Param`/`LocalRef` READ in `C` to compute a value is a
  **Borrow**; the occurrence THREADED into the next dispatch's `#st` tuple (a ctor element / call arg)
  is a **Consume**. `binding_escapes_dup_aware`'s existing `Param`/`LocalRef` arms should classify
  these correctly (read = borrow, ctor-element/arg = consume). *Verify.*
- **escaping-continuation captured state** — a `Captured` slot read inside the closure body: a
  `Proj`-to-scalar / match-dispatch is **Borrow**; a ctor-element / onward-call arg is **Consume**.
  `init = 0` (closure env owns + drops the original), so EACH consuming/escaping occurrence gets `+1
  dup` (borrowed rule). *Verify the `Captured` arm classifies the onward-flow as Consume.*
- **`#st` tuple `Proj`** — the projection extracting `value`/`state` is the standard payload-node
  occurrence; borrow/consume by how the projected value is then used (identical to sum-payload).

**FLAG (the one place a targeted arm may be needed):** a state read that is threaded onward UNCHANGED
(pure pass-through, `(resume … s)` with `s` also read) has TWO occurrences — one Borrow (the read),
one Consume (the thread). If the existing walk collapses these (counts the pass-through as a single
borrow), the Consume is missed → under-dup → UAF. This is the seam to check first against
`binding_escapes_dup_aware`; if it mis-counts, a targeted `Param`/`Captured` consume-on-thread arm is
the minimal fix (NOT a new global predicate).

## 4. glb1 = the §2.3 borrowed-N-escapes acceptance case

glb1 (the collapse-continuation join-point duplication) in post-splice Core is a `Core::CallClosure`
(or an inlined `C`) reachable via a JOIN that duplicates the reified continuation across N paths. Its
captured `#st` state is a `Captured` value, `init = 0` (borrowed). The join → N reachable escaping
occurrences of that capture ⇒ **§2.3 borrowed rule: N dups, PATH-AWARE** (mutually-exclusive collapse
arms take MAX-over-arms, not sum — the `ifcap` anti-over-dup: a continuation reached via both arms
needs ONE dup on the taken path). So the OccTable SUBSUMES glb1 — no separate glb1-emit lowering; the
parked glb1 pairing folds away. This is the acceptance proof that the resume seam is "just values
with occurrences."

## 5. Soundness gates (v-memory-safety owns these; captured here so the slice honors them)

- **(a) Conservative — uncertain ⇒ HOLD.** A post-splice seam-node whose classification is uncertain
  stays HOLD (leak), never a speculative drop. Leak beats UAF.
- **(b) The xar5-class forward-escape is a CORRECT must-HOLD, EXCLUDED from get-to-0.** A captured
  value that escapes FORWARD inside the continuation (passed onward within `C`, Core-invisible as a
  resume-forward) must NOT be dropped — dropping = double-free. It stays a leak BY DESIGN (v-mem's
  gate). So the resume-seam get-to-0 target EXCLUDES the resume-continuation-forward-escape minority
  (xar5/xas1 class); those remain intentional holds, not failures.
- **(c) EXIT-SEAM placement, fires once.** Resume-seam drops go at the `reduce_handle` top-level exit
  seam (fires ONCE), NOT the shared per-call/per-dispatch arm — a per-call drop under a loop is a
  double-free. Same terminal-only discipline as the Class-B / terminal-shell reclaim.

## 6. Acceptance witnesses (the resume-seam corpus for Increment C)

- `rsh1` (#5829) + the same-effect straddling twins (heap-state #5883, compound-resume `crn1` #5885)
  — nested-handler state + compound-resume threading, probed sound (a45/a46): PASS-value +
  `live-objects 0` under the OccTable.
- `#5090` flagship-reducer UAF fences + SITE-A over-retention (`#5142`): stay 0.
- `glb1`: flips from parked-leak to PASS `live-objects 0` under the borrowed-N-escapes rule (the
  acceptance proof).
- Path-aware control: a continuation escaping via BOTH collapse arms → ONE dup (the resume-seam
  `ifcap1` analogue) — proves no over-dup leak.
- xar5/xas1 (forward-escape): stay HELD (leak), assert NO speculative drop (gate b) — a drop here is
  the double-free regression to guard against.

## 7. Next steps

1. v-memory-safety soundness-reviews §2 (ValueKey mapping) + §3 (borrow/consume verdicts, esp. the
   §3 FLAG pass-through seam) before any emit.
2. v-effects verifies §3 against `binding_escapes_dup_aware` (which arms classify each seam-node;
   flag the mis-counted one) and confirms glb1's §4 shape.
3. v-effects implements the three §2 entry points into `classify_occurrences` + the glb1 acceptance
   case, feeding v-core-opt a classified slice — NOT touching `select.rs` emit or placement. Lands as
   part of #5857 Increment C, behind v-core-opt's Increment A + B.
