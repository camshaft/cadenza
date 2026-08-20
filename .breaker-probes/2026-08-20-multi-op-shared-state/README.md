# multi-op-shared-state — two ops of one effect thread the SAME state, alternating

## pymo1 — inc (advance, no read-scale) + get (read-scale, no advance), interleaved
```
(effect E (op inc (-> Int64)) (op get (-> Int64)))
((inc () s (resume s (+ s 5)))
 (get () s (resume (* s 10) s)))
body: 1000*inc + 100*get + 10*inc + get   (evaluated inc,get,inc,get)
```
inc answers the current s and threads s+5; get answers s*10 and threads s unchanged.
Model: n=10 s0=1: inc=1(s->6), get=60(s=6), inc=6(s->11), get=110 -> 7170.
       n=0  s0=0: inc=0(s->5), get=50, inc=5(s->10), get=100 -> 5150.

## Verdict: PASS-WITNESS (compiles + correct)
Verified 7170 / 5150 on wasm + rust + rust-async (fresh worktree-local cdz).
Exercises the tail-resumptive fold threading one shared state across DISTINCT ops
(inc vs get) of a single effect, each with its own answer AND its own next-state rule,
interleaved in the body. Distinct from all prior single-op state-thread probes.
