## 10. 🟡 A spike's "verified byte-correct" claim must become a corpus case, not stay a probe

**Finding.** The spike's handoff docs repeatedly certify emit paths as "verified byte-correct" —
LEB128 (`uleb 624485 → E5 8E 26`), signed LEB128 boundaries, the core-module framing "byte-identical
to cdz-rustc for main=42", the component envelope. These were verified by ephemeral `emit` probes in
the gitignored `implementation/` tree, not by gate obligations. The corpus pinned each *primitive*
(`&`, `|`, `>>`, `Int.to-byte`, `Bytes.concat`) but not the *composition* — so the compiler's
byte-emitting spine was protected only by a scratch buffer that vanishes when the spike is cleaned.

**Why it touches the spec/process.** The two-compilers differential gate only protects what the
corpus pins; a hand-run probe is exactly the drifting parallel verification the
one-executable-semantics discipline exists to prevent. Verifying primitives separately does not verify
they compose to the right bytes — a single-primitive slip (wrong mask, dropped continuation bit) is
invisible per-primitive yet miscompiles the emitted component.

**Resolution (🟡 — partially applied; the rest is a standing rule).** Applied: two `10-bytes.sexp`
cases now pin the unsigned-LEB128 encoder to its known answer (`624485 → b"\xe5\x8e&"` + base-case
`100 → b"d"`, both PASS). Standing rule for the operator to bless as practice: **every "verified
byte-correct" claim in a spike handoff must be promoted to a known-answer corpus case before it counts
as durable.** The outstanding claims to promote as their paths stabilize: the signed-LEB128 encoder
(`-300 → D4 7D`, boundary values), the section/vector length-prefix framing, and the core-module /
component envelope byte shape (currently only exercised end-to-end via ignition, not as a
byte-asserting corpus case). Learning:
`spec/learnings/2026-07-06-the-compilers-byte-emitting-spine-needs-a-known-answer-corpus-case.md`.

---
