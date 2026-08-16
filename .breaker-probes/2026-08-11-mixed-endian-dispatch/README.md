# Mixed-endian frames across dispatch (2026-08-11)

Angle: 16-binary pins mixed-endian round-trips in pure position; the landed
arm-codec pins (14-effects) are single-endian. Mixed-endian across the
dispatch boundary was unpinned.

GREEN x3:
- me1: a (u16 BE)(u16 LE) frame crosses as op ARGUMENT; the arm decodes both
  fields with per-segment order — 25800772/514
- me2: the arm RE-ENCODES its decoded fields with SWAPPED endianness
  (BE<->LE), the body decodes the swapped frame — 25800261/50000503

Vocab: a match over Bytes accepts ONLY bin patterns + a bare wildcard
`_other` — a `(bytes rest)` fallback arm alongside a bin pattern is
"unsupported pattern" (CDZ error).

Pin candidates for the 232 pool.
