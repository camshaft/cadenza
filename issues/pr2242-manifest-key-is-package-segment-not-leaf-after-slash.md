# PR #2242 review — cdz-kernel/src/wasm_host.rs (v-agent-harness) — OPEN — doc-correctness [VERIFIED, LOW-MED]

https://github.com/camshaft/cadenza/pull/2242 (transitive dep-of-dep compose — recurse the runtime's bare
`cadenza:nfc/normalize`; §23, unblocks b1 e2e). Copilot 1 inline (2 sites). This is the §23 store
resolution my HIGH #2210 lives adjacent to.

## the doc says the `runtime.toml` manifest key is the interface "leaf (after the last `/`)" — which for `cadenza:nfc/normalize` is `normalize` — but the code + `ComponentStore` key on the PACKAGE segment (between `:` and `/`), i.e. `nfc`; the doc's own adjacent text says `nfc`, so it's self-contradictory (Copilot, wasm_host.rs:525 & :533) — doc-correctness [VERIFIED, LOW-MED]
> The doc comment incorrectly states that the `runtime.toml` manifest key is the interface "leaf" after
> the last `/` (which would be `normalize`). In this code (and in `ComponentStore`), the manifest key is
> the package segment between `:` and `/` (i.e. `nfc`). This mismatch is confusing and contradicts the
> later inline comment that correctly explains the parsing.

VERIFIED in the #2242 diff — the doc is INTERNALLY CONTRADICTORY. For `NFC_IFACE = "cadenza:nfc/normalize"`
(diff:55): the PACKAGE segment (between `:` and `/`) is `nfc`; the LEAF (after the last `/`) is `normalize`.
The code + comments consistently use `nfc` as the manifest key: "resolved from the store by the manifest
name `nfc`" (diff:53-54), "`runtime.toml`'s `nfc = "<hash>"` → `<hash>.wasm`" (diff:62), "nfc → runtime →
reducer" (diff:40). But the SAME doc block (diff:54) then says "The interface's leaf (after the last `/`)
is the `runtime.toml` manifest key" — "leaf (after the last `/`)" = `normalize`, NOT `nfc`. And diff:62
compounds it: "resolved from the store BY NAME (the interface leaf, e.g. `nfc`)" — mislabels `nfc` as the
"leaf" when `nfc` is the PACKAGE segment. So the doc calls the package-segment-key a "leaf after the last
`/`", which is backwards (the leaf is `normalize`). LOW-MED/doc-correctness — no code bug (the code keys on
`nfc` correctly), but the doc would mislead anyone editing `get_by_manifest_name`'s key derivation into
using `normalize`. Fix per Copilot: correct the doc to say the manifest key is the PACKAGE segment (between
`:` and `/`) — `nfc` — NOT the leaf after the last `/` (`normalize`); fix both the diff:54 parenthetical and
the diff:62 "(the interface leaf, e.g. `nfc`)" mislabel. v-agent-harness owns cdz-kernel/src. PR OPEN →
foldable pre-merge. (§23 transitive compose unblocks b1 e2e — worth the manifest-key terminology being
exactly right since a future manifest-lookup edit keys on it.)
