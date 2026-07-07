# A decline that lands on a trap-expecting oracle is coincidental agreement, not a semantic trap — the trap-oracle dual of reject-don't-miscompile

*2026-07-07*

**What happened.** A quiet cycle (no new spike code since the previous run) turned into a **completeness sweep**
of the interim corpus harness, which had grown a new `trap-ok` bucket: a case whose oracle expects a runtime
**trap** (`(trap "…")` — integer overflow, division by zero, byte out of range) and whose `compiler.cdz`
component also traps is scored `trap-ok` — "CORRECT, and stronger than a value match," per the harness README,
which cites `(/ 5 0)` compiling to a real `i64.div_s` that traps on the zero divisor. The full sweep reported
**22 agree, 6 soft, 4 trap-ok, 0 hard, 93 decline, 0 error, 124 n/a, 5 skip** — a clean board. But probing the
four realized `trap-ok` cases directly (disassembling the component `compiler.cdz` built for each) showed every
one of them is a **bare `unreachable`**:

| trap-ok case | input | compiler.cdz func 0 |
|---|---|---|
| member access of a missing field traps | `(let ((p (record (x 1)))) (. p z))` | `local i64; unreachable; local.set 0; unreachable` |
| byte out of range traps | `(Bytes.of (list 0 256))` | `unreachable` |
| byte negative traps | `(Bytes.of (list -1))` | `unreachable` |
| byte runtime out of range traps | `(mk n) = (Bytes.of (list n))` | `unreachable` |

None of these traps for the reason the case tests. `compiler.cdz` does not support `record` or `Bytes.of` yet —
the reader lowers them to `KError → unreachable` — so the trap is a **decline**, not the byte-range check or the
missing-field check the oracle is pinning. The clincher: a *valid, in-range* `(Bytes.of (list 65 66))`, which
should return a two-byte sequence and NOT trap, ALSO compiles to bare `unreachable` and traps. The construct is
declined wholesale; the byte value is never examined.

So the `trap-ok = 4` line is **coincidental agreement**: the right observable (a trap) for the wrong reason (an
unsupported construct, not the semantic). And the README's claim is half-right in a misleading way — the
*mechanism* is genuinely strong where it fires (`(/ 5 0)` really does emit `i64.const 5; i64.const 0;
i64.div_s`, a true semantic trap the const-folder deliberately refuses to fold — verified), but that case is
scored `n/a`, and **none of the four cases actually in the `trap-ok` bucket are semantic traps** — they are all
declines. The bucket conflates two behaviors that a value-only comparison cannot tell apart.

**Why.** This is the exact dual of the reject-don't-miscompile discipline
([[the-self-hosted-reader-miscompiles-unsupported-constructs-instead-of-declining]]), one axis over. On a
*value*-expecting case a decline is visibly distinct from a correct answer — the component traps where a value is
wanted, and the harness scores `decline` (the honest frontier). On a *trap*-expecting case that distinction
collapses: a decline and a correct semantic trap produce the identical observable (`unreachable`), so a
value-first harness cannot separate "compiler correctly implemented the trapping semantic" from "compiler
doesn't support the construct and declined onto a trap." The danger is not today's behavior — declining
`Bytes.of` is correct right now — it is the **masking**: when `Bytes.of` gains real support, a component with a
*wrong* range check (say, off-by-one, trapping at 255 instead of 256, or not trapping at all on a valid byte)
would still score `trap-ok` for the out-of-range cases and could regress silently, because the bucket never
distinguished the decline from the real check in the first place. A green `trap-ok` count reads as "these
trapping semantics are conformant" when it currently means "these constructs are unsupported and decline."

**The requirement it drove.** No corpus change — the four cases are correct as written (they pin real trapping
semantics native implements); the gap is in the *measurement*, not the spec. Two durable outputs. First, a
harness caveat (operator-facing, in the interim harness README): `trap-ok` must be read as "traps, reason
unverified" until the trap's *cause* is checked — a bare-`unreachable` decline is not a semantic trap, and the
honest count distinguishes them (disassemble, or better, once the construct is supported, pair each
out-of-range case with a matching in-range case that must NOT trap — the in-range companion is the discriminator
a value-only trap oracle lacks). Second, a SPEC-BACKLOG measurement note: the eventual `component-check`
differential (SPEC-BACKLOG #22, now unblocked seed-side) has the same blind spot — a trap-vs-trap comparison
agrees whether the trap is semantic or a decline — so the real differential gate needs a **trap-cause
discriminator**, most cheaply the in-range companion case, before a `trap-ok`/agree on a trap-expecting case
counts as conformance. General lesson, a recurrence of this loop's standing rule: **a green aggregate deserves
a direct probe, especially a bucket that agrees by coincidence — `trap-ok` looked like the strongest evidence on
the board and was the weakest, because the one observable it compares (does it trap?) is exactly the one a
decline and a semantic trap share.** The buckets that agree for a *reason unique to correctness* (a byte-
identical `agree`, a value-matching `soft`) are trustworthy; a bucket that agrees on an observable a decline can
counterfeit is not, until the reason is verified.
