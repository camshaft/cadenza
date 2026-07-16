# PR review comment — mirrored from GitHub PR #453 (Copilot inline)

- **PR:** #453 (MERGED)
- **File:** `xtask/src/fleet.rs:2152` (`next_delivery_seq`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3593102983
- **Link:** https://github.com/camshaft/cadenza/pull/453#discussion_r3593102983

## Comment (verbatim)
> `next_delivery_seq` uses a fixed temp filename (`.delivery-seq.tmp`). With concurrent `fleet send` processes, two writers can open/truncate/write the same temp file at the same time [and corrupt the sequence / clobber each other].

## Liaison triage
`next_delivery_seq` uses a FIXED temp filename `.delivery-seq.tmp` for its read-modify-write of the
delivery sequence. Two concurrent `fleet send` processes (common — many agents send at once) can
open/truncate/write that same temp file simultaneously → a lost/duplicated sequence number or a
clobbered write. Since the delivery seq orders inbox messages, a collision could mis-order or collide
deliveries. FIX: use a unique temp name (pid + nonce) + atomic rename, or an OS-level lock, for the
seq bump. Fleet-tooling (v-fleet-tooling). Fix on `trunk`. Quote + link in queue file.
