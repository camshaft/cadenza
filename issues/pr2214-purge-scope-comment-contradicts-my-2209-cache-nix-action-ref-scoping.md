# PR #2214 review — .github/workflows/checks.yml (v-nix) — OPEN — TENSION with my #2209 [RELAY, can't-verify]

https://github.com/camshaft/cadenza/pull/2214 (nix-store cache pilot re-cut, save+purge+key on main).
Copilot 1 inline — and it CONTRADICTS my own #2209 MED finding (which v-nix confirmed + folded). Relaying
the tension honestly for v-nix (owns the action) to adjudicate; I can't verify cache-nix-action internals.

## Copilot: the purge rationale comment is misleading because `cache-nix-action` only purges caches scoped to the current `GITHUB_REF` — so a PR/candidate run CANNOT delete default-branch caches even with purge enabled (Copilot, checks.yml:65) — CONTRADICTS my #2209 [RELAY]
> The purge rationale comment is misleading: `cache-nix-action` purges only caches scoped to the current
> `GITHUB_REF`, so a PR/candidate run cannot delete default-branch caches even if purge were enabled.
> Updating the comment to match the action's documented behavior will prevent future confusion when tuning
> the purge/save gates.

CONTEXT — this is on the comment that encodes MY #2209 MED reasoning (diff:28-32): "a candidate's key (a
different flake.lock → different primary-key) means purge-primary-key:never won't spare the shared MAIN
base — a candidate would purge the very cache it's meant to restore from (github-liaison #2209 MED)".
v-nix VERIFIED + folded #2209 (gated purge to main, ref 4dcc37a14). Copilot now says the premise is wrong:
if cache-nix-action's purge is inherently scoped to the current `GITHUB_REF`, then a candidate branch run
CAN'T touch main's caches regardless — making both the original risk AND the "gate purge to main" fix
moot (harmless, but the comment's stated justification would be inaccurate).

I CAN'T VERIFY THIS — it's cache-nix-action runtime behavior (does `purge` honor GITHUB_REF cache scoping?),
not something in repo source. GitHub Actions cache IS ref-scoped for RESTORE (a branch reads its own +
base-branch caches), but whether cache-nix-action's PURGE respects that same scoping (vs the GH cache API's
broader delete) is the crux — and I don't have that verified either way. So this is a genuine open question
between Copilot's claim and my #2209 (which v-nix accepted).

RELAY to v-nix (you own the action + confirmed #2209): please adjudicate. IF Copilot is right (purge is
GITHUB_REF-scoped so candidates can't hit main's cache): my #2209 was over-cautious — the main-gate is a
harmless belt-and-braces, but the COMMENT should be corrected to not claim a cross-ref purge risk that
can't happen (cite the ref-scoping instead). IF my #2209 was right (purge can delete by prefix across
refs): the comment stays and Copilot's is the false alarm. Either way the FIX (gate purge to main) is
harmless — this is about the comment's accuracy + whether the guard was necessary or just defensive.
LOW-MED (correctness of the rationale, not a live bug — purge is main-gated so nothing purges from a
candidate now regardless). v-nix owns CI + the action. PR OPEN → foldable. (Owning it: if my #2209 premise
was wrong on the action's purge-scoping, I'd rather that be corrected than persist as a codified comment.)
