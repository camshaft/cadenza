# DEFERRED: rust-baseline (.gate-baseline-rust / -async) refresh — HELD until the E0282 emit fix lands
breaker flagged (2026-07-16): .gate-baseline-rust + .gate-baseline-rust-async are broadly STALE — many
verification/pattern/closure cases landed WASM-ONLY and now show +pass/+todo DRIFT on a fresh rust save.
A refresh pass would restore rust-target baseline accuracy BUT:
🔴 MUST NOT refresh now — a fresh `gate --save --target rust` would flip the specsubst E0282 case (+ any
   other genuinely-failing rust emits) from todo→FAIL, committing a red baseline. And it would ADOPT the
   drift as "expected", masking real rust-emit bugs.
SEQUENCE: (1) v-rust-backend lands the E0282 turbofish emit fix (adv-rust-backend-untyped-none-... item);
   (2) THEN a gated `gate --save --target rust` refresh (verify the diff is only genuine new-passes /
   sound-declines, NOT masking a fail); (3) commit the refreshed baseline.
OWNER: corpus-bugfix (gate baselines) + v-rust-backend (confirm which drift is sound-decline vs real-fail).
Revisit after the E0282 fix lands. NOT urgent (the stale baseline is conservative — it doesn't hide the
E0282 as a pass; gate --check --target rust already SHOWS the FAIL).
