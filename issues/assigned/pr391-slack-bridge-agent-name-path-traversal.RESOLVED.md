# PR review comment — mirrored from GitHub PR #391 (Copilot inline) — SECURITY

- **PR:** #391 "fleet: seventeenth batch (cdz LSP server, map-match no-fold fix, guide pages-unblock, trap-observation)" (OPEN at time of triage; files already on trunk)
- **File:** `fleet/slack-bridge/inbox.js:30` (`inboxDir` / `deliver`)
- **Reviewer:** Copilot (automated)
- **Comment id:** 3590196720
- **Link:** https://github.com/camshaft/cadenza/pull/391#discussion_r3590196720

## Comment (verbatim)
> `inboxDir` blindly `path.join`s the Slack-provided agent name into the filesystem path. Because `agent` can be `..` (and `format.js` currently allows `.`), a Slack message like `@.. hi` can write outside `<fleetDir>/inbox/…`. This should validate/sandbox the agent name before building paths.

## Liaison triage — CONFIRMED against trunk — PATH TRAVERSAL
Confirmed on trunk:
- `fleet/slack-bridge/inbox.js`: `inboxDir(fleetDir, agent) => path.join(fleetDir, "inbox", agent)` with
  NO validation, and `deliver()` calls `fs.mkdirSync(dir, {recursive:true})` + writes a file there.
- `fleet/slack-bridge/format.js:45`: the retarget regex is `/^@([A-Za-z0-9._-]+)\s*(.*)$/s`, which ALLOWS
  `.` and `-` — so `@..` parses as agent `".."`, and `path.join(fleetDir,"inbox","..")` resolves to
  `fleetDir` itself (and `@../../x` escapes further). A Slack sender can thus create dirs / write JSON
  files OUTSIDE the inbox tree.
This is a real path-traversal write primitive reachable from Slack input — the highest-severity finding
in this sweep. Fleet-tooling territory (`v-fleet-tooling` owns slack-bridge). FIX: validate the agent
name against the known roster (or a strict `^[A-Za-z0-9][A-Za-z0-9-]*$` with no `.`), rejecting `..`
and any name that isn't a registered agent, BEFORE building the path. Fix on `trunk`. Quote + link in
queue file.

## RESOLUTION (v-slack-bridge, commit 6078404b — mr sent to pr-sync)
FIXED both the Node bridge (on trunk) and its Rust port. Defense in depth:
- SINK: inbox_dir validates a strict slug `^[A-Za-z0-9][A-Za-z0-9-]*$` (leading alphanumeric,
  then alphanumerics/hyphens; NO dots/slashes/separators → cannot be `..` or hold a dir boundary).
  Every roster agent name already matches. JS: inboxDir throws (bridge.js already try/catches →
  warns, no crash). Rust: inbox_dir -> Option (None → deliver errors InvalidInput; drain empty).
- PARSE: the `@agent` retarget regex tightened to the same slug, so `@..`/`@../x` never becomes
  the recipient (falls through to the default). Dots dropped (no roster name uses them).
Regression tests pin it: Rust 32 pass (+7 security), Node smoke 21 pass (+5). clippy -D + fmt clean.
