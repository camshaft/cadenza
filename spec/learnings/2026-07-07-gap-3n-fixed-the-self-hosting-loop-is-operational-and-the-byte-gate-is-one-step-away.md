# Gap 3n is fixed — the self-hosting loop is operational for arbitrary programs, and the byte-level gate is one small seed step away

*2026-07-07*

**What happened.** The `compile`-return alignment bug (gap 3n) — which the previous two cycles narrowed from
"fails at every size" → "value threshold at 24" → "input-length mod 4" (`retptr = base + input_len`, 4-aligned
only when `input_len % 4 == 0`), converging with the compiler agent's own diagnosis and the fix `(p+3)&!3` —
**landed in the seed** (a rebuild between cycles). Re-probing every input that failed last cycle confirmed it:
`(main) 5`/`0`/`1`/`true` (input AST len 31), `1000` (33), `(module mmm (def (main) 42))` (34), and an unfolded
`(if (< 3 5) 42 99)` — **all now return `Ok`**, across all four mod-4 residues. The return marshalling is robust
regardless of input length.

The consequence is the milestone: the self-hosting `compile-run` loop now works for **arbitrary** programs, not
just the `len % 4 == 0` ones. Driving `compiler.cdz` (as its real `(def (compile b) (compile-bytes b))` entry)
over a spread of programs and byte-comparing against native `cdz-rustc`:

| program | native | compiler.cdz | verdict |
|---|---|---|---|
| `(main) 42` | 89 B | 89 B | **byte-identical** |
| `(main) (< 3 5)` | 89 B | 89 B | **byte-identical** |
| `(main → isLt → lt → <)` depth-2 chain | 124 B | 124 B | **byte-identical** |
| `(main) (+ 20 22)` | 128 B | 89 B | soft (value-correct; native emits overflow helpers, mine folds) |
| `(main) (dbl 21)` | 145 B | 105 B | soft (same reason) |

So on the programs where byte-identity is expected (no overflow-checked arithmetic to fold away), the
Cadenza-authored compiler is **byte-for-byte the native compiler**, driven through the real `bytes → bytes`
seam — the actual self-hosting agreement, not a value-only proxy.

**Why.** This closes the arc that ran across the last several cycles: the self-hosting loop was *functionally*
closed once the entry was rewired to `compile` (gap 3l), but gap 3n made the return path unreliable for most
inputs, so the loop's own test harness had to stay the interim `emit`-based value check that sidesteps the
compile-return path. With 3n fixed, that workaround is no longer forced — the compiler can be driven through the
exact ABI it will ship. The reporting channel to the compiler agent (established the prior cycle) paid off
immediately: the loop had handed the agent the mod-4 root cause and the `(p+3)&!3` fix, the agent (or the same
fix, independently) applied it, and this cycle the loop *verified the fix landed* and told the agent so — a full
report → fix → confirm round trip through the `📡 FROM THE CONFORMANCE LOOP` section, exactly the loop's job.

**The requirement it drove.** No corpus case — gap 3n was a seed component-ABI defect (return-pointer alignment
in the hand-emitted `compile` wrapper), not a language behavior with a value oracle, and the *values* it would
have corrupted are already pinned by the ordinary corpus cases (they now pass through `compile-run` too). The
durable output is this learning plus a precisely-scoped next step handed to the compiler agent and the operator:
**the byte-level self-hosting gate (`component-check`) is one small seed step away.** `component-check` already
does the whole-corpus native-vs-component byte diff, but it reads a compiler component from a fixed crate path
and cannot be pointed at a *`compiler.cdz`-built* compile-component; `compile-run` builds exactly that component
in memory but never writes it to disk. The missing piece is a seed subcommand (or a `compile-run` flag) that
**persists the compiler.cdz compile-component**, after which `component-check <that> spec/semantics` grades the
corpus at the byte level — the real differential self-hosting gate, replacing the interim value harness. (Corpus
REJECTION cases additionally need the diagnostics ABI — the `compile` export returning `result<list<u8>,
list<diagnostic>>` rather than trapping — a separate, later gap; SUCCESS cases are gradeable the moment the
component is persistable.) General lesson, the payoff of the report-to-agent discipline: **a loop that both
hands the agent a root cause AND verifies the resulting fix closes the feedback edge** — the agent doesn't have
to re-derive whether its fix worked on the real workload, because the loop re-probes the live binary and reports
back; the value of the loop is not just finding gaps but confirming they stay closed.
