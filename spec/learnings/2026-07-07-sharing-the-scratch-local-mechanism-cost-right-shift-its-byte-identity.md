# Sharing the scratch-local mechanism cost right-shift its byte-identity — reuse has a fidelity price

*2026-07-07*

**What happened.** A regression spot-check on the `agree` anchors — the strongest byte-gate bucket, which I'd
been trusting as unambiguous — caught that `(>> 256 4)`, byte-identical to native in Run 73 when shifts landed,
is now `soft` (value-correct → 16, but byte-different). Not a correctness regression (WRONG sweep stayed 0), but
an `agree → soft` fidelity regression, and probing it found a precise cause:

`compiler.cdz`'s shift emit reuses the **checked-arithmetic scratch-local mechanism**, which reserves a uniform
**3** scratch slots (arithmetic's `+ - *` need 3: two operands + result). But the two shift directions need
different counts, and native tailors them:
- `<<` (left shift) needs 3 (count-range guard AND left-shift overflow guard) → mine 3, native 3, **agree**.
- `>>` (right shift) needs only 2 (count-range guard; a right shift can't overflow) → native **2**, mine **3**
  (one unused slot) → `>>` is now `soft`.

So when shifts landed by *reusing* the arithmetic scratch-local machinery (the reuse that made shifts cheap
wiring — [[shifts-landed-as-the-second-guarded-op…]]), `>>` inherited arithmetic's 3-slot reservation and
over-declared one local, dropping out of byte-identity.

**Why.** This is the fidelity cost of the capability-reuse that was, correctly, celebrated as progress. Reusing
one mechanism (the local-allocating scratch-slot machinery) across operations (checked arithmetic, then shifts)
is the right architecture — it turned shifts from fresh work into wiring — but a *shared* mechanism emits the
*union* of what its clients need, and a client that needs less (right shift: 2 slots) pays for the client that
needs more (arithmetic/left-shift: 3). The value is unaffected (an unused local is harmless at runtime), but
byte-identity is not: native, emitting each op with a bespoke minimal footprint, declares exactly what each needs.
So `agree` (byte-identical) is where reuse-vs-bespoke shows up, and it is the *only* bucket that shows it —
`soft` and value-correctness are blind to a spare local. The lesson: **a byte-identity target is stricter than a
correctness target in a way that penalizes mechanism reuse — the same sharing that makes a capability cheap to
extend costs byte-fidelity on the clients that need less than the shared mechanism provides.** This is not an
argument against the reuse (it was right); it is the reason the last mile to `agree` on a reused mechanism is
per-client tailoring (here, a direction-specific scratch-local count for shifts).

Also a process note: the spot-check that caught this was a deliberate guard on the assumption that `agree` is
stable — I'd treated "agree count rose" as unambiguously good, but a rising count can hide a previously-agree
case dropping to soft while a new one joins. Checking that the long-standing anchors *stay* agree is the cheap
guard; here it caught a real (if minor) fidelity regression the aggregate count masked. **A count is not a set:
"agree went from 61 to 65" does not prove the 61 are a subset of the 65.**

**The requirement it drove.** No new corpus case — the shift value/trap behavior is already fully pinned, and
this is a byte-fidelity gap the byte gate's `agree` count measures directly, not a value the corpus oracle can
express. The output is ask-41 (low priority: make the shift scratch-local count direction-specific — `>>`/`>>>`
reserve 2, `<<` reserves 3 — and optionally match native's operand-stash order; moves `>>` `soft → agree`, no
value change). It is explicitly LOW priority: `>>` is correct and traps correctly; only its byte-identity to
native is off, by one unused local. General lesson: **when a reused mechanism drops a client out of `agree`,
that's the fidelity signal to tailor the mechanism per client — and it's worth catching, because it is invisible
to every gate weaker than byte-identity, and a rising aggregate `agree` count can hide it.**
