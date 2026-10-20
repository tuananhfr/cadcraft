# Workspace profile migration runbook

Code: `crates/ui-egui/src/wsprofile/migrate.rs`. Tests: `crates/ui-egui/tests/ws_migrate.rs`.
Fixtures: `crates/ui-egui/tests/fixtures/workspace/`.

## What `migrate` does

`migrate(value)` takes a stored profile as raw JSON and returns `Migrated { value, steps, backup }`.

- Works on JSON, not on the typed model, so fields this build does not know are kept.
- `steps` lists the steps that changed the document. It is empty when the document is already current,
  so running `migrate` twice gives the same value and no steps the second time.
- `backup` is the input exactly as received, set only when `steps` is not empty. Store it before
  replacing the profile; it is the rollback copy.
- A document is accepted only if the result still parses as a `WorkspaceProfile`.

Outcomes by schema version `X.Y.Z` of the input:

| Input | Result |
| --- | --- |
| Legacy `workspace.json` (`{"version":1,"workspace":{..}}`) | Converted by `from_legacy_v1` to a `user` profile over `SIMPLE`, step `legacy_v1` |
| Same major as `SCHEMA_VERSION`, older | Upgraded through `STEPS` |
| Same major, newer minor or patch | Returned unchanged, unknown fields kept, no steps |
| Newer major | `MigrateError::NewerMajor`; startup falls back to the `SIMPLE` preset and keeps the snapshot |
| Older major with no path, missing or unreadable `schema_version`, wrong types, not an object | `MigrateError::Invalid` / `NotAnObject` |

Command ids listed in `cadcraft_engine::cmd::registry::RENAMED` are rewritten in toolbars, ribbon groups,
quick access, favorites, hidden tools, shortcut keys and alias targets (step `rename_command_ids`). Ids
the build does not know are left alone; the UI shows them as inactive placeholders.

## Adding a migrator

1. Change `SCHEMA_VERSION` in `wsprofile/schema.rs` (patch for a fix, minor for an added field, major only
   for a change older code cannot read).
2. Add a `Step { name, from, to, apply }` to `STEPS` in `migrate.rs`. `from` is the previous
   `SCHEMA_VERSION`, `to` the new one. `apply` edits the JSON in place, must not drop fields, and must be a
   no-op on a document that already has the change. Return `MigrateError::Invalid` rather than panic.
3. Add a fixture under `tests/fixtures/workspace/` written by hand in the previous schema (no real user data,
   keep it under 3 KB) and add it to `VALID_FIXTURES` in `ws_migrate.rs`; the idempotence and no-loss tests
   then cover it.
4. Add a `(to_version, one-line change)` row to `CHANGELOG`. The last row must equal `SCHEMA_VERSION`; a
   test checks this and that every step has a row.
5. Update the schema document and presets if the new field needs a default there.

`migrate_with(value, steps)` runs the same logic with a step table you pass in. Tests use it to exercise
multi-version upgrades without changing `SCHEMA_VERSION`.

## Fixtures

| File | Models |
| --- | --- |
| `legacy-v1.json` | Current `apps/cadcraft` `workspace.json` |
| `aged-1y.json` | 1.0.0 profile using a command id that was later renamed |
| `aged-3y.json` | 1.0.0 profile that predates most optional fields |
| `aged-5y.json` | 1.0.0 profile with a panel that no longer exists |
| `aged-10y.json` | Older legacy file with a key the app no longer writes |
| `future-1.3.0.json` | Written by a newer minor: unknown fields at several levels |
| `corrupt-truncated.json`, `corrupt-wrongtypes.json` | Must produce an error, never a panic |

The aged files are hand-written guesses at how the schema could look over time, not real user data.

## Rollback

Before a migrated profile replaces the stored one, the store keeps `Migrated::backup`. To undo an upgrade,
run the `ui.workspace.restore` command (added with the store in W5): it writes the backup back unchanged.
The backup serializes to the same bytes as the original input.

A profile that fails to migrate is not deleted: the app loads `SIMPLE` and keeps the broken snapshot so it
can be repaired or restored.
