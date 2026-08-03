# PR #1537 review comments — implementation/design/DESIGN-host-capability-discovery.md (design-host-capabilities)

Mirrored from https://github.com/camshaft/cadenza/pull/1537 (PR: "[design-host-capabilities] 99f95b1ba").

## 1. Stray unmatched `</content>` tag at EOF (Copilot, DESIGN-host-capability-discovery.md:351) — doc
> The file ends with a stray `</content>` tag, but there is no matching opening `<content>` tag. This
> isn't valid Markdown and will render as an orphaned literal line.

Remove the stray `</content>` line at EOF (looks like an accidental paste of a tool/wrapper tag).

## 2. "Memory:" references don't correspond to repo files/URLs (Copilot, DESIGN-host-capability-discovery.md:350) — doc
> The references under "Memory:" (e.g. `agent-harness-v2-kernel-design-and-v0-plan`) don't appear to
> correspond to any file/URL in the repo, so future readers won't be able to follow them. Consider
> replacing them with actual repository paths/links (or removing the "Memory:" note).

Those "Memory:" entries are shared-memory wikilink slugs (from the fleet's private memory, not the
repo) — a repo reader can't follow them. Either replace with real repo paths/links or drop the
"Memory:" note from the committed design doc.
