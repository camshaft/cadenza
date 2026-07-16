# PR review comments — mirrored from GitHub PRs #419/#420 (Copilot inline) — cdz-tooling cluster

- **PRs:** #419, #420 (MERGED)
- **Reviewer:** Copilot (automated)

## 1. `cdz/src/lsp.rs:1179` `path_to_uri` under-encodes (#419, id 3591806921)
> `path_to_uri` only percent-encodes spaces. Unescaped `%` (and other reserved characters like `#` / `?`) can make the constructed `file://...` URI invalid or change its meaning.

CONFIRMED: `let encoded = path.replace(' ', "%20");` — only spaces. A path with `%`/`#`/`?` yields an
invalid/misinterpreted `file://` URI. FIX: percent-encode the reserved set (or use a proper URI encoder).

## 2. `cdz/src/main.rs:685` `cdz new` injects proj_name into a string literal (#420, id 3591841677)
> `proj_name` is interpolated directly into a quoted string literal in `Project.cdz`. If the directory name contains `"`, `\`, or control characters … [malformed manifest].

CONFIRMED: `let manifest_src = format!("def name = \"{proj_name}\"\ndef entry = \"{entry_file}\"\n");`
— raw interpolation of the dir name into a quoted string. A dir name with `"`/`\`/control chars produces
a malformed `Project.cdz`. FIX: escape the name (or reject invalid project names up front).

## 3. `cdz/src/main.rs:658` `cdz new` non-empty-target guard (#420, id 3591841665)
> The non-empty-target guard treats any `read_dir` error (including when the path exists but is a file) as "not empty", yielding a misleading error message. Explicitly reject the file case.

CONFIRMED plausible: a `read_dir` error is conflated with "directory not empty". FIX: distinguish
"target exists as a file" from "directory non-empty" for a clearer error.

## Liaison triage
All three are cdz-CLI tooling correctness/robustness in the `cdz` crate → v-cdz-tooling. Low-to-medium
severity. Fixes on `trunk`. Links: #419#discussion_r3591806921, #420#discussion_r3591841677,
#420#discussion_r3591841665.
