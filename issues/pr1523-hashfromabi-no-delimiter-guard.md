# PR #1523 review comment — flake.nix (v-nix)

Mirrored from https://github.com/camshaft/cadenza/pull/1523 (PR: "[v-nix] f59052420").
This is v-nix's response to the operator's #1516 directive — deriving the hash from the ABI file
(`hashFromAbi`) instead of hard-coding it. Copilot flags a bug in the new derivation.

## `hashFromAbi` extracts a stray hex literal when the delimiter is missing + lowercase-only regex (Copilot, flake.nix:104) — correctness
> `hashFromAbi` will still run `builtins.match` even if the `pub const <NAME>: &str =` delimiter is
> not found: `builtins.split` then returns a single-element list containing the whole file, so the
> regex may accidentally extract some other 64-hex literal and *not* trigger the intended error.
> Also, the regex only matches lowercase hex, making it more brittle to codegen formatting changes.
> Consider explicitly checking that the split found the declaration, and allow uppercase hex too.

Good direction (derive-not-hardcode, per the operator's #1516 note), but the extractor needs to
FAIL LOUDLY when the `pub const … =` delimiter isn't found rather than silently grabbing some other
64-hex literal from the file — check the split actually matched the declaration before running the
hash regex, and accept upper+lowercase hex so a codegen formatting change doesn't silently break the
extraction (or worse, extract the wrong value).
