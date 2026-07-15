# PR review comment — mirrored from GitHub PR #398 (Copilot inline)

- **PR:** #398 (MERGED)
- **File:** `spec/capabilities/modules-and-namespaces.md:36` (+ duvet citation in `implementation/seed/crates/rcdzc/src/resolve.rs`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3590785554
- **Link:** https://github.com/camshaft/cadenza/pull/398#discussion_r3590785554

## Comment (verbatim)
> This requirement combines two independent RFC-2119 obligations into a single sentence ("…MUST govern…" and "a definition MUST remain visible…"). That conflicts with the spec's own stated convention that each requirement is "a single self-contained sentence carrying exactly one obligation" (see the file preamble) and with the constitution's requirement-extraction model. Split this into two separate normative sentences under the same heading, and then update the corresponding duvet citation in `implementation/seed/crates/rcdzc/src/resolve.rs` to cite the new sentence(s).

## Liaison triage — CONFIRMED against trunk
Confirmed: modules-and-namespaces.md:36 is one sentence carrying TWO obligations — "The explicit
visibility rule MUST govern only a definition's reachability from outside its module; a definition MUST
remain visible to the other definitions in its own module…". This violates the file's one-obligation-
per-sentence convention (same class as my pr385 module-record compound-sentence split, which
v-duvet-coverage resolved). Split into two atomic RFC-2119 sentences under the same heading and update
the duvet citation in resolve.rs to cite each. Squarely v-duvet-coverage territory (spec-sentence
atomicity + citation agreement). Fix on `trunk`. Quote + link in queue file.
