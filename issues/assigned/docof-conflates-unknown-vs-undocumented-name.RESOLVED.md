> **RESOLVED (sidecar half)** by v-lsp MR c3d0c6a99 — DocOf emits "no such definition X" for an unknown name via name_is_known. cdz-doc exit-code mapping = v-cdz-tooling.

# UX/diagnostic (low pri, v-cdz-tooling): cdz doc / DocOf query conflates UNKNOWN name vs KNOWN-but-undocumented

Design observation (low priority, not a bug — flagging for the query/diagnostics owner). cdz doc <name> can't distinguish an UNKNOWN name from a KNOWN-but-undocumented one: both yield "no documentation for `name`" with exit 0. Repro: a module with (def documented (doc "...")) + (def plain) → `cdz doc plain` and `cdz doc totally_unknown` both print `no documentation for `X`` rc=0. So a user who TYPOS a name gets a misleading "no documentation" rather than "no such definition".

Root: the DocOf sidecar query (rcdzc/src/sidecar.rs:665, doc_of_name → unwrap_or_else "no documentation for X") is INTENTIONALLY TOTAL (comment at :208). That totality is a deliberate contract, so I did NOT change it from the cdz command side — and a cdz-side "is this a known symbol?" pre-check is unreliable because cdz doc also documents BUILT-INS (prelude names / grammar keywords) that aren't in the file's Symbols. So distinguishing unknown-vs-undocumented really wants to happen INSIDE the query (which knows the Db + built-in tables).

Possible fix if you think it's worth it: DocOf returns a distinct "no such definition `X`" (vs "no documentation for `X`") when the name resolves to NO def AND no built-in — and cdz doc maps the former to a non-zero exit. Purely a UX nicety; the totality contract can stay (both are still defined answers). No action expected — filing so it's tracked. cdz doc command layer is mine; the query semantics are yours.

Corpus-bugfix confirmed: DocOf is intentionally TOTAL (sidecar.rs:208-209). The distinguishing logic wants to live INSIDE the query (knows Db + built-in tables). Query/sidecar layer = v-lsp territory.
