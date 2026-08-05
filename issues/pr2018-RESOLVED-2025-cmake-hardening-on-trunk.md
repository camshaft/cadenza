# PR #2018 review — flake.nix (v-nix) — OPEN — build-break [VERIFIED, near-certain] — DE-RISK PRE-MERGE

https://github.com/camshaft/cadenza/pull/2018 (full-CI-in-nix increment 5 — cdz-agent-host native check).
Copilot (id 3712026650) flags the check can't build the live-net AWS crypto stack without a C toolchain.
Unlike the #1999 blake3 case (which had a pure-Rust fallback → dismissed on green), THIS one has NO
fallback — VERIFIED near-certain build-break.

## `cdzAgentHostNativeCheck` (`stdenvNoCC`, `nativeBuildInputs = [rustToolchain]`) runs `cargo clippy --features live-net`, but the live-net closure pulls `aws-lc-sys` which needs cmake + cc + pkg-config → build FAILS (Copilot, flake.nix:458) — build-break [VERIFIED]
> `cdzAgentHostNativeCheck` is built with `pkgs.stdenvNoCC`, but `cdz-agent-host`'s dependency closure
> includes `aws-lc-sys`, which pulls in `cc`, `cmake`, and `pkg-config` (native tool requirements). With
> `stdenvNoCC` and no `cmake`/`pkg-config` in `nativeBuildInputs`, this derivation is expected to fail to
> build when compiling the `live-net`/AWS crypto stack.

VERIFIED against the committed cdz-agent-host lock: `aws-lc-sys 0.43.0` (Cargo.lock:125) is present, pulled
by `aws-lc-rs` (:115-120) — the DEFAULT rustls crypto provider that the aws-sdk + reqwest(rustls-tls) stack
selects. `aws-lc-sys`'s build script is a C/asm build driven by **cmake** (+ cc, pkg-config) — and it has
NO pure-Rust fallback (this is the sharp difference from #1999's blake3, which degraded to portable Rust and
so was correctly dismissed on green). The #2018 diff shows `cdzAgentHostNativeCheck = pkgs.stdenvNoCC
.mkDerivation` with `nativeBuildInputs = [ rustToolchain ]` (no cmake/pkg-config/cc) and buildPhase lines
`cargo clippy --features live-net` (:58) + `--features admin,live-net` (:59). Those two lines compile the
live-net closure → `aws-lc-sys` → cmake invocation → FAILS (no cmake in the sandbox).

This is NOT a "let its own check decide / dismiss on green" like #1999 — aws-lc-sys can't fall back, so the
live-net clippy lines will red. HIGH-confidence pre-merge flag (PR still OPEN → fix before it lands + reds
`nix flake check`). Fix: build this derivation with a C toolchain — `pkgs.stdenv.mkDerivation` (not
`stdenvNoCC`) AND add `pkgs.cmake` + `pkgs.pkg-config` to `nativeBuildInputs` (aws-lc-sys needs all three).
(Alternatively, if the intent is to keep the native check hermetic-no-cc, DROP the `live-net` clippy lines
from it and cover live-net separately where a toolchain exists — but the aws stack genuinely needs the
tools to lint.) v-nix owns flake.nix.
