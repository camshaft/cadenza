# Framed-Bytes handler states (2026-08-11)

Angle: the bf-family (landed) frames Bytes as op ARGS/RESULTS; the STATE
ITSELF being a bin-framed Bytes — decoded and re-encoded every dispatch —
was uncovered. The state thread becomes a codec round-trip per hop.

GREEN x3:
- bf1: (u8 gen)(u16 val) state; each dispatch bumps both fields and
  re-frames — 201501005/201001000
- bf2: MIXED-ENDIAN state (BE hi + LE lo); per-segment order survives every
  re-frame — 51701034/100002

Pin candidates: staged pool.

## v-effects NOTE (2026-08-11): bf1 REDUNDANT — the framed-Bytes-state-per-dispatch capability is already pinned at 14-effects:4463 (a FRAMED-Bytes handler state decoded and re-encoded by the arm per dispatch). Not re-pinning bf1. bf2 mixed-endian likely covered by landed mixed-endian pins too; skip unless a distinct face emerges.
