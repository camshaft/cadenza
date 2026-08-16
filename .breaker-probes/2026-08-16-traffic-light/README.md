# trf1 — traffic light with pedestrian button (2026-08-16, tick 1609)

Attack: a LATCH consumed at a phase boundary. The tick arm is a 4-way nested-if
(timer-expiry test outer, then a 3-way phase roll) where the green→yellow
transition's answer AND rebuild both read the latch (`(+ 1 held)` yellow timer)
and simultaneously CLEAR it — a consume-and-clear in one branch. The press arm
guards the latch behind an if-in-if condition (`(if (= ph 0) (not (= (% n 3) 0)) false)`
— boolean short-circuit via nested if, first use of that shape as a CONDITION).

Differential: the seed gates the BUTTON, not the schedule. Latched (n=10) buys
a 2-tick yellow so every later phase boundary lands one tick late vs unlatched
(n=0) — rows diverge from position 2 onward and never re-align.

Model slip caught by cdz classify: wrote `/=` for not-equals (CDZ0101 unbound)
— corpus idiom is `(not (= ...))`. Fixed and re-gated.

Hand model: n=10 rows [20,21,11,120,110,220] → 20021011120110220;
n=0 rows [20,20,10,110,220,210] → 20020010110220210 (base-1000).

Pass ×3 wasm + rust + rust-async on trunk 68122fd42.
