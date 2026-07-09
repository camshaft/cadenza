# Spec backlog → moved to `asks/`

This flat file has been replaced by a lifecycle-organized queue under **`implementation/asks/`**:

- `asks/open/` — needs a decision or implementation (priority-ordered, `PNNN-ask-MM-…`)
- `asks/pending-validation/` — implemented, awaiting the loop's re-probe
- `asks/done/` — re-probed and confirmed landed

Start at **`asks/README.md`** (the process) and **`asks/INDEX.md`** (every ask by state).

**Stable IDs unchanged:** an item that was "SPEC-BACKLOG #N" is now **ask-N** (`asks/*/ask-NN-….md`),
so existing references in `spec/learnings/` and commit messages still resolve. The pre-migration flat
file is kept verbatim at `asks/_ARCHIVE-SPEC-BACKLOG.md` for history.
