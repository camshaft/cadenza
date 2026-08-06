# PRs #2439 / #2443 / #2444 (all MERGED) follow-up — v-agent-harness late Copilot nits [VERIFIED]

Late Copilot inline on 3 merged v-agent-harness PRs (trunk 0e12d9bac). All follow-up, owner discretion. Verified vs trunk.

## #2439 c2 — comment claims KV records member_count + hex members, but code only puts member_count (Copilot, kernel.rs:4114) — doc-vs-code [VERIFIED, LOW]
VERIFIED (kernel.rs:4113-4121): comment "record the decoded membership count + the hex members (sorted) into the KV" —
but the block only does `kv.put(b"member_count", vec![members.len() as u8])`; no `members` are put. Fix: either record
the members too, or drop "+ the hex members (sorted)" from the comment to match what the reducer actually stores.

## #2439 c1 — test comment says byte-stability is order-independent but encode_members preserves input order (Copilot, event_ast.rs:1661) — doc-vs-code [VERIFIED-CLAIM, LOW]
> byte stability comes from providing canonical ascending-hash order (e.g. via BTreeSet), not from encode_members
> itself (which preserves the given order).
Relay the semantic point: if encode_members preserves caller order, then byte-stability is a CALLER contract (feed
canonical order), not a property of encode_members — the test comment should say so rather than implying the encoder
canonicalizes. (v-ah to confirm whether encode_members sorts internally; if it does, Copilot is wrong — verify.)

## #2443 c1 — new (descendant-of <hex>) predicate not reflected in encode_capability_manifest docs / "five arms" test comment (Copilot, event_ast.rs:142) — doc-hygiene [LOW]
The I6 descendant-of predicate form was added to the manifest wire shape, but nearby doc/test comments still enumerate
the older predicate set ("five arms"). Update the enumerations to include descendant-of so docs match the codec vocab.

## #2443 c2 — controller.to_hex() then str_leaf re-clones via to_string() = double alloc (Copilot, event_ast.rs:145) — efficiency [VERIFIED-STRUCTURE, LOW]
VERIFIED (event_ast.rs:145): `let v = str_leaf(&mut b, &controller.to_hex())` — to_hex() allocates a String, passed as
&str, and (per Copilot) str_leaf clones it again via to_string(). If str_leaf can take an owned String (or Leaf::Str can
be built directly from the hex String), the second alloc is avoidable. LOW — one alloc per manifest encode. Confirm
str_leaf's signature before acting (Copilot's "immediately clones via to_string()" is the checkable claim).

## #2444 c1 — group_multicast_e2e module doc overstates the test (implies full member fold, but only asserts routing) (Copilot, group_multicast_e2e.rs:4, also 104) — doc-vs-test [LOW]
Copilot: the doc says each member folds the message end-to-end, but the test only drives the controller and asserts
EmitExecutor routes one Inbound per member into the shared inbox — it never delivers those inbounds into the member
sessions or checks their KV. Reword the doc to "asserts routing/fan-out" (matches what it tests), or extend the test to
drive members. Secondary site line 104 — verify per-site (same doc-block, likely same reword).

All merged → follow-up at v-agent-harness discretion; all LOW (4 doc-accuracy + 1 double-alloc). No pre-merge urgency.
