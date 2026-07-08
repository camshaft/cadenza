# Coverage advances through two doors — decline→agree (byte-identical) and decline→soft (value-correct, byte-differ) — and the soft door opening is its own milestone

*2026-07-07*

**What happened.** For several cycles the coverage phase moved only through the agree bucket (const-fold wins,
decline→agree, byte-identical to native). This cycle the shape changed: agree held at 134, but **soft jumped
26→37 (+11)** and decline dropped 421→410. Confirmed it was real compiler.cdz progress (the emitted component
differs from last cycle; the corpus was stable at ~785 cases), and characterized the +11: they are
runtime-SCALAR function/binding cases — multi-arg functions (`(def (add3 a b c) (+ (+ a b) c))`), let-in-function
(`(def (f n) (let ((x (+ n 1))) (+ x x)))`) — that previously DECLINED and now EMIT runnable code producing the
correct value, just with a different byte layout than native (soft = same value, different bytes). Genuinely
runtime (arg-dependent, not const-foldable); the byte gate stayed 0 disagree, so every one is value-correct. The
runtime-COMPOUND tier (M2) still declines, and HOF still declines — this is specifically the runtime-scalar emit
path maturing.

**Why.** In a differential gate with a four-bucket classifier (agree/soft/disagree/decline), coverage — moving a
case out of decline — has TWO destinations, and they mean different things:

- **decline → agree**: the compiler emits code BYTE-IDENTICAL to native. For the const-fold tier this is easy
  (both fold to the same `i64.const`), which is why the early coverage gains all landed here.
- **decline → soft**: the compiler emits code that RUNS to the same value but with different bytes. This is the
  compiler gaining a real EMIT capability — it can now compile and correctly run a class of programs it used to
  refuse — while its instruction selection / layout hasn't yet matched native's exact bytes.

The soft door opening is its own milestone, distinct from and often preceding agree, because **value-correct
emission comes before byte-identical emission.** A compiler learning to emit runtime scalar function bodies will
first produce *correct* code (soft), then be tuned to *byte-match* the reference (agree) — the two are separate
engineering steps, and the soft bucket is where a freshly-online emit path lives before the fidelity work. So a
cycle where agree is flat but soft rises is NOT stalled coverage; it is coverage advancing through the other door
— the compiler crossed the "can I emit this at all, correctly?" threshold for a class, which is the harder and
more important threshold. Byte-fidelity (soft→agree) is a follow-on refinement; value-correctness (decline→soft)
is the capability.

The measurement discipline this sharpens: **track soft as a coverage-progress signal, not just agree.** Reporting
only "agree held at 134" would read as a stalled cycle, when in fact the compiler gained runtime-scalar emit on
~11 cases — a real capability step that agree-counting misses. The honest coverage metric is `agree + soft` (cases
the compiler runs correctly), with agree being the byte-fidelity subset; decline is the true not-yet-covered
count. And the soft door is the leading indicator for a new emit path: when a previously-declining CLASS starts
showing up as soft (not one case but a cluster — here multi-arg functions and let-in-function together), an emit
path came online, and the follow-on is byte-fidelity tuning to convert soft→agree.

**The requirement it drove.** No new corpus case — the ~11 cases are already pinned (that is how the gate saw
them move decline→soft), and they convert soft→agree when the compiler's byte layout is tuned to match native.
The output is this learning and the characterized shift (soft 26→37, runtime-scalar function/binding emit now
online and value-correct; M2 runtime-compound and HOF still decline). General lesson: **coverage in a differential
gate advances through two doors — decline→agree (byte-identical) and decline→soft (value-correct, byte-differ) —
and the soft door is the real capability milestone (a compiler emits CORRECT code before it emits byte-MATCHING
code), so track `agree + soft` as coverage and watch a class moving decline→soft as the signal an emit path came
online; a flat agree count with rising soft is coverage advancing, not stalling.**
