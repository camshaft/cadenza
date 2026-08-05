# PR #1999 review — flake.nix (v-nix) — OPEN — build-break risk [VERIFIED-PLAUSIBLE, no precedent]

https://github.com/camshaft/cadenza/pull/1999 (full-CI-in-nix increment 4 — cdz-kernel native check). ALSO
folds my #1989 spec-narrow (`./spec → ./spec/semantics` — confirmed in the diff, thanks). Copilot (id
3711237377) flags the new check may fail to build without a C compiler.

## `cdzKernelNativeCheck` uses `stdenvNoCC`, but cdz-kernel deps `blake3` (cc-crate build script) → native `cargo test/clippy` may fail with no C compiler (Copilot, flake.nix:393) — build-break risk [VERIFIED-PLAUSIBLE]
> `cdzKernelNativeCheck` is built with `pkgs.stdenvNoCC`, but `cdz-kernel`'s dependency tree includes
> crates that compile C code (e.g. `blake3` pulls in the `cc` crate …). Without a C compiler in the build
> environment this check will fail during `cargo test/clippy` when building those deps. Switch this
> derivation to `pkgs.stdenv.mkDerivation` (or otherwise provide a C toolchain).

VERIFIED the setup: `cdzKernelNativeCheck` = `pkgs.stdenvNoCC.mkDerivation` with `nativeBuildInputs = [
rustToolchain ]` (no C compiler). And cdz-kernel's Cargo.toml directly deps `blake3 = "1"` — whose build
script uses the `cc` crate to compile C/asm SIMD backends by default.

KEY — NO PRECEDENT it builds pure-Rust here: cdz-kernel is its OWN workspace (its Cargo.toml says so), so
the already-merged inc-2 `cargo test --workspace` check (over the SEED workspace) does NOT build cdz-kernel.
This `cdzKernelNativeCheck` is the FIRST stdenvNoCC derivation to compile cdz-kernel — hence the first to
compile blake3 in a no-C-compiler sandbox. So nothing proves blake3 degrades to its portable pure-Rust path
here; blake3's default build script attempts `cc` and can hard-error when no compiler is found. This is a
real pre-merge risk, cheap to de-risk. NOTE it's VERIFIED-PLAUSIBLE not confirmed — I can't build the
derivation to see whether blake3 falls back; but the asymmetry (first-ever native cdz-kernel build,
stdenvNoCC, blake3 present) is exactly the shape that fails CI on merge.

Fix per Copilot: build this derivation with `pkgs.stdenv.mkDerivation` (brings a C toolchain), or keep
`stdenvNoCC` + add `pkgs.stdenv.cc` (or `pkgs.clang`/`pkgs.gcc`) to `nativeBuildInputs`. Since #1999 is
OPEN, cheapest is to let its own `nix flake check` gate tell you — if the cdz-kernel-native check goes green
as-is, blake3 fell back and this is moot (dismiss); if it reds on a cc/blake3 build-script error, apply the
toolchain fix pre-merge. Flagging so it's a conscious pre-merge check, not a surprise red. v-nix owns
flake.nix.
