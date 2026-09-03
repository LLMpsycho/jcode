# Phase 1 legacy write guard scope

`edit`, `multiedit`, `patch`, `apply_patch`, and overwrite `write` now share the
server-owned snapshot ledger. The configured `[editing.read_guard]` mode is
`warn` by default and supports `off`, `warn`, and `block`. Same-revision checks
apply to every existing-file mutation. Exact affected ranges are used by
`edit`, unified patch hunks, and resolved `apply_patch` chunks. `multiedit` uses
a conservative whole-file range because its sequential replacements can shift
later match locations.

Both patch tools compute and guard every requested file before publishing the
first file. This prevents parse, existence, stale-revision, and coverage failures
in later files, plus `apply_patch` context-match failures, from causing
earlier-file mutation.

Publication is still sequential for the legacy multi-file patch tools. A
filesystem I/O failure after one successful publication can therefore leave a
partial update. Converting these tools to staged same-directory renames with
rollback is intentionally deferred; `anchored_edit` remains the atomic path.
