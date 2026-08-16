# Nested-record match-binder x effects (2026-08-11) — trunk 97c119eb9

Target: the fresh nested-record match-binder wiring ((tuple (record (x a)) c)
resolves the field, activated by ff32c8987's dormant Ty::Record arms). The
commit's own tests are pure-position; the dispatch faces were unpinned.

GREEN x3:
- nr1: the new pattern destructures a DISPATCHED tuple-with-record op argument,
  two dispatches with state advancing — 5760345/710340
- nr2: the pattern destructures the STATE (tuple-with-record), field read and
  record REBUILT per dispatch — 30014/11

Pin candidates: 240 pool (fresh-commit pins, high value — they gate the new
resolve path against the effect machinery).
