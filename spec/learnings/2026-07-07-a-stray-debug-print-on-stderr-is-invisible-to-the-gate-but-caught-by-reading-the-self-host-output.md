# A stray debug print on stderr is invisible to every gate — the self-hosting probe caught it because it reads the whole output

*2026-07-07*

**What happened.** Probing the self-hosting `compile-run` output this cycle surfaced a line the gates never see:
`DBG ctor-arm match, scrut_kind=Int64, scrutinee=Name("node")`. It is a leftover `eprintln!` in the seed's
codegen (`codegen.rs:4296`), inside a guard that fires when a `match` has a constructor-pattern arm but a
non-Heap scrutinee — a diagnostic tripwire the agent left in while working the ctor-arm-match kind inference. It
prints once while the seed compiles `compiler.cdz` itself.

Why no gate caught it: it is on **stderr**. The behavior gate reads the corpus result (0 DBG lines — the trace's
guard doesn't fire on the corpus's own programs, only on compiler.cdz's `node`-scrutinee ctor-match); `emit`'s
**stdout** (the emitted bytes) is untouched; `component-check`'s stdout parsing is untouched; the WRONG sweep
runs the built components, not the compiler's stderr. So every automated check is blind to it by construction —
it corrupts no bytes, flips no case, moves no count. The only thing that saw it was a human-style read of the
*full* `compile-run` output (stdout + stderr) on the self-hosting path.

**Why.** Two small lessons. First, **a gate measures the channel it's built to measure, and debug noise on any
other channel is invisible to it** — stderr, log files, timing, memory are all outside a stdout/bytes gate's
view, so a stray print (or, more dangerously, a stray *warning* that should have been an error, or a perf
regression) can ride along indefinitely without a red gate. The loop's habit of reading the *whole* artifact
output, not just the classified result, is what catches these; a gate's green is silence on its channel, not
silence everywhere. Second, **a debug tripwire is a map of where the implementer is actively uncertain** — the
`eprintln!`'s guard condition (a ctor-pattern arm with a non-Heap scrutinee, here `Int64`/`node`) is precisely
the inference edge the type-inference work is still probing; the trace isn't just litter, it points at the live
case. When it disappears, that case resolved.

**The requirement it drove.** No corpus case — a stderr debug print is not a value-behavior; the corpus oracle
has nothing to say about it. The output is ask-44 (LOW: remove/gate the `eprintln!`; and note it marks the
ctor-arm/non-Heap-scrutinee inference case the agent is working), reported to the compiler agent. Severity is
hygiene, not correctness: the gate is green (569), WRONG=0, agree anchors stable — the print changes nothing an
automated check measures. General lesson: **read the whole output of the self-hosting run, not just the gate's
verdict — a gate is green on its own channel while noise (or worse) accumulates on the channels it doesn't
watch, and the cheapest way to catch a stray print, a should-be-error warning, or a silent perf cliff is to look
at everything the compile actually emitted, stdout and stderr both.**
