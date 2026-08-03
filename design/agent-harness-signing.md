# Agent Harness — Global-Store Signing: Proposal & Tradeoffs

> **Status:** proposal for operator decision. Nothing here is built yet. Written to be readable
> WITHOUT the kernel design in your head — it explains the problem before the solution, and ends with
> the specific choices you're being asked to approve. Companion to §4c ("mutable-name authority &
> anti-hijack") of `agent-harness-kernel.md`.

## TL;DR — the decision in one paragraph

The global store lets agents publish **named pointers** like `system/compiler/latest → <hash>`. Whoever
can write that name controls what every agent picks up as "the compiler." To stop a rogue agent from
repointing it at malicious code, each write must be a **signed event** we can attribute and reject if the
signer lacks authority. Signing needs (a) a signature scheme, (b) a root key someone holds, and (c) two
small fields on every event. **This doc asks you to approve a scheme (recommend: ed25519), where the root
key lives (recommend: a file on the hub node for P0), and adding the two envelope fields now.** Session
delegation (agents getting their own scoped keys) can come in a later slice — flagged below.

---

## 1. What is being signed, and why — the threat in plain terms

**The setup.** Two kinds of thing live in the store:
- **Immutable blobs**, addressed by their content hash. The hash IS the proof: you cannot forge bytes
  that hash to a name you didn't produce. These need **no signing** — the address self-verifies.
- **Mutable names**, like `system/compiler/latest`, that POINT at a hash and can be re-pointed over time.
  This is the *only* thing that needs write control — and the entire attack surface.

**The attack (why this matters).** Suppose any agent could write `system/compiler/latest`. A compromised
or buggy agent repoints it at an *evil* compiler build. Every agent that later resolves "the latest
compiler" now fetches and runs the attacker's code — a **supply-chain hijack** that spreads on its own.
The same shape applies to `memory/*` (a poisoned shared memory) and any shared authoritative pointer.

**What signing buys.** We make a mutable name an **append-only log of signed `set(name, hash)` events**;
the current value is the latest *authorized* set. Because each set is signed:
- **Attribution** — we know exactly who set what, when (who-did-it is not guessable, it's cryptographic).
- **Rejection** — the store checks the signer's authority against the name's namespace (only a
  system-release key may set `system/*`); an unauthorized set is *refused at write time*, so injection
  fails rather than silently taking effect.
- **Rollback + forensics** — the history is a signed audit trail; a bad set is attributable and revocable.

**What breaks WITHOUT it.** Any process that can reach the store can silently repoint any name; there is no
way to tell a legitimate release from an injection, no attribution, and a hijack is *silent and total*
instead of *loud and bounded*. (Note: the namespace-authority *parse* — which prefix is governed by whom —
is being modeled as a Cedar resource + prefix grant, already decided separately. Signing is the other
half: proving the writer actually holds that authority.)

**Scope note.** Session-LOCAL state (a session's own KV/log) needs no signing — its sole writer is its own
reducer. Signing exists ONLY for the shared, mutable, global-name layer.

---

## 2. Signature scheme — options & tradeoffs

| Scheme | Signature size | Speed | Wasm/Rust ecosystem | Dep weight | Notes |
|---|---|---|---|---|---|
| **ed25519** (recommend) | 64 bytes | very fast verify/sign | excellent — `ed25519-dalek` is std in the ecosystem, pure-Rust, wasm-friendly | small | modern default; deterministic signatures; what "signed events" in the design implies |
| ECDSA (P-256) | ~64–72 bytes | fast | good, but more footguns (nonce handling) | small–med | needed only if an external system mandates it |
| RSA | 256+ bytes | slow sign, big keys | fine but heavy | heavier | no upside here; larger everything |
| HMAC (shared secret) | 32 bytes | fastest | trivial | none | **rejected**: symmetric — every verifier can also FORGE; no attribution across parties. Wrong trust model for multi-writer authority. |

**Recommendation: ed25519.** Smallest modern asymmetric scheme, fast, deterministic, pure-Rust +
wasm-friendly (`ed25519-dalek`), and asymmetric so a verifier can check a signature without being able to
forge one (the property HMAC lacks and the whole authority model needs). The 64-byte signature on each
event is negligible next to the payload hash it protects.

**Decision asked:** approve ed25519, or name a scheme you require for external-compatibility reasons.

---

## 3. Key management / root of trust — where the root key lives

Signing needs a **root keypair** for the single operator (you) that anchors `system/*` authority. Three
provisioning options, by environment trust:

| Option | How it works | Pros | Cons |
|---|---|---|---|
| **(A) Hub-node file** (recommend for P0) | the kernel loads a root keypair from a configured path/secret at boot | real crypto, dead-simple, no new infra; key provisioning is ops-config not code | the key sits on the hub host (acceptable for single-operator P0; harden later) |
| (B) IMDS / Bedrock-broker | reuse the existing "prove environment identity → scoped credential" primitive to mint/hold signing authority | no standing key file; leans on already-built cred-broker | more moving parts; overkill for P0 single-operator |
| (C) Dev key checked into the repo | a fixed keypair in-tree | zero setup for local dev | **NOT for anything real** — a checked-in private key is public; dev/test only |

**Recommendation: (A) hub-node file for P0**, with (B) as the natural upgrade when multi-node / multi-
operator arrives (it federates the same way node identity does). **Rotation:** because a name is an
append-only signed log, rotating the root key is itself a signed event (old key signs "new key is now
authoritative for this namespace"); no destructive re-keying. P0 can defer rotation tooling — the log
shape makes it additive later.

**Decision asked:** approve (A) hub-file for P0 (recommend), or pick (B)/(C). And: is deferring rotation
tooling to post-P0 acceptable?

---

## 4. Event envelope change — adding `producer` + `signature`

Today a kernel event is `{seq, cause, body}`. The design (§10) always intended `producer-identity` +
`signature` "from day one, optional/unverified in P0" — but they were never added. Signing needs them.

**The change:** add two fields to the event envelope — `producer` (who emitted it, an identity/public-key
reference) and `signature` (over the event's content + cause + producer). 

- **Cost:** small — two fields; the codec gains two entries; existing events get `None`/empty in P0.
- **Compatibility:** additive. Tolerant readers (the design's posture: ignore-unknown, default-missing)
  handle old events without them. We control both the kernel and its only consumer today, so it's a clean
  additive migration, not a wire break.
- **Reversibility:** the *fields* are cheap to add and hard to retrofit later (§10's "day one" point —
  adding provenance to a log format after it has durable history is painful). So adding the fields now is
  low-risk and future-proofing; making them *verified/required* is the separable, later step. Recommend:
  **add the fields now (P0: present but unverified for most events; verified for `set(name,hash)`), gate
  enforcement per event-type.**

**Decision asked:** approve adding `producer` + `signature` to the envelope now (unverified except on
authority writes in P0).

---

## 5. Session delegated identities — the attenuating chain

The full §4c vision: a session signs with its OWN short-lived identity, chained to its spawner's grant,
and **can never grant a child more than it holds** — this is what stops privilege-escalation-by-spawning
in the multi-operator future.

- **P0-now option:** the root key signs authority writes directly; sessions don't yet have their own keys.
  Simplest; sufficient while there's one operator and few authoritative writes.
- **Next-slice option:** mint per-session delegated keypairs chained to the root. More machinery
  (delegation issuance, chain verification) — real value only once multiple sessions/operators write
  authoritative names.

**Recommendation: P0 = root signs; session delegation is the very next slice** (the envelope + scheme land
now, so delegation drops in without reworking them). **Tradeoff:** deferring delegation means, in P0, the
few authoritative writes go through the root identity rather than a per-session one — fine at single-
operator scale, revisited before multi-operator.

**Decision asked:** approve "P0 root-signs, delegation next slice" — or do you want delegated identities in
the first cut?

---

## 6. Recommendation & the decisions you're being asked to make

**Overall recommendation** (the "real thing, minimally"): real signatures, real prefix authority, real
append-only signed log — with delegation sequenced right after, not stubbed:

1. **Scheme:** ed25519.
2. **Root key:** a keypair loaded from a configured file on the hub node (P0); IMDS/broker later.
3. **Envelope:** add `producer` + `signature` now; verify them on `set(name,hash)` authority writes in P0,
   leave other events unverified for now (gate enforcement per event-type).
4. **Delegation:** P0 root-signs; per-session delegated identities are the next slice.
5. **Rotation:** deferred to post-P0 (the signed-log shape makes it additive).

**Each is independently approvable** — you can accept the scheme + envelope but ask for delegation-now, or
pick a different key home, etc. If any option here is unclear, that's the thing to push back on; the point
of this doc is that you approve/reject each choice understanding what it buys and costs — not that you
rubber-stamp a bundle.

**What proceeds regardless (already unblocked):** the NON-crypto authority half — modeling the namespace as
a Cedar resource + prefix grant, and the mutable-name set/resolve store — is being built now; it does not
depend on this decision. Only the signature bytes on `set` events wait on your approval here.
