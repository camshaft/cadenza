# PR #1582 review comments — fleet/NIX-FLAKE-PIPELINE-SCOPING.md (v-nix)

Mirrored from https://github.com/camshaft/cadenza/pull/1582 (PR: "docs(nix): correct the scoping doc —
operator redesign (hash-from-built-bytes) supersedes the N0–N4 plan"). Three LOW doc nits, all verified.

## 1. "since SUPERSEDED" reads as a causal clause (Copilot, :5) — doc/grammar
> The sentence "the N0–N4 plan below is the ORIGINAL scoping, since SUPERSEDED …" is grammatically
> incorrect/unclear; "since" reads like a causal clause rather than "now superseded".

VERIFIED (:14): "the N0–N4 plan below is the ORIGINAL scoping, since SUPERSEDED by an operator
redesign". Reword to "…the ORIGINAL scoping, now superseded by an operator redesign". LOW.

## 2. "CURRENT STATE" introduces R2/R3/R4 without defining the "R" scheme vs N0–N4 (Copilot, :14) — doc/clarity
> This "CURRENT STATE" summary introduces R2/R3/R4 without defining what the "R" staging scheme means
> (the document otherwise uses N0–N4). Adding a short inline definition at first use would prevent
> reader confusion.

VERIFIED: the CURRENT STATE block jumps to R2/R3/R4 while the rest of the doc is N0–N4. Add a one-line
"(R = the redesign's staging, replacing N0–N4)" at first use. LOW.

## 3. "nix" inconsistently lowercased vs "Nix" (Copilot, :9) — doc/consistency
> "nix" is inconsistently lowercased here, while the rest of the document uses the proper noun "Nix"
> (including "Nix store").

VERIFIED (:18): "all in a **nix store**" lowercase, vs "Nix" elsewhere. Capitalize the proper noun
("Nix store"). LOWEST.
