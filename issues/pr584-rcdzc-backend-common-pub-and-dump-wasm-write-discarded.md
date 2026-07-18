# pr584 — rcdzc nits: `pub mod common` API surface + CDZ_DUMP_TEST_WASM discards write error (2 Copilot)

Mirrored from GitHub PR #584 review comments (Copilot). Both VERIFIED against `git show trunk`.
PR: https://github.com/camshaft/cadenza/pull/584 (10-MR publish batch)

## id 3608237762 (rcdzc/src/backend/mod.rs:19) — pub module widens API surface
> `backend::common` is introduced as a `pub` module, which expands rcdzc's public API surface even
> though it appears to be an internal implementation detail used only within the crate. Consider
> making this `pub(crate)` (or private) to avoid accidentally stabilizing internal analyses for
> external consumers.

VERIFIED: `pub mod common;` at backend/mod.rs:17 (sibling to `pub mod rust; pub mod wasm;`). Minor
API-hygiene nit — `pub(crate)` if `common` is only used within rcdzc. (rcdzc is a compiler crate, not
a published lib, so low stakes; still a reasonable tightening.)

## id 3608237770 (cdz/src/main.rs:3462) — debug dump prints success even if write fails
> The `CDZ_DUMP_TEST_WASM` debug path prints a success message even if `std::fs::write` fails, because
> the write result is discarded. This can mislead debugging (and hide permission/path errors).

VERIFIED: `let _ = std::fs::write(&path, &component);` then unconditionally `eprintln!("[dump] wrote
test component ({} bytes) to {path}", ...)`. The `let _ =` swallows a write error but the message
claims success. It's a throwaway debug env-var path (comment: "Throwaway."), so low stakes, but the
misleading "wrote" on failure is a real (tiny) papercut. Fix = match the write result / print the error.

## Owner
Both in rcdzc / cdz crates (compiler + CLI internals). Minor cleanups — filing to PM to fold into a
sweep (not worth a dedicated fixer). Same class as the low-priority doc/hygiene items the PM has
folded into next-commit before.

---
BACKLOGGED to concierge (corpus-bugfix 2026-07-18): 2 trivial cosmetic nits, both grepped-real. (1) backend/mod.rs:17 pub mod common -> pub(crate). (2) cdz main.rs:3457 CDZ_DUMP_TEST_WASM throwaway debug path discards write err. Fold into a next rcdzc/cdz hygiene sweep or dismiss; not worth a fixer.
