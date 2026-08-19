# Mixed tolled/untolled arms in a multi-op effect (2026-08-19)

BOUNDARY: in a multi-op effect where SOME arms carry post-resume tolls,
a tail-resumptive (untolled) arm alongside them DECLINES at the fold
boundary — but ALL-tolled arms fold:

- pyx3-decline.sexp: tolled tick + UNTOLLED transplant poke -> declines.
- pyx4-decline.sexp: tolled tick + UNTOLLED additive poke   -> declines.
- pyx5.sexp: BOTH arms tolled -> PASSES x3 (719100 / 716080,
  CPS-modeled; tick-poke-tick tolls unwind innermost-first).

Note pym1 (corpus, batch 325-era) had two DIFFERENT toll shapes and
passed — consistent: the fold handles per-arm toll VARIETY but not
toll/no-toll MIXING within one effect. 7th face on the later-increment
flip watch. pyx2 (untolled poke, untolled tick) passed earlier — the
mix is the trigger, not the poke itself.
- `pyx6.sexp` — a TOLLED PASSIVE peek between tolled ticks: advances
  nothing, charges 100000*s of the state it observed; the second tick
  reads exactly what the first wrote (205030 / 102010, CPS-modeled).
  Confirms the all-tolled fold surface covers passive arms too — the
  toll, not the state advance, is what the mixing boundary keys on.
  PASS x3 at 0c95d1a44.
