# Euclid trace (2026-08-12)

Angle: GCD via Euclid where the effect LOGS every remainder step — the
algorithm-trace shape (real programs instrument loops this way). The divisor
chain length and values are data-dependent per seed (18: 12,6 -> gcd 6;
35: 12,11,1 -> gcd 1; 12: 12 -> gcd 12), k=20 budget guard.

GREEN x3:
- gcd1: 6018/1024/12012 (gcd*1000 + divisor-chain sum)

Staged: pbr1/pbr2 + sqm1 + cz1 + gcd1 (5 fresh — next 14c batch ready).
