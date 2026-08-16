# Self-concat sharing (2026-08-11) — 05-compound target (NOT 14-effects)

Angle: (List.concat xs xs) — the same rope twice in one concat — then the
original grows and re-reads. An FBIP-mutating concat on the rc==1-looking
second operand would corrupt the doubled view or the later reads.

GREEN x3:
- lc1: doubled len/content exact, grown len exact, original re-read exact —
  403034/400031

TARGETS 05-compound-types (pure position) — appendable DURING the 14-effects
hold since it routes to a different file. Pin candidate.
