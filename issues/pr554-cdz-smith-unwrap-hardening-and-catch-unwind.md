# pr554 — cdz-smith robustness sweep: path/new_path .unwrap() + catch_unwind critique (5 amazon-q comments)

Mirrored from GitHub PR #554 review comments (all amazon-q-developer[bot]).
PR: https://github.com/camshaft/cadenza/pull/554 (12-MR publish batch, MERGED to trunk)
Files: `cdz-smith/src/differential.rs` (4) + `cdz-smith/src/finding.rs` (1).
Same class as PR#552's cdz-smith robustness sweep — fuzzer territory. amazon-q source → VERIFY
loci (line numbers shift post-merge).

## .unwrap() hardening (4) — reasonable
- id 3607250145 differential.rs:208 — `changes.new_path.unwrap()` panics if None. Suggest `.ok_or_else`.
- id 3607250160 differential.rs:250 — `path.to_str().unwrap()` panics on non-UTF8 path. `.ok_or_else`.
- id 3607250159 differential.rs:386 — `path.to_str().unwrap()` panics on non-UTF8 path. `.ok_or_else`.
- id 3607250155 finding.rs:336 — `path.to_str().unwrap()` panics on non-UTF8 path. `.ok_or_else`.
These are legit-but-low-priority robustness (non-UTF8 paths are rare; an internal fuzzer, not a
service). Fine to harden in a sweep; your call on priority.

## catch_unwind critique (1) — SKEPTICAL, likely WRONG advice
- id 3607250142 differential.rs:242:
> :stop_sign: Logic Error: The panic recovery mechanism is flawed. Catching panics with
> `catch_unwind` and continuing execution silently discards errors without propagating them. This
> masks failures that should be reported. Remove panic handling and let errors propagate naturally...

⚠️ For a FUZZER/differential oracle, catching a panic and continuing is usually the INTENDED design
— you want to record the panic as a finding and keep fuzzing, not abort the whole run. amazon-q's
"remove panic handling" is probably backwards here. The only real question is whether the caught
panic is turned into a recorded finding vs silently swallowed. Fuzzer owner should judge: if the
catch_unwind result IS surfaced as a finding, dismiss; if it's truly discarded, wire it to a finding.

## Owner
All cdz-smith → fuzzer (consistent with PR#551/#552 oracle routing).
