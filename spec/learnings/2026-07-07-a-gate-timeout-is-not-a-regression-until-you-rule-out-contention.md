# A gate timeout is not a regression until you rule out contention — isolate before escalating

*2026-07-07*

**What happened.** After a seed rebuild, the behavior gate — which normally finishes in ~2 s — **timed out at
2 minutes**, twice. The tempting read: the rebuild introduced a hang or an exponential blowup (a real category
here — the compile-cost-exponential and fixpoint-OOM families have bitten before), and the seed just regressed.
Isolating it instead told a different story:

- Per-corpus-file, `10-bytes.sexp` showed `30 s rc=124` (timeout) — seemingly the culprit.
- But bisecting it, every half and every quarter passed in ~1 s.
- Running `10-bytes.sexp` alone: **1 s, 51/51 PASS.**
- Re-running the *full* gate once the box was quiet: **2 s, 569 PASS, green.**

So there was no hang. The 2-minute timeouts were **transient contention** — the box was under load (the sibling
agent's concurrent seed rebuild competing for CPU/IO at the same timestamp), and a normally-2 s run stretched
past a 2 min wall clock. The per-file "30 s timeout" was the same contention hitting one file's turn in a loop,
not that file being slow (it runs in 1 s alone).

**Why.** A timeout is a *wall-clock* signal, and wall-clock conflates two very different causes: the work got
slower (a regression) or the machine got busier (contention). On a box where a sibling agent rebuilds the seed
and runs its own probes concurrently, contention is common, and a gate that shares the machine will occasionally
stretch. The discipline that separates the two is cheap and must come *before* escalating: **re-run the suspect
in isolation and time it against its known baseline** — if `10-bytes` alone is 1 s and the full gate alone is
2 s, there is no regression, only a loaded box. Escalating a contention timeout as a regression is a false alarm
that wastes a cycle chasing a phantom (and, worse, could trigger a "revert the seed" reaction to a seed that is
fine). The inverse error is as bad — dismissing a *real* exponential blowup as "probably contention" — so the
rule is not "ignore timeouts," it is "isolate and time against baseline; a genuine regression reproduces in
isolation, contention does not." Here it did not reproduce: 1 s and 2 s, clean.

**The requirement it drove.** No corpus case, no ask, no learning about the seed — because there was no seed
defect; the finding is about the *loop's own reaction to a timeout*. The durable output is this discipline note
and a standing rule for the loop: **a gate/probe timeout is triage, not a verdict — before recording a
regression, re-run the suspect alone and compare to its baseline runtime; a real slowdown reproduces in
isolation, contention (a busy box, a concurrent sibling rebuild) does not.** (This cycle's actual result: the
gate is green in 2 s, WRONG=0, and ask-44 — the stray `DBG` `eprintln!` flagged last cycle — was removed by the
14:02 rebuild, verified 0 occurrences and moved to done. The false alarm was the only thing that looked like a
finding, and dismissing it correctly *was* the cycle's work.) General lesson: **on a shared machine, wall-clock
timeouts are noisy; the loop's probe-don't-trust discipline extends to its own tooling's timing — re-probe in
isolation before believing a timeout means the artifact got worse.**
