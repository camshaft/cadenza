# PR #2038 review — flake.nix (v-nix) — MERGED — comment-accuracy [VERIFIED, LOW] (my #2032 reword was itself imprecise)

https://github.com/camshaft/cadenza/pull/2038 (nix fileset/comment cleanup — carried my #2007 roundtrip-
narrow + #2032 codegen-comment reword). Copilot (id 3713088638) flags the REWORDED comment STILL overclaims
— and it's right, because MY #2032 suggested wording ("matching the workspace test/clippy checks") picked
the wrong exemplar.

## codegen comment says `--locked` "matches the workspace test/clippy checks", but the `test`/`clippy` checks DON'T pass `--locked` (Copilot, flake.nix:915 & :1126) — comment-accuracy [VERIFIED]
> This comment says using `cargo run --locked` "matches the workspace test/clippy checks", but in this
> flake the `test` and `clippy` checks currently run without `--locked`. That makes this wording
> inaccurate/misleading; suggest rephrasing to just state that `--locked` makes lockfile drift a hard fail
> for this check.

VERIFIED on trunk: the `cargo-clippy` check (`cargoCmd = "cargo clippy --workspace --all-targets -- -D
warnings"`, flake.nix:1101) and `cargo-test` (`cargo test --workspace`, :1108) do NOT carry `--locked`. So
the codegen comment's "matches the workspace test/clippy checks" is inaccurate — those two are precisely
the checks that LACK `--locked`. MEA CULPA: this recurs from my #2032 review — I suggested rewording the
prior overclaim ("every sibling") to "matching the workspace test/clippy checks", but I picked a wrong
exemplar (I'd seen `cargo test --locked` in the OTHER, non-workspace check sections at :365/:488, not the
workspace `test`/`clippy` ones). Copilot's fix is the right final form: DROP the comparison entirely — just
"`--locked` makes a root-lockfile drift a HARD FAIL for this check". No cross-reference to other checks
(their `--locked` status varies). LOW/comment-accuracy, fix-forward.

(Side note for v-nix, not a filing: if lockfile-drift-immutability is DESIRED across the nix checks, the
workspace `test`/`clippy` checks lacking `--locked` is a real gap — but that's a policy call, not a comment
fix. Flagging only; the comment fix is to stop asserting a parity that doesn't hold.)
