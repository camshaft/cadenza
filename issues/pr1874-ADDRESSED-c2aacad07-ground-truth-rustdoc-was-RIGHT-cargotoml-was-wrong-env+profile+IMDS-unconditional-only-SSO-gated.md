# PR #1874 review comments — cdz-agent-host/src/{model,host}.rs (v-agent-harness-host) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1874 (MERGED). The creds-chain doc over-swung — now claims
profile/IMDS ARE available, contradicting the Cargo.toml feature set. (#1863/#1841 lineage, opposite direction.)

## Doc claims profile/SSO/IMDS creds are in the default chain "not feature-gated", but the crate does NOT enable those features (Copilot, model.rs:132 + host.rs:328) — doc/accuracy [VERIFIED]
> The doc claims the SDK's default credential chain (shared config/credentials profiles + IMDS) is
> available and "not feature-gated", but the crate builds aws-config with default-features=false + only
> behavior-version-latest/rustls/default-https-client/rt-tokio (Cargo.toml). Inaccurate — misleads
> operators about which credential sources actually work in live-net.
VERIFIED against the #1874 branch's Cargo.toml (:72-76): it EXPLICITLY says "We do NOT enable the
profile/SSO/IMDS credential-source features (sso, credentials-process) — env-var credentials [are the
supported source]", and the aws-config feature list is only [behavior-version-latest, rustls,
default-https-client, rt-tokio]. But model.rs:132 + host.rs:328 now claim profile/IMDS ARE in the default
chain "not feature-gated" — directly contradicting the crate's OWN Cargo.toml comment. This is doc-drift
that over-swung: #1863 (my note) said "profile/IMDS not enabled", #1841 corrected, #1874 now over-claims
the other way. Reword to match Cargo.toml: env-var creds are the supported/compiled source; profile/SSO/
IMDS require adding those features. LOW-MED/doc (misleading deployment guidance on a security-relevant
creds path). Fix-forward. (2 sites.)
