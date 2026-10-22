# Workspace profile security

LISP file loading uses a content-based review gate on desktop and web. JSON
`appload` and `(load)` refuse unapproved UTF-8 source with
`LISP_NOT_REVIEWED: NAME (sha256 DIGEST)`; they never open a prompt. Typed/menu
APPLOAD and dropped files ask `Load unreviewed LISP file NAME (sha256 12345678)?
[Yes/No/Always] <No>:` in the `APPLOADTRUST` interactive machine. Yes loads the
reviewed bytes once; Always also approves their full SHA-256. No, Enter and Escape
leave the file unloaded. Editing the source changes its digest and requires review
again. Approval applies to identical contents regardless of filename or location.

`lisptrust {action: "list"|"approve"|"revoke", sha256}` manages up to 256 unique
SHA-256 approvals. The existing user-profile preferences store them alongside
FONTALT on desktop and web, under `lispApproved`; these approvals are not part of
a portable workspace. Invalid digests are ignored on restore. A poisoned approval
store refuses loading. Startup `acad.lsp`/`acaddoc.lsp` in `<settings>/support`
retain their existing location-based startup trust. Other files loaded by those
routines still require review. Direct local `lisp {code}` remains available to
callers that already hold CLI/control authority; this is not an arbitrary-code
sandbox or protection against a caller that can rewrite local settings.

MCP may list approvals and write folders, but only the user can approve/revoke
source or change write folders. `USER_ONLY_COMMANDS` and the shared
`USER_ONLY_ACTION_RULES` hook govern these permissions; Company print-profile
mutations are also refused, including imports and default Company lock/unlock
actions. Both headless and remote MCP calls carry their origin into dispatch, including nested JSON commands, aliases and
each input within a script. No MCP input can answer or cancel a pending
`APPLOADTRUST` prompt. LISP routines also cannot grant themselves permissions.

Desktop LISP writes are restricted to canonical allowed folders. The desktop host
creates `<settings>/support` and `<settings>/lisp-out` at startup and installs them
as the defaults; the current drawing's directory is not implicitly allowed.
`lispsandbox {action: "list"|"add"|"remove", path}` manages the process-wide list
(up to 64 existing directories). MCP may list folders but may not add or remove
them. Changes last for the process; they are not saved in the workspace profile.

The checks cover `open` in `w`/`a` mode (including a recheck at `close`), and
`export`, `exportpdf`, `plot`, `publish`, and every `ubplot.*` output parameter
(`path`, plus `out`/`dir` for `ubplot.*`) while `Session::lisp_running()` is true.
`ubplot-publish` also requires a `.pdf` extension, ignoring case. A refusal uses
`LISP write blocked: PATH is outside the allowed folders (LISPSANDBOX)`; `open`
returns nil and echoes it. Existing parents and final symlink targets must stay
within canonical roots; Windows comparisons ignore case. Missing parents are
refused. Reads and browser file delivery retain their existing behavior.

SAVE/SAVEAS/WBLOCK invoked through `(command ...)` still use the host's I/O hooks
and are outside this write policy. LISP has no network builtin. This is a local
application policy, not an OS sandbox: path checks cannot prevent another process
from replacing directories or symlinks between validation and the filesystem call.
Direct local `lisp {code}` callers already have control-channel/CLI authority;
ordinary JSON commands outside a running LISP routine keep their existing paths.

Only net entity changes are audited. The session ring retains 1000 records; `lispaudit`
returns newest first, defaults to 20 and caps requests at 1000. Records contain time,
source SHA-256 when known, entity counts, up to 50 command ids and the undo label.
The panel shows the latest 20. Desktop writes `<settings>/logs/lisp-audit.jsonl`,
rotating before the next record would exceed 5 MiB, with three backups. Web audit
records last only for the session. Logs are local and editable, not a security ledger.

Print-profile Company locks apply to later levels and UBPLOT parameter overrides,
including LISP calls. Company policy is local policy, not a security boundary:
someone controlling the local store can change it. MCP cannot change Company policy
or LISP permissions. Print profiles accept at most 256 KiB, depth 32 and 256 locks per
level; template registries have at most 1000 templates and 2 MiB JSON. Template audits
accept at most 64 MiB, 10,000 blocks and 200,000 entities. The LISP approval cap is 256
hashes (smaller than the draft plan's 4096 to fit host preferences), write roots are
capped at 64, and file reads retain a 64 MiB limit.

This is the W12 client threat model and proposed server contract (WP02/WP06),
not evidence of deployed server security. The `/api/v1/cad-workspaces` prefix in
[openapi.yaml](openapi.yaml) is a **PROPOSAL awaiting approval**, chosen to avoid
Drupal's existing `/api/v1/workspace` NAS API. The eight operations retain §6
semantics. Backend hosting, authentication, CSRF, proxy/CORS and transport wiring
are not yet implemented for workspace synchronization.

The trust boundary is external JSON → validation → migration → typed profile →
policy merge → UI application. A profile describes UI state; tenant/owner fields,
command ids and unknown fields do not confer permissions. Files and browser
storage are local data, not trusted identity or a secure credential store.

| Threat | Client measure implemented and regression coverage | Server work not yet implemented |
| --- | --- | --- |
| Malicious import consumes memory, hides executable payloads or corrupts state | W3 `validate.rs` checks 256 KiB raw text before parsing, nesting ≤32, arrays ≤500, strings and keys ≤1024 characters, and caps reported issues at 64. Import-mode checks traverse values and object keys. W5 `commands.rs::import` validates, migrates and deserializes before application; native file reads are capped at limit + 1 byte. Tests: `ws_profile::limits`, `invalid_json_and_non_objects_are_reported`, `import_checks_object_keys_like_values`; `ws_persist::import_dry_run_lists_changes_without_applying` checks preview does not apply and a rejected payload leaves the layout intact. | Enforce body limits before allocation, schema/patch validation and identical dry-run checks on every server entry point; rate limits and resource budgets. Local checks do not protect a server. |
| Tenant A reads or writes tenant B's profile, base, snapshot or metadata | W5 portable export clears tenant_id, owner_id and etag. W11 MemoryRemote simulates per-id denial, not tenant isolation: `ws_sync::memory_remote_checks_etag_permissions_and_revision_overflow` and `forbidden_is_reported_not_retried_forever`. There is no client tenant authorization test or trustworthy tenant identity. | RBAC/ABAC with principal-derived tenant filters on list, read, clone, patch, publish, import, export and restore, including nested lookups and 412 responses. UX10 is blocked: real tenant A/B 403/404 tests with no metadata disclosure are not yet implemented. |
| A user overwrites a company preset or bypasses a locked field | W5 `merge.rs` applies Platform → Product → Tenant → Company → Discipline → Project → User → Device → Session, preserving higher-level locks including ancestor replacement/deletion. Policy is read from a host/local store, not accepted as import authority. Edits overlay bundled presets. Tests: `ws_merge::a_lock_survives_deletes_and_replacements_from_later_levels`, `ws_persist::locked_pointers_refuse_user_changes`, `policy_change_reaches_committed_user`, `published_preset_is_immutable`; W7 `ws_toolbars::locked_by_policy_ux11`. | Server-side lock and RBAC/ABAC enforcement, immutable published versions, admin approval, Ed25519 signing/verification and key lifecycle, audit log for publish/migration/restore. Local policy can be changed by someone controlling local storage; it is a UX11 client guard, not a security boundary. |
| A profile starts LISP, APPLOAD or another executable | W3 import rejects executable suffixes `.lsp .fas .vlx .dvb .arx .dll .exe .js .ps1 .bat .cmd`, normalized exact keys `script/lisp/exec/eval/code/onload/startup`, and absolute/UNC/file: paths. W5 import applies layout/preferences only; it never dispatches imported text or command ids. Tests: `ws_profile::import_rejects_executable_and_secrets`, `import_payload_rules_cover_the_whole_list`, `import_checks_object_keys_like_values`, plus the rejected nested onload case in `ws_persist::import_dry_run_lists_changes_without_applying`. Composite keys such as extra_startup or inert text may survive validation but have no execution hook. The W10 end-to-end test `profile_import_never_runs_lisp_ux12` is not yet present in this checkout. | Revalidate manifests and disallow executable hooks independently of the client. LISP sandbox/compatibility scanning and permissions are separate work, not privileges granted by a profile. UX12 engine-history/loaded-files end-to-end evidence is not yet available here. |
| A shared profile leaks tokens, credentials or sensitive local paths | W3 rejects keys containing normalized `token/password/secret/cookie/authorization/apikey` in both Local and Import modes. W5 export validates in Import mode and refuses unsafe output rather than silently exporting it; it omits host identity and does not capture recent drawing paths. Tests: `ws_profile::import_payload_rules_cover_the_whole_list` covers secret keys in both modes; `ws_persist::export_has_no_secrets_and_round_trips` covers portable output without recent paths. No token field or auth token store exists in profiles. | Authentication secrets must live outside profiles, manifests, snapshots and audit payloads. Add server-side secret/path rejection, safe logging and credential lifecycle controls. Key-name screening is not content classification: arbitrary free text can still contain a pasted secret, and Local mode does not reject all paths. Encryption or general secret detection is not yet implemented. |
| Concurrent writers silently lose revisions or retry forbidden writes indefinitely | W11 `sync.rs` commits locally before sync, preserves a base for three-way merge, and `conflict.rs` exposes KeepMine/UseServer/KeepBoth instead of silent last-write-wins. Tests: `ws_sync::offline_edit_then_online_syncs_ux05`, `two_machines_same_field_conflict_ux06`, `different_fields_merge_silently`, `forbidden_is_reported_not_retried_forever`, `repeated_stale_responses_have_a_bounded_retry`, `sync_waits_for_local_commit_and_failed_storage`. | Atomic server If-Match comparison/write with strong ETags and 412, persistent version history, authorization on every retry and an async HTTP transport. MemoryRemote is a synchronous test double, not a backend or evidence of deployed UX05/UX06. |

GĐ4 portable exports contain only the user profile, with tenant/owner/ETag cleared.
`commands.rs::export_text` strips Device geometry before camelCase conversion and adds a
canonical SHA-256 checksum, then validates in Import mode. Exports omit `deviceGeometry`,
UI scale, screen data, panel/toolbar coordinate ratios, command-line height ratio and the
dock tree. Import strips incoming geometry and keeps the receiving device's state.
Local drawing paths are not captured; absolute/UNC/file paths elsewhere in a profile
cause export refusal through Import-mode validation. Relative inert text may remain;
this is path screening, not general secret-content detection. Coverage:
`ws_device::export_has_no_device_geometry`, `import_keeps_this_devices_geometry`,
`ws_v3::export_is_wire_v3_with_checksum`, and
`ws_persist::export_has_no_secrets_and_round_trips`.

Workspace export does not include drawing autosaves, print manifests or migration
backups. Those local stores may contain drawing data or source paths and are not
portable profile files. IndexedDB's origin isolation and checksums do not encrypt them
or authenticate their author. Import rejects a mismatched checksum before applying
changes and warns when no checksum exists (`ws_v3::import_refuses_checksum_mismatch`,
`import_without_checksum_warns`).

W5 `store.rs` defines atomic current-set replacement, retention of 20 committed
snapshots and preservation of damaged input. `ws_persist::corrupt_store_falls_back_ux15`,
`store_write_failure_keeps_running`, `restore_creates_new_revision` and
`a_damaged_set_that_was_not_kept_is_not_overwritten` cover recovery behavior.
These local snapshots are neither tamper-proof storage nor a server audit log.
[The migration runbook](migration-runbook.md) covers fail-closed enum handling
and the additional risk of applying old stored patches over today's base.

The HTTP contract uses RFC 6902 JSON Patch; W11 uses a custom merge patch with
id-keyed arrays. Conversion, server schema enforcement, signed preset bytes and
trust anchors remain not yet implemented. Export checksums detect accidental
corruption and do not replace an Ed25519 signature. Before a backend is enabled,
server acceptance must include UX10 tenant isolation, policy bypass attempts,
import payloads, stale writes and immutable signed publication with audit evidence.
