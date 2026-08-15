# twn1 — structurally identical twin effects (2026-08-15, tick 1541)

POST-S3-FLAG-DAY identity probe (25ccebf3d deleted EffectRequest.content_type
+ EffectKind — effect identity is schema-hash-only now). Two effect
declarations with the SAME op name and SAME signature (bump: Int64→Int64),
differing only in the effect name, nested as an A-outer/B-inner tower.
Interleaved draws must dispatch by effect identity alone: A threads the seed,
B threads 100 — rows 13,103,18,108,19 / 3,103,8,108,9 (B rows seed-invariant
anchors, A rows seed-shifted).

If schema-hash-only identity ever collapses same-shape effects (hash of the
structural schema colliding when names are excluded — the exact question a
hash-of-shape regime raises), this pins the failure loudly: B's draws would
land on A's handler and every row shifts.

Verified on ALL THREE backends ×(3,1,1) on the flag-day base. **Pool.**
