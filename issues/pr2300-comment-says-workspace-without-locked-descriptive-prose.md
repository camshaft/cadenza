# PR #2300 review — flake.nix (v-nix) — OPEN — comment/command wording [VERIFIED-weak, LOW / mostly-dismiss]

https://github.com/camshaft/cadenza/pull/2300 (revert checks.test to `cargo test --workspace` option-b —
crane cargoTest regressed test-ubuntu; keep clippy on crane. A course-correction on the crane arc:
#2262/#2273/#2279/#2282/#2286 clippy stays, test reverts). Copilot 1 inline (id 3724174046, flake.nix:426,
also 1658, 1701).

## comment says checks.test reverted to `cargo test --workspace` but the actual check runs `cargo test --workspace --locked` — update the comment to match (+ the `--locked` reproducibility intent) (Copilot, flake.nix:426, also 1658/1701) — wording [VERIFIED-weak, LOW]
> The comment says checks.test reverted to `cargo test --workspace`, but the actual check runs `cargo test
> --workspace --locked`. Update the comment to match the real command (and the reproducibility intent of
> `--locked`). This issue also appears at line 1658, 1701.

VERIFIED — but WEAK, and the "also appears" sites are NOT the same defect (per-site checked, per
[[liaison-copilot-also-appears-at-line-N-secondary-occurrence-needs-per-site-verify]]):
- The ACTUAL command is correct: flake.nix:1666 `cargoCmd = "cargo test --workspace --locked"` — has
  `--locked`. And the primary structural comment at 1651 ALREADY says "`cargo test --workspace --locked`".
- The sites Copilot flags are DESCRIPTIVE PROSE, not command specs, and split by kind:
  - 1658: "Coverage parity = the INC 2 whole-workspace run (`cargo test --workspace` = ∪ all member test
    binaries)" — this describes the COVERAGE SHAPE (union of member binaries); `--locked` is a
    reproducibility flag IRRELEVANT to a coverage-union statement. NOT a defect.
  - 1701/1702: "checks.test is a whole-workspace `cargo test --workspace`, option-b" — an INFORMAL
    identity mention of the check, not the literal invocation.
  - (Others in the file — 181/183/263/424 — are the same informal/coverage-shape shorthand.)

So the finding is LOW / mostly-dismiss: the real `cargoCmd` is correct + reproducible (`--locked` present at
1666), and the flagged comments are informal shorthand for the check's identity/coverage where the flag is
immaterial. There's no command/behavior bug. If v-nix wants prose-precision it can append `--locked` to the
bare mentions, but this is optional polish, not a correctness or reproducibility issue (the reproducibility
comes from 1666, which is right). v-nix owns flake.nix. PR OPEN. (Flagged the Copilot "also-at-line-N"
per-site divergence explicitly — 1658 is coverage-shape, not command.)
