# PR #2200 review — cdz-agent-host/src/factory.rs (v-agent-harness-host) — OPEN — 1 doc/atomicity (LOW-MED) + 1 test-precision (LOW) [VERIFIED]

https://github.com/camshaft/cadenza/pull/2200 (run_genesis_ceremony — one-call genesis boot: seed setup
events + install authorizer). Copilot 2 inline.

## `run_genesis_ceremony` doc claims an error leaves the session "un-booted rather than half-installed", but it seeds KV BEFORE attempting install → an install failure leaves the session PARTIALLY SEEDED, contradicting the doc; caller should be told to discard it (+ a zero-width char in the doc) (Copilot, factory.rs:379) — doc/atomicity [VERIFIED, LOW-MED]
> The `run_genesis_ceremony` docs claim an error leaves the session "un-booted rather than
> half-installed", but the implementation (and the test below) seeds KV before attempting installation.
> If `install_genesis_authorizer` fails, the session is already partially seeded, so the docs should
> describe that and advise callers to discard the session. Also, there is an invisible/zero-width
> character between `absent/` and `non-lifting`…

VERIFIED in the #2200 diff: the doc says "`Err` propagates the first failure … leaving the session
un-booted rather than half-installed" (diff:19-20). But the impl (diff:34-37) is
`session.seed_genesis(root_identity, authorizer_hash, context).await.map_err(…)?;
self.install_genesis_authorizer(session, principal).await` — it SEEDS first, then installs. So if
`install_genesis_authorizer` fails AFTER `seed_genesis` succeeded, KV is ALREADY seeded → the session is
half-booted, NOT "un-booted". The PR's OWN test comment confirms this: "seed succeeds, then the install
step [fails] … The seed step still ran before the install failed (root + the recorded hash are in KV)"
(diff:69, 89). So the doc's atomicity claim is FALSE — there's no rollback of the seed on install failure.
LOW-MED (a caller trusting "un-booted rather than half-installed" would NOT discard a partially-seeded
session, then reuse a KV that has genesis root/hash but no installed authorizer — a half-booted state).
Fix per Copilot: reword the doc to state that seed happens first and is NOT rolled back on an install
failure — the session may be partially seeded, so callers should DISCARD it on `Err` (don't reuse). ALSO:
remove the zero-width char between `absent/` and `non-lifting` (diff:19) — it breaks copy/paste + search.

## the test comment says the seed step recorded BOTH root identity AND authorizer hash, but the assertion only checks root identity (Copilot, factory.rs:983) — test-precision [VERIFIED, LOW]
> This test comment says the seed step recorded both the root identity and the authorizer hash, but the
> assertion only checks the root identity. Adding an assertion for `KV_AUTHORIZER_HASH` would make the
> test match its stated intent…
VERIFIED: the test comment (diff:89) says "root + the recorded hash are in KV", but the assert (diff:62)
only checks `session.kv().get(genesis_ct::KV_ROOT_IDENTITY)`. LOW/test-precision — add a
`KV_AUTHORIZER_HASH` assertion so the test actually proves BOTH were seeded before the install failed
(which is the point — that seed ran fully before install). This ties to c1: the test that DEMONSTRATES the
half-seeded state should assert the full seed, making the doc-vs-behavior gap explicit. v-agent-harness-host
owns cdz-agent-host. PR OPEN → both foldable pre-merge. The doc/atomicity one matters most (it's a
misleading failure-mode guarantee on the genesis boot path).
