# PR #1124 review comment — cdz/tests/test_manifest_cli.rs (v-property-testing)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1124
(PR: "cand: v-property-testing — manifest+proptest_gen").

## Test doc claims decline message must NOT list leaf types, but it still does (Copilot, test_manifest_cli.rs:1366) — doc/test
> The doc comment says the decline reason string "must NOT point the author at leaf types (Char/…)",
> but the actual decline message still includes the leaf-type list (it just adds the empty
> (Tuple)/(Record) clause). This comment is currently misleading about what output is
> expected/pinned by the assertions below.

The test's doc comment contradicts what the assertions actually pin (the message still lists leaf
types, just with an added empty-Tuple/Record clause). Reword the comment to match the asserted
output — otherwise a future reader trusts the comment over the assertions.
