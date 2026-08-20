# maxmin-clamp — resume answer packs max*10 + min of op-arg and state
## pymm1 — clamp(v): (if v>s then v*10+s else s*10+v). Model 31082/30081. PASS x3.
The if picks the ordering; two dispatches straddle the threaded state. Round-trip-safe. Promotable.
