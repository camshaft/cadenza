#!/usr/bin/env bash
# Run cdz-runtime under Miri — the UB-freedom check for the hand-written refcount accounting.
#
# Usage (from anywhere; the script resolves the repo root itself):
#   implementation/seed/crates/cdz-runtime/miri.sh [TEST_FILTER]
#
# With no argument it runs the whole crate under Miri; with a filter it runs only matching tests, e.g.
#   .../miri.sh prop_bytes_matches_reference_under_random_op_sequences
#
# See MIRI.md (beside this script) for WHY each piece is load-bearing. The short version:
#   - CWD must be the repo ROOT so this crate's `.cargo/config.toml` build-std is NOT picked up
#     (nightly Miri honors build-std -> E0152 "duplicate lang item in crate core"); cargo reads
#     `.cargo/config.toml` from CWD upward, not from the --manifest-path location.
#   - RUST_MIN_STACK is a plain BYTE COUNT (67108864 = 64 MiB), never a suffixed "64M" (rustc rejects it).
#   - -Zmiri-ignore-leaks: the runtime holds deliberate process-lifetime immortals (EMPTY_BYTES/EMPTY_STR).
#   - -Zmiri-disable-isolation: proptests seed RNG from the clock/OS entropy.
set -euo pipefail

# Resolve the repo root from THIS script's own location (not CWD, not git) so it works from anywhere,
# including outside a git checkout. The script lives at <repo>/implementation/seed/crates/cdz-runtime/,
# so the repo root is four directories up from the crate dir.
crate_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$crate_dir/../../../.." && pwd)"
manifest="$crate_dir/Cargo.toml"

cd "$repo_root"
exec env \
  MIRIFLAGS="-Zmiri-ignore-leaks -Zmiri-disable-isolation" \
  RUST_MIN_STACK=67108864 \
  cargo +nightly miri test --manifest-path "$manifest" "$@"
