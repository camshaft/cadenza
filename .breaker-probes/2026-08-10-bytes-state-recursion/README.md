# Bytes-state recursion + slice windows (2026-08-10)

Angle: a Bytes handler STATE accumulated across recursive dispatches (rope growth
per hop), and arm-returned SLICE windows over the growing state.

All GREEN x3, python-modeled first:
- br1: Bytes state grows one byte per recursive dispatch (walk 3 -> 3 pushes);
  the dumped frame's length + first/last bytes pin the accumulation — 3691
- br2: the arm returns a slice WINDOW (drop-first) over the grown state; two
  pushes give windows [6,40] then [6,40,50] — 260410
  (two authoring slips caught pre-gate: Bytes.slice returns Option (must match),
  and my first pin 260420 disagreed with the python model 260410 — model wins.)

Vocab: Bytes.of takes (List UInt8) — wrap with UInt8.wrap; Bytes.at/Bytes.slice
return Option (match Some/None); no Bytes.empty/Bytes.push — use
(Bytes.of (list)) and Bytes.concat with a 1-byte frame.

Pin candidates alongside the 222/223/224 pools.
