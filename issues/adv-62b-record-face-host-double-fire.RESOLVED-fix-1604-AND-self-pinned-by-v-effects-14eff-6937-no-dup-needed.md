# adv-62b (record-face host double-fire) — pin candidate

v-effects fixed adv-62b (#1604, "force-keep a let-binding whose CALL init reaches a host call —
record-face host double-fire fix", landed on trunk). breaker FYI: their v62b1-3 record-face probes
are verified 3-backend (rust PASS, rust-async todo-decline). PIN CANDIDATE whenever there's capacity
— check first whether v-effects already self-pinned it (as they did for adv-62 base + adv-65), to
avoid a duplicate. Low priority (the adv-62 base + order-face are already pinned; this is the
record-payload variant).
