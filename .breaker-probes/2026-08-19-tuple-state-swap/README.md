# tuple-state-swap — pair state, each tick threads the SWAPPED pair as next-state
## pyts2 — state (a,b); tick answers a*10+b, next-state (b,a). Model 15051/5050. PASS x3.
Two dispatches read the fields in opposite roles; an order-insensitive swap or wrong thread flips the packed value. Round-trip-safe (no nested-do/list). Promotable.
