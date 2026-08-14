# ffs1 — first-free-slot bitmask allocator (2026-08-14, tick 1479)

Int64-bitmask state: `alloc` finds the lowest clear bit via a recursive probe
(lowest0, width-8 scan) and sets it, answering the index; `freeb` clears a bit
answering whether it was live. The SEED pre-occupies slots (n=10 = 0b1010:
slots 1,3 busy; n=0: empty), so the runs allocate around different holes:
n=10 → 0,2,free(1)=1,1,4,free(6)=0,5 → 20101040005
n=0  → 0,1,free(1)=1,1,2,free(6)=0,3 → 10101020003
The free(6) is a live-vs-absent differential only via mask state; the last
alloc lands in a different hole per seed.

Complements bms1 (batch 266, explicit set/clear by index) with the SEARCHING
face (allocator picks its own bit). Bit ops: & | ^ << >> per bms1 syntax.

PASS ×3 wasm. **Pool.**
