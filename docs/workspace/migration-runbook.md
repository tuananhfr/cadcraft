# Workspace schema migration and rollback

| Input | Read behavior | Next stored form | Kept original |
| --- | --- | --- | --- |
| Legacy `version: 1` workspace | Convert to a user profile, then migrate device geometry | Schema 3 | Exact original bytes |
| `schema_version: "1.0.0"` or another 1.x version, including the historical `future-1.3.0` fixture | Run `device_layer`; move UI scale and geometry to the device | `schemaVersion: 3` | Exact original bytes, kept once |
| Schema 3, numeric or string version | Read current fields and preserve unknown fields | CamelCase schema keys, snake_case enum values | Existing backup retained |
| Major version above 3 | Refuse the newer schema and use SIMPLE | Original kept as damaged evidence | No v1 migration backup created |

Rust fields and internal JSON Pointers remain snake_case. The stored set uses `scope` for profile ownership and `deviceGeometry` for local UI scale, screen information and geometry. User shortcut, alias, status item and dock-tree keys remain verbatim. The checksum covers the wire JSON that is stored, excluding only its top-level `checksum` field.

Startup reads and validates the migrated set before writing it. It keeps the original before the first v3 snapshot or current-set write. A backup failure leaves the current set and snapshots unchanged and reports a failed save; retries must keep the backup first. Backups are outside snapshot pruning.

On desktop, the backup is `<settings dir>/workspace/backup-v1.json`, published atomically and never replaced. To roll back in the running application, execute `ui.workspace.rollback {}` through the UI command/control interface. It returns `{"restored":"1.0.0"}`, restores the original current set and applies its layout. The status reports `suspended`, with the message: “Restored the 1.0.0 workspace. Changes are not saved until restart.” Autosave and sync writes remain suspended for the rest of that session. A missing backup is an error and changes nothing.

For a deployment rollback, exit the v3 application, restore `backup-v1.json` as `current.json` in that workspace directory, then launch the older build. Restarting a v3 build deliberately migrates the v1 original again while retaining the same backup.

In a browser, IndexedDB database `cadcraft` stores the current set in `workspace` at key
`current`, and the exact original in `meta` at `backup:1.0.0`. Its database version is 2
after adding GĐ5 `print-jobs`; that database version is separate from profile schema 3.
Migration from localStorage writes the converted set, snapshots, damaged evidence and
backup in one transaction. Cleanup runs only after it commits, retaining
`cadcraft.ws.current` and `cadcraft.ws.backup:1.0.0` for one release; only copied,
unchanged snapshot/damaged keys and their indices are removed.

`ui.workspace.rollback {}` restores the exact backup through the active store and
suspends workspace writes for that session. With IndexedDB this is an asynchronous
write; the command response is not a durable transaction acknowledgement. It does not
copy the restored set into the legacy localStorage current key. In the localStorage
fallback, the backup is `cadcraft.ws.backup:1.0.0` and rollback writes `cadcraft.ws.current`.

For a browser deployment rollback, close all tabs for that origin before changing stored
values or serving the older build. Keep the IndexedDB database and backup keys. An older
build reads the retained `cadcraft.ws.current`; it sees the pre-migration layout, not later
v3 edits. If the legacy current key is missing or must be restored, copy the exact v1
backup text from localStorage `cadcraft.ws.backup:1.0.0`, or IndexedDB `meta` key
`backup:1.0.0`, into localStorage `cadcraft.ws.current`, then open the older deployment.
Do not substitute the v3 wire JSON. This operational browser rollback still needs QA.
A later v3 deployment reads its existing IndexedDB current set; restoring only the legacy
key does not replace that set.

The GĐ3 fields `ribbonMode`, `quickAccessPosition` and `density` survive migration and subsequent saves (`ws_v3::gd3_fields_survive_v1_to_v3_and_back`). Enum values such as `tabs_only` remain snake_case. Bare legacy `ui_scale` is a read-only migration input; wire round-trip checks therefore use migrated fixture values, where the scale lives in `deviceGeometry`.

Adding future migrations requires a named `Step`, a matching `CHANGELOG` row, a synthetic previous-version fixture, a stored-patch startup test and an idempotence check. `migrate_with(value, steps)` injects a table for tests; an empty table can read the original v1 profile without upgrading it. Unknown enum values still fail typed validation. Command ID renames continue to follow the engine registry through `rename_command_ids`.

Stored `layoutPatch` is expanded over the current base and policy before profile migration. Changes to the patch structure itself require a raw-patch migration before that expansion; a full-profile fixture alone cannot prove that path. The tests include old `layout_patch` input with geometry and scale and verify actual startup, device ownership, backup and restart.

Synthetic fixture coverage includes profiles aged 1, 3 and 5 years, the legacy-format 10-year fixture, unknown future fields and invalid/truncated JSON. These are hand-written examples, without real user data. Pre-release ws-v21 profiles that explicitly docked the command line remain docked; no migration rewrites that choice merely because the SIMPLE default now floats.

Source assertions: `ws_migrate::aged_fixtures_migrate_to_v3_and_keep_the_original`,
`rollback_restores_the_v1_original` and `backup_failure_never_commits_the_migrated_set`;
web `idb_profile_store::tests::local_storage_set_is_copied_once`,
`legacy_migration_keeps_the_original_and_signs_v3` and
`cleanup_preserves_concurrent_local_storage_edits`. These tests use synthetic fixtures
and fake storage; no migration/rollback run is claimed by this documentation update.
