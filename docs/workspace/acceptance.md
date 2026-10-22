# Workspace V2.1 acceptance evidence

2026-10-21 · W13a / WP08 · Source and test inventory; independent acceptance is pending.

The controller owns the release performance run, browser visual QA and final Windows/native/wasm gate. No builds, tests, browser runs or measurements were performed for W13a. Existing test names below describe assertions in source, not passing run results. `pass` requires recorded execution evidence; `partial` records implemented client coverage with outstanding evidence or scope; `blocked` requires an unavailable dependency. No UX case is declared passed by this inventory.

## UX01–UX16

Test paths are under `crates/ui-egui/tests/`; each reference is `test_file::test_name`.

| Case | Status | Evidence in source | Still owed |
|---|---|---|---|
| UX01 First run | partial | `ws_profile::first_run_shows_picker_ux01`, `picker_escape_selects_simple`, `host_or_sample_skips_picker` | Lead gate and browser first-run picker → AUTOCAD_FAMILIAR capture; confirm ribbon, command line and Properties. |
| UX02 Toolbar | partial | `ws_toolbars::dock_each_side_and_float_survive_restart_ux02`, `hide_show_and_reorder`, `escape_cancels_toolbar_drag` | Lead gate; visual four-edge docking, float, hide/show and browser reload. |
| UX03 Ribbon | partial | `ws_ribbon::custom_tab_and_group_order_survive_restart_ux03`, `contextual_dimension_tab`, `ribbon_buttons_run_real_commands` | Lead gate; custom-tab and group-order browser reload captures. |
| UX04 Crash | partial | `ws_persist::crash_recovery_ux04`, `a_drag_is_one_undo_step_and_is_saved_on_release`, `commit_writes_snapshot_and_keeps_20` | These tests reconstruct an app from committed memory-store state. Real process kill/restart, native disk durability and crash/restart video remain due. |
| UX05 Offline | partial | `ws_sync::offline_edit_then_online_syncs_ux05`, `offline_restart_preserves_base_for_later_conflicts` | Lead gate; only MemoryRemote is implemented. Authenticated HTTP and real offline/reconnect evidence are absent. |
| UX06 Conflict | partial | `ws_sync::two_machines_same_field_conflict_ux06`, `different_fields_merge_silently`, `conflict_dialog_lists_fields_and_resolves_keep_both` | Fake remote only; real server ETag/412, two-client revisions and transport integration. |
| UX07 Monitor | partial | `ws_screen::screen_shrink_pulls_floating_back_ux07`, `ws_toolbars::offscreen_toolbar_pulled_back` | Floating controls remain inside the main window. Physical two-monitor disconnect and OS window relocation are untested. |
| UX08 DPI | partial | `ws_screen::controls_reachable_ux08`, `interface_scale_tracks_egui_zoom_without_committing_each_frame` | Lead gate and visual DSF 1.5/2 at 1366×768 and 3840×2160. Reachability test skips combinations below 640×360 logical points; the full requested DPI/zoom Cartesian product is not covered. |
| UX09 Upgrade | partial | `ws_migrate::upgrade_two_versions_then_rollback_ux09`, `migration_is_idempotent`, `aged_profiles_load_without_loss`, `future_minor_keeps_unknown_fields` | Lead gate; aged fixtures are synthetic, not historical production profiles. Operational upgrade/rollback rehearsal. |
| UX10 Tenant | blocked | No authenticated server or tenant-isolation acceptance test. `ws_sync::forbidden_is_reported_not_retried_forever` exercises a test double only. | Backend, real PI ID, tenant fixtures, 403/404 and metadata-isolation tests. |
| UX11 Policy | partial | `ws_toolbars::locked_by_policy_ux11`, `ws_persist::locked_pointers_refuse_user_changes`, `ws_merge::a_lock_survives_deletes_and_replacements_from_later_levels` | Client only; local policy is not an authorization boundary. Server RBAC/ABAC, signed presets and audit evidence. |
| UX12 LISP | partial | `ws_integrity::profile_import_never_runs_lisp_ux12`, `ws_profile::import_rejects_executable_and_secrets`, `ws_persist::import_dry_run_lists_changes_without_applying` | Lead gate, including parallel W10 fixes. Import rejection does not establish native LISP filesystem sandboxing. |
| UX13 CAD integrity | partial | `ws_integrity::switch_100_times_keeps_drawing_ux13`, `ws_persist::profile_save_never_dirties_drawing_ux13`, `ws_profile::switch_during_drag_cancels_drag_keeps_command` | Lead gate after W10; business registry/dependency/publish integrations remain unavailable. |
| UX14 Accessibility | partial | `ws_ribbon::customize_keyboard_only_ux14`, `ws_toolbars::toolbar_keyboard_only_ux14`, `ws_a11y::new_controls_have_accesskit_labels`, `rename_tab_accepts_vietnamese_ime`, `focus_is_visible_on_ribbon` | Lead gate; human keyboard/screen-reader, dark/light and WCAG 2.2 AA review. |
| UX15 Corrupt | partial | `ws_persist::corrupt_store_falls_back_ux15`, `second_damaged_start_keeps_both_copies`, `ws_migrate::corrupt_fixtures_error_not_panic` | Lead gate and native/web corrupt-profile recovery evidence. |
| UX16 Stress | partial | New ignored `ws_perf::restore_p95`, `switch_p95`; `ws_merge::layout_history_is_capped_and_a_new_change_drops_redo`, `ws_persist::commit_writes_snapshot_and_keeps_20` | Release numbers, FileProfileStore commit benchmark, and profiler evidence. Structure/texture counts do not prove absence of a memory leak. |

## V2.2/V3.1 GĐ0: TC12 / TC13 source inventory

2026-10-22 · These TC numbers refer to UX/Workspace V3.1 and are separate from the V2.1 UX cases above. No GĐ0 gate, browser QA or desktop smoke was run for this documentation task. Test references describe assertions in source, not passing execution results.

| Case / scope | Status | Evidence in source | Still owed |
|---|---|---|---|
| TC12 Browser-key blocking | partial | `web_keys::web_defaults_avoid_browser_reserved_keys`, `web_defaults_keep_every_safe_desktop_binding`, `web_binder_rejects_browser_keys`, `web_active_drops_browser_keys_from_profiles`, `web_menu_hides_browser_reserved_hints`; `shortcuts.rs` filters defaults/profile dispatch, rejects browser keys and hides menu hints. Shift+F10 and Ctrl+Shift+C are reserved; F8/F9 remain. | Lead native/wasm gate; browser verification that F5/Ctrl+R remain browser actions and menus omit F5/Ctrl+E hints. Alternative web defaults are intentionally deferred to GĐ3, so this is not full TC12 acceptance. |
| TC13 beforeunload protection only | partial | `drawing_recovery::unsaved_drawings_reported_until_saved` asserts clean → dirty after LINE → clean after SAVEAS. `closing.rs::has_unsaved_drawings` checks all open drawings; `apps/cadcraft-web/src/leave_guard.rs` requests confirmation only when dirty; `main.rs` refreshes the flag and omits the guard for `?host=parent`. | Lead gate and browser evidence after a user gesture: dirty reload/navigation/close prompts; cancelling retains the drawing; a clean page does not prompt. Embedded parent behavior needs separate verification. Browser crash/forced termination is outside this GĐ0 fix; GĐ4 drawing autosave/recovery source coverage is listed below, with browser execution still owed. |

T3 Customize keyboard routing lands separately and is not implemented in this checkout: its agreed behavior sends Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y to layout history even with a focused Customize page button, lets a focused text field retain the keys, and blocks other shortcuts while a Customize widget has focus. Integration and lead verification remain pending.

## V3.1 GĐ4: local persistence source inventory

2026-10-23 · U-C, UX18 and TC13 refer to UX/Workspace V3.1. These tests assert the
following behavior in source; no test run or browser acceptance is claimed by this update.
UI test references are under `crates/ui-egui/tests/`; web references are unit tests under
`apps/cadcraft-web/src/`. All rows remain partial pending the lead's gate and QA.

| Case / scope | Status | Evidence in source | Still owed |
|---|---|---|---|
| U-C Schema 3, checksum and migration | partial | `ws_v3::v3_set_is_camel_case_with_version_3` checks numeric version, wire keys, Device fields and a valid checksum; `gd3_fields_survive_v1_to_v3_and_back` retains ribbon mode, QAT position and density; `checksum_mismatch_is_corrupt` rejects edited data. `ws_migrate::aged_fixtures_migrate_to_v3_and_keep_the_original` checks exact backup bytes for synthetic aged fixtures. | Native/wasm gate; real profile upgrade and deployment rollback rehearsal. |
| U-C Device ownership and portable export | partial | `ws_device::device_layer_sits_between_user_and_session`, `ui_scale_lives_on_the_device`, `export_has_no_device_geometry`, `import_keeps_this_devices_geometry` check precedence, local persistence, stripped geometry and import isolation. `ws_sync::ui_scale_is_not_synced` retains scale locally across restart and excludes it from MemoryRemote. | Real cross-device export/import; no production sync backend exists. |
| U-C Transaction, durability and quota | partial | Web `idb_profile_store::tests::write_commit_updates_mirror_and_queues_one_op` checks one queued commit and Pending → Committed; `conflict_from_transaction_is_reported` preserves the prior mirror; `quota_error_evicts_only_old_snapshots` retains current, latest valid snapshot, damaged evidence and backup after quota failure. Actual transaction wiring is `idb.rs::commit_workspace`. | Real IndexedDB transaction abort, quota exhaustion, concurrent-tab race and fallback tests in browsers. Fake backends do not establish browser atomicity. |
| U-C Separate save states and debounce | partial | `save_state::debounce_is_600_ms`, `layout_state_walks_clean_dirty_saving_saved`, `pending_write_is_not_reported_saved`, `drawing_and_layout_labels_are_separate` cover timing, durable acknowledgement and independent labels. | Browser visibility/pagehide flush and status-bar captures; no cloud sync is enabled. |
| UX18 Last valid state, safe mode and rollback | partial | `ws_v3::checksum_damage_restores_the_last_valid_snapshot` selects revision 7; `ws_persist::safe_mode_starts_simple_and_writes_nothing` checks no workspace-store method calls through safe attachment; `leaving_safe_mode_restores_the_stored_profile` discards temporary edits; `ws_migrate::rollback_restores_the_v1_original` restores exact bytes and suspends writes. | Desktop `--safe-mode` and web `?safe` smoke; web host preloads/migrates storage before safe attachment. Browser startup isolation and asynchronous rollback durability need QA. |
| TC13 Drawing crash recovery apart from workspace recovery | partial | `browser_autosave::crash_recovery_tc13_keeps_drawing_and_workspace_apart` recovers a dirty/pathless drawing without changing the stored workspace and keeps drawing recovery across workspace reset; `recovery_after_snapshot_restore_reports_both` reports both recoveries. Web `drawing_autosave::tests::leftovers_skip_live_and_own_instances` excludes active instances; `drawing_autosave_never_evicts_workspace` restricts sink operations to drawing stores. | Forced browser kill/reopen, stale heartbeat, dirty recovery and independent workspace restore in a real browser. SAVETIME=0, oversize files and unavailable IndexedDB require explicit QA; beforeunload is only an additional leave guard. |

## V3.1 GĐ1B: registry, search and Vietnamese source inventory

2026-10-22 · G1B-T13 / WP01. The identifiers below refer to UX/Workspace V3.1, independently
of the V2.1 cases above (for example, V3.1 UX10 is localization, not tenant isolation).
These are source assertions only: no T13 test run, native/wasm gate, trunk build, visual QA,
desktop smoke or native terminology review was performed by this worker. All cases remain
partial until the lead records execution evidence.

Unit references below use `src/i18n/tests.rs` unless another path is given.

| V3.1 case | Status | Evidence in source | Still owed |
|---|---|---|---|
| UX01 / WP01 Command registry | partial | Engine `tests/command_ids.rs`: `namespaced_ids_are_unique_and_stable`, `resolve_command_accepts_both_forms`, `registry_contexts_status_version`, `command_schema_doc_is_current`; UI dispatch returns outcome codes. | Lead native/wasm gate, schema/control integration verification; authenticated tenant permission enforcement remains deferred. |
| UX08 Command search | partial | `command_palette::lop_layer_and_la_all_find_layer`, `decomposed_query_finds_layer`, `duong_thang_finds_line`, `fuzzy_and_recent_ranking`, `palette_runs_by_keyboard_and_escape_closes`, `hostile_queries_are_bounded`. The shared index serves palette, Command Reference, QAT and Customize. | Lead gate; browser/native search, keyboard and unsupported-command checks. |
| UX10 Vietnamese interface | partial | `translations_cover_commands_menus_tools_and_snap_labels`, `every_marked_ui_literal_has_a_translation`, `translations_cover_ribbon_labels_with_context_fallback`, `every_engine_prompt_has_a_vietnamese_translation`, `vietnamese_catalog_follows_the_glossary`, `no_english_catalog_text_in_p0_screens_vi`. Ukrainian and Vietnamese UI coverage is required; only Vietnamese prompts are required. | Lead gate; 1366×768 and 1920×1080 captures, 200% zoom, desktop smoke; native review pending (user/LPC). Dynamic uncatalogued text and engine history/diagnostics remain English. |
| TC05 Live vi/en switching | partial | `live_vietnamese_switch_preserves_drawing_command_and_saved_choice`, `user_named_ribbon_tab_is_not_translated`: command state, drawing data and user labels survive switching. | Lead test execution and native/browser switching while prompts and dialogs are open. |
| TC06 Editor typing | partial | `ime_guard::typing_in_text_fields_never_dispatches_alias_tc06`, `vietnamese_in_mtext_editor_does_not_run_alias_h08`; `command_palette::palette_ime_enter_never_runs_a_command`. | Real Telex/VNI composition in MText, Customize, palette and command line on each platform. |
| H06 Vietnamese font and fallback | partial | `vietnamese_ui_fonts_cover_catalog_when_shown`, `vietnamese_falls_back_to_english_without_a_covering_font`; P0 renders include Start, AUTOCAD_FAMILIAR + LINE, Customize, Fonts, Plot, Command Palette and Quick Plot. Font-dependent checks print a skip reason without a covering font. | Lead run with a covering font and without optional web fonts; native review pending. |
| H07 Alias and IME command entry | partial | `ime_guard::ime_composition_never_dispatches_alias`, `ime_enter_during_composition_never_repeats_last_command`, `typed_aliases_dispatch_after_commit_h07`. | Real IME commit/Enter/Space sequences and native/browser QA. |
| H08 MText and composition | partial | `ime_guard::vietnamese_in_mtext_editor_does_not_run_alias_h08`, `typing_in_text_fields_never_dispatches_alias_tc06`; Vietnamese/Latin-1 NFC input normalization tests. | Real Telex/VNI editing, caret and composed/decomposed text round-trip evidence. |

`pseudo_loc::pseudo_keeps_placeholders_and_expands`, `pseudo_is_not_saved_and_hides_from_picker`
and `pseudo_ribbon_and_dialogs_fit_1366x768` remain in place. qps layout assertions and the
Vietnamese painted-text check do not replace browser overflow/contrast or native language review.

## V3.1 GĐ3 Task 15: CAD Manager and Minimal presets

2026-10-23 · Source inventory only; build, test execution and visual acceptance remain with the controller.
The picker now offers eleven presets. CAD Manager and Minimal are local layouts; CAD Manager adds no company authorization or policy-locking service.

| Preset / scope | Status | Evidence in source | Still owed |
|---|---|---|---|
| CAD_MANAGER | partial | Expanded ribbon: Home, Standards (`style`, `dimstyle`, `layer`, `purge`, `audit`), Fonts (`fonts`, `style`), Xref (`xref`, `xattach`), Plot (`plot`, `publish`, `pagesetup`); right-docked Layers, External References and Fonts. `ws_profile::cad_manager_preset_uses_only_real_commands` checks both validation modes, exact tab commands, real IDs and panel placement. No Sheet Set tab. | Lead native/wasm gate and visual QA; confirm Xref buttons report Unsupported on web through the existing registry. |
| MINIMAL | partial | TabsOnly ribbon retains the eight standard tabs and Dimension/Hatch/Block contexts; no panels, toolbars or Quick Access, with a floating command line and visible status bar. `ws_profile::minimal_preset_is_tabs_only_with_canvas` applies it at 1366×768 and requires at least 80% canvas height. | Lead test run and browser/native captures. |
| SIMPLE fallback | partial | SAFE_PRESET stays SIMPLE; its JSON is untouched. `ws_profile::simple_preset_is_unchanged_and_still_safe` pins SHA-256 for the original LF and CRLF Git checkout representations and checks default layout/validation. | Lead regression run on native and wasm. |
| Picker / Vietnamese | partial | `picker_lists_every_preset_and_a_row_selects_it` includes both new English labels; `presets_offer_the_vietnamese_names` finds “Quản lý CAD” and “Tối giản” and selects Minimal. Ukrainian/Vietnamese labels and new ribbon glossary entries are supplied. | Lead glossary scan, UI test run and language review. |
| All presets | partial | Existing PRESET_IDS loops cover validation, real commands, unique IDs/ratios, contextual Block commands, round trips, applying, persistence and drawing integrity. The tab test retains exact eight-tab expectations for existing presets and Minimal, with the exact five-tab list for CAD Manager. | Lead execution of these suites and a new eleven-preset performance run; the nine-preset measurements below remain historical. |

## Measured

| Metric | Samples / SLO | Result |
|---|---|---|
| Restore P95 | 50 restores, 200 ribbon/toolbar command controls; ≤1500 ms | 8.0 ms (50 samples) |
| Switch P95 | 100 preset switches with sample drawing; ≤500 ms | 3.1 ms (100 samples; history 100, snapshots 20, egui textures 1→1) |
| Local commit P95 | 100 commits to FileProfileStore in a temporary directory; ≤250 ms | 2.1 ms (100 samples, real disk) |

Release command (run by the `Workspace SLO` job):

```text
cargo test --release -p cadcraft-ui-egui --test ws_perf -- --ignored --nocapture
```

`restore_p95` times app/context creation, MemoryStore startup validation/application and the first egui frame. The fixture contains 100 ribbon command entries and 100 toolbar command entries, all real command IDs. Fixture serialization is outside the timer; disk reads, GPU submission and presentation are not measured. Collapsed/overflow controls remain part of the profile.

`switch_p95` warms all nine presets, then measures command dispatch (including local MemoryStore commit) plus one egui frame per switch. It asserts drawing Arc identity, view and clean state. Every measured iteration checks layout history ≤100 and stored snapshots ≤20. The egui texture count is compared before/after at SIMPLE after warm-up, and printed with the structure counts. These are bounded-structure proxies only: **profiler evidence chưa có** (not available), including allocator/RSS and GPU resource accounting.

`restore_p95` and `switch_p95` are ignored tests in `crates/ui-egui/tests/ws_perf.rs`; `commit_p95` is an ignored unit test in `apps/cadcraft/src/profile_store.rs`, because `FileProfileStore` is private to the desktop binary. All three run in the on-demand `Workspace SLO` job of `.github/workflows/ubotbuild-ci.yml` (`workflow_dispatch` with `perf: true`, release build).

Measured on GitHub Actions ubuntu-24.04 (shared runner, indicative), release build of d4dd370, run 39584899889 job "Workspace SLO". Gate on the same commit (run 39583913358): fmt, native and wasm clippy, workspace tests, layers and the trunk web build all green. Later fix commits are gated by their own CI runs.

## Visual QA

CI web artifact of dae2395 served locally, Chrome driven through DevTools (pointer input injected on the canvas, keyboard real), real DSF 1.5 from OS scaling, 2026-10-21. Only screenshots that were opened count as evidence. Findings fixed afterwards: ribbon buttons without an icon showed "..." (now sized to their label) and the picker list was clipped (now scrolls).

| Scenario | Evidence / verdict |
|---|---|
| Fresh storage → picker → AUTOCAD_FAMILIAR | Verified: picker lists all nine presets; AutoCAD Familiar gives ribbon, Properties and command line. |
| Toolbar dock Top/Bottom/Left/Right, float, hide/show and lock | Verified dock on all four edges, float, and clamp inside the window; lock not exercised. |
| Add ribbon tab/reorder group; Quick Access and Customize | Partial: tabs switch, Minimize Ribbon works, Quick Access and Customize open/close; tab add/reorder and double-click collapse not exercised. |
| Reload web page preserves committed state | Verified: floating Draw toolbar and status-bar item survive reload; preset switch and back keeps the drawing. |
| Fill actual localStorage quota; save reports failure and app continues | Partial: with the quota full the app keeps running, evicts the oldest snapshots and still saves the current set ("Saved locally" is true); a refused current write was not reproduced in the browser. |
| DSF 1.5/2, 1366×768 and 3840×2160; expanded/collapsed ribbon, accessible controls | Partial: DSF 1.5 at 1366×768 and 2555×934 verified; DSF 2 and true 3840×2160 not available on the QA machine. Below the 1366 minimum (1024×640) the menu bar clips. |
| Font/LISP panels, auto-hide peek/pin, dark/light, keyboard/IME | Not exercised in the browser; covered by ws_integrity tests only. |

`ws_persist::store_write_failure_keeps_running` uses `MemoryStore::fail_writes`; it is not evidence of a browser quota experiment. Physical monitor QA and crash video remain separate deliverables.

## §13 handover package

Present means present in source, not runtime acceptance or approval.

| Deliverable | Present | Missing / owner |
|---|---|---|
| Source diff + commit SHA | W13a diff and this report; baseline audit in `audit.md` | Lead-reviewed final diff and commit SHA; worker git is read-only. |
| API OpenAPI | `openapi.yaml`, eight operations, proposed `/api/v1/cad-workspaces` prefix | Prefix approval and real service. |
| JSON Schema | `workspace-profile.schema.json` | Lead validation log. |
| DB migrations | None | Backend schema/database implementation; client JSON migrators are not DB migrations. |
| UX prototype | Runnable Rust ribbon/toolbars/picker/Customize source | Separate approved UX/Figma prototype and stakeholder review. |
| Mapping command IDs | `command-ids.md`, registry metadata and stable-ID tests | Lead gate log. |
| Import/export sample | Inline spec sample in `ws_persist.rs`; preset JSON and export round-trip tests | Standalone portable exported sample/checksum from accepted build. |
| Test fixtures | `crates/ui-egui/tests/fixtures/workspace/` (aged, corrupt, future profiles) | Lead execution evidence. |
| Unit/integration/E2E logs | Test source inventory above | Lead unit/integration logs and browser E2E logs; none generated here. |
| Crash/restart video | None | Native process kill/restart recording (lead/QA). |
| Two-monitor test | In-window shrink/remap test source | Physical two-monitor disconnect evidence (QA). |
| Performance report | Ignored restore/switch tests and measured-results template above | Commit benchmark, all release numbers, profiler evidence (lead). |
| Security review | `security.md` threat model; client import/policy tests | Independent review and server tenant/RBAC/signature/audit evidence. |
| Accessibility checklist | UX14 test inventory and visual QA checklist | Human screen-reader/keyboard/contrast review and sign-off. |
| Migration/rollback runbook | `migration-runbook.md`, synthetic upgrade/rollback tests | Operational rehearsal/logs and server runbook. |
| Known limitations | `limitations.md`, this matrix | Reconcile older W10-pending statements after parallel lane lands. |
| CAD Manager/CTO/QA sign-off | None | Human review; discipline presets remain drafts, LPC source/preset unavailable. |

Release acceptance remains pending. No real backend, LPC company preset, tenant Font/Master Layer registries or Sheet Set service is claimed by the client UI.
