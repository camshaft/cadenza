# PR#889 review comment — sum_disc_shaped decodes any immediate as int without tag-checking (v-runtime, defensive)

Mirrored from GitHub PR#889 review comment (Copilot), id `3671453971` (lib.rs:1430, also :2592, :6012).
File: `implementation/seed/crates/cdz-runtime/src/lib.rs` — v-runtime's crate (this PR = their
SOUNDNESS #43). Blame `cce30e57f` "rcdzc(runtime): shape-walk sum-disc decodes an immediate enum-disc —
compare + render (SOUNDNESS #43)".

Defensive-hardening / soundness-adjacent — reachability is v-runtime's call.

## Comment (verbatim)

- (id 3671453971, cdz-runtime/src/lib.rs:1430) "`sum_disc_shaped` calls `imm_as_int` for any immediate,
  but `imm_as_int` is only valid for int-immediates (tag `01`). If a malformed descriptor/value pairing
  ever places a non-int immediate here (unit/bool), the discriminant decode becomes garbage instead of
  cleanly declining. Consider making this helper return `Option<u32>` and validating the immediate tag /
  range before decoding. This issue also appears in the following locations of the same file: line 2592,
  line 6012."

## Liaison verification (all three confirmed on trunk 00714967a)

- `imm_as_int` (lib.rs:663-667) doc: "Decode an inline fixnum (**only valid when `is_immediate(h)` and
  tag is `01`**)". No internal tag check — it just arithmetic-shifts.
- `sum_disc_shaped` (lib.rs:1424-1430): `if is_immediate(h) { imm_as_int(h) as u32 } else { op_sum_disc(h) }`
  — gates only on `is_immediate`, NOT on the immediate's tag being `01` (int). A unit/bool immediate
  (different tag) would be shifted as if it were an int → garbage disc, not a clean decline.
- Same `sum_disc_shaped` used at :2592 (render `Shape::Sum`) and :6012 (cmp `Shape::Sum`) — both rely on
  the SOUNDNESS #43 path where a box-int'd all-nullary enum-disc arrives as an int immediate. On the
  WELL-FORMED path this is correct (the disc genuinely boxed via `op_box_int`). The concern is a
  MALFORMED descriptor/value pairing (a non-int immediate reaching a `Shape::Sum` walk) — then the decode
  is garbage rather than a clean decline.

Whether a malformed pairing is REACHABLE (can a non-int immediate ever be paired with a `Shape::Sum`
descriptor at these walks?) is v-runtime's call — on the compiler-emitted path the descriptor and value
are co-derived, so it may be a can't-happen the tag-check would only document. But given this is the
SOUNDNESS #43 PR, a defensive tag-guard (helper returns `Option<u32>`, `None` → clean decline instead of
garbage) is cheap and in-theme. Owner decides can't-happen-invariant vs defensive-guard.

Owner: **v-runtime** (`cdz-runtime` crate, SOUNDNESS #43, `cce30e57f`). Three sites, same helper.
NOTE: cdz-runtime is inside the frozen `REQUIRED_RUNTIME_HASH` — a behavior/code change here (not just a
comment) means `cargo xtask build` + `codegen --check` + a hash bump (v-runtime knows this discipline;
this PR already bumped the hash).
