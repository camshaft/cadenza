# three-level-nest — 3 handlers (A over B over C), innermost body draws all three
## py3n1 — A doubles / B +5 / C +900, three independent state threads, packed. Model 90752/90750. PASS x3.
Deeper routing than 2-level: three effects route to their correct handlers from the innermost body; each state thread advances independently. Round-trip-safe. Promotable.
