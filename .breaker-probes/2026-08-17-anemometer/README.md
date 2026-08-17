# anm1 — anemometer with running average (2026-08-17, tick 1709)

Attack: a RUNNING-AVERAGE compound `(/ (+ speed v) 2)` appearing x4 (peak
test, both answers, both rebuilds' speed — and the peak-branch stores it
TWICE, into speed AND peak: one compound feeding two fields of one rebuild).
The averaged value tested against a third field (peak) before storing. Lock
is an ECHO op — answers two fields, mutates only its counter (the pure-read
+ counter shape).

Differential: wind 10 vs 4: n=10's average stays high (9, 5, 8) with peaks
at gusts 1 and... rows [91,99,50,80] — peak set once (91) then never re-hit
(the decaying average); n=0 climbs (61, 40, 81 — peak reset at the LAST
gust). Peak-tag patterns [1,0,0] vs [1,0,1]; reads 981 vs 881.

Hand model: n=10 → 910990500800981; n=0 → 610660400810881 (mixed base).

Pass ×3 wasm + rust + rust-async on trunk 0657b816d.
