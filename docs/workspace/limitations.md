# Workspace limitations

Floating panels, toolbars and the command line stay inside the main application window. UbotCAD never opens a second native window, so a panel cannot be torn off onto another monitor; a main window stretched across two monitors carries its floating panels with it. On a PC, when a second monitor is disconnected, the operating system moves or shrinks the main window, or moves it to a screen with another display scale. Every change of window size, display scale or interface zoom remaps the live layout: floating rectangles and docked split and accordion sizes keep their proportions of the content area (the same fractions the device layer stores), then the dock clamp bounds each Canvas-path split to 0.15–0.85 and every floating panel to 85% per axis of the content area. The 15% canvas share is asserted for the tested layout, not guaranteed for every nested split tree; see the GĐ3 limits below. Going back to the original screen restores the original sizes, except for panels that had to grow to their minimum size (160×120 points) or move to stay on the smaller screen; those keep the adjusted geometry. The desktop app runs with `persist_window: false`, so the window always opens at its default size and position, never on a monitor that is no longer attached. These rules are covered by `ws_screen::unplugging_a_wide_monitor_pulls_panels_back_tc15` and `ws_screen::ppp_change_with_shrink_keeps_layout_recoverable_h13` with simulated screens; monitor disconnection has not been tested on physical hardware (UX07, TC15 and H13 partial). Multi-monitor handling does not apply on the web: the page is the window and the browser owns monitor placement; browser window resizes and zoom use the same remap.

Status-bar item visibility belongs to the workspace profile. Hiding a drafting toggle does not change its system variable; those settings continue to persist through `PROFILE_VARS`. The schema IDs `lineweight`, `transparency`, `quickprops`, `annotation`, `workspace`, and `settings` address the existing lineweight, transparency, Quick Properties, annotation visibility/scale, workspace picker, and Drafting Settings controls. `layouts` hides the layout tabs and their add/list buttons; `coordinates` hides the coordinate readout. Unknown status-bar IDs have no control to render: they are retained in the profile and ignored without a placeholder.

SIMPLE has neither a ribbon nor Quick Access buttons. Its default command line floats at the bottom centre with the existing 48% width rule (360–760 points, bounded by the canvas). Top and Bottom command lines reserve their own row, sized by `height_ratio` and constrained to leave room for the drawing. Interface scale is 0.75–2.0; the Options slider uses steps of 0.05, and egui keyboard zoom is saved through the normal workspace debounce.

The in-window menu bar scrolls horizontally when its menus and Quick Access exceed the available width. When zoom or a shorter window leaves insufficient height for the drawing, the ribbon steps down through Compact to TabsOnly without changing its saved mode. `crates/ui-egui/src/ribbon/render.rs::rendered_mode` and `crates/ui-egui/tests/ribbon_modes.rs::ribbon_steps_down_when_height_is_short_h14` cover this behavior.


Workspace synchronization currently uses an in-memory ProfileRemote test double. No HTTP
backend, authenticated tenant identity, tenant isolation (UX10), server-side RBAC/ABAC or policy
locks, Ed25519-signed company publication, or publish/migration/restore audit log is implemented.
The `/api/v1/cad-workspaces` prefix in openapi.yaml is a proposal awaiting approval to avoid
Drupal's existing `/api/v1/workspace` NAS endpoint. HTTP uses RFC 6902 JSON Patch; adapting the client's
custom keyed-array merge patches is future work.

Import/export screening bounds hostile payloads and rejects executable hooks, secret-like keys
and sensitive paths in Import mode. It is not general secret-content detection or a LISP sandbox;
unknown inert text can be preserved without execution. Local storage and policy files are not
authorization boundaries or encrypted credential stores. UX12 end-to-end loaded-files/command-history
evidence remains outstanding until the profile_import_never_runs_lisp_ux12 test lands with the LISP panel.

The supported minimum working area is 640×360 logical points (window size divided by display scale and interface scale). Below
that, for example 1366×768 at 200% display scale with interface scale 2.0 (about 341×192 points), the menu bar, status bar and
command line leave no usable drawing area; the layout is not guaranteed there.

On macOS the native menu bar installs its key equivalents once at start-up; after a shortcut is rebound in Customize
or a workspace with other shortcuts is selected, the native menu still shows and answers the old key. The shortcut
router inside the window follows the new bindings. Refreshing the native accelerators needs a macOS build to verify.

A workspace whose pending changes cannot be saved (storage full or refused, or a damaged set that could not be
kept) cannot be left for another workspace, so its changes are not lost silently; the error stays on screen and
the save is retried. Reset to the preset is the way out.

## V3.1 GĐ3 chrome and focus limits

2026-10-23 · Source inventory; native smoke, browser behavior and visual QA are pending.
Paths below are relative to the repository root.

| Scope | Current limit | Code / test evidence |
|---|---|---|
| Fullscreen: desktop / web | Desktop sends native viewport requests and accepts programmatic calls. Web requires an application click plus browser permission; an iframe needs `allow="fullscreen"`. A refused or unavailable request enters Focus mode with a status message. Programmatic web calls return Unsupported, including control/MCP. Fake hosts do not verify real browser activation timing. | `crates/ui-egui/src/fullscreen.rs` (`toggle`, `rejected`, `sync`); `apps/cadcraft-web/src/fullscreen.rs` (`WebFullscreen::request`, `install`), `apps/cadcraft-web/src/main.rs` event handling; `crates/ui-egui/tests/focus_fullscreen.rs::web_fullscreen_requires_a_user_gesture`, `asynchronous_fullscreen_rejection_and_control_requests`. |
| Focus / Escape | Focus is a temporary render override; the command line can float or be hidden. Commands, selection and dialogs take priority over leaving focus; IME composition keeps focus active. Browser Escape also belongs to the browser's fullscreen handling. Desktop defaults are Ctrl+Shift+F for fullscreen and Ctrl+0 for focus; web assigns neither. | `crates/ui-egui/src/focus.rs` (`hides`, `route`, `escape`); `crates/ui-egui/tests/focus_fullscreen.rs::focus_mode_restores_the_exact_layout`, `escape_cancels_command_before_leaving_focus_tc04`, `ime_composition_never_leaves_focus_mode`, `focus_hidden_command_line_and_desktop_keys`. |
| Ribbon at short heights / 200% | Expanded can render as Compact or TabsOnly without changing the saved preference. Flyout groups wrap and scroll vertically; all commands need not be simultaneously visible. Overflow tabs use the `»` menu. These bounds are harness assertions, with real-browser visual QA pending. | `crates/ui-egui/src/ribbon/render.rs` (`rendered_mode`, `tab_strip`, `flyout`); `crates/ui-egui/tests/ribbon_modes.rs::tabs_only_flyout_reaches_every_command_at_200_percent_tc11`, `tab_overflow_keeps_every_tab_reachable`, `ribbon_steps_down_when_height_is_short_h14`. |
| Toolbar conversion | Dropping a toolbar copies its known commands into a new ribbon group and hides the source. It does not embed a live toolbar. Unknown IDs are reported as skipped, while original toolbar data stays available; Hidden ribbon refuses conversion. Layout undo reverses the conversion. | `crates/ui-egui/src/ribbon/mod.rs::convert_toolbar_to_group`; `crates/ui-egui/tests/ws_toolbars.rs::toolbar_to_ribbon_reports_unknown_commands_and_keeps_source_data`, `hidden_ribbon_is_not_a_drop_target`, `toolbar_to_ribbon_is_one_layout_undo_step`. |
| Dock bounds | Each split on the Canvas path has a 0.15–0.85 ratio bound; fixed sizes use content-axis dimensions. Each floating group is capped at 85% per axis and moved inside content. The 50-cycle test checks one layout; it does not establish a global 15% canvas share for every nested tree, or uncovered canvas under several overlapping floating groups. | `crates/ui-egui/src/dock_clamp.rs::clamp_layout`; `crates/ui-egui/tests/dock_clamp.rs::fifty_dock_cycles_keep_canvas_usable_h02_tc02`, `floating_panel_never_covers_the_canvas`. |
| Monitor / window restoration | Floating controls belong to the main window, with no separate native monitor placement. Resize/scale remaps proportions then clamps; minimum-size or edge corrections can change the rectangle retained on returning. Desktop starts at default window geometry (`persist_window: false`); browser monitor placement is outside the app. Tests simulate screens, not physical disconnects. | `crates/ui-egui/src/wsprofile/apply.rs` (`remap_screen`, `remap_docking`, `floating_rect`); `apps/cadcraft/src/main.rs`; `crates/ui-egui/tests/ws_screen.rs::unplugging_a_wide_monitor_pulls_panels_back_tc15`, `ppp_change_with_shrink_keeps_layout_recoverable_h13`. |
| Density / accessibility | Comfortable gives sampled ribbon/QAT/toolbar/file-tab buttons ≥32-point targets. Contrast tests cover selected opaque token pairs in light/dark themes; they do not certify every widget, rendered text, disabled state or full WCAG conformance. | `crates/ui-egui/tests/density.rs::comfortable_targets_are_at_least_32_points`, `theme_text_and_focus_contrast_meet_wcag`; `crates/ui-egui/src/theme.rs` (`apply_density`, `contrast_ratio`). |
| Alias editor | Aliases use ASCII letters/digits/underscore/hyphen, 1–16 characters, stored uppercase. Command names cannot be shadowed; built-in/user conflicts need confirmation. Import skips invalid/conflicting entries unless an allowed conflict is confirmed. Vietnamese command search does not make Vietnamese aliases valid. | `crates/ui-egui/src/customize/alias_tab.rs` (`check_alias`, `checked_command`, `import`); `crates/ui-egui/tests/alias_editor.rs::vietnamese_alias_is_rejected_with_a_reason`, `alias_cannot_shadow_a_command_name`, `overriding_a_builtin_alias_requires_confirmation`, `aliases_export_and_import_round_trip`. |
| Block context / replacement | Any INSERT-only selection activates the same Block context, including U-BOT; mixed selections do not. `blockreplace` targets an existing definition and rejects external references and locked-layer references. It preserves transforms and synchronizes attributes by tag; no special U-BOT semantics are defined. | `crates/ui-egui/src/ribbon/contextual.rs::active_contexts`; `crates/engine/src/cmd/blocks/replace.rs::run`; `crates/ui-egui/tests/block_context.rs::u_bot_block_is_handled_like_any_block`; `crates/engine/tests/block_replace.rs::blockreplace_matches_attributes_by_tag`, `blockreplace_rejects_mixed_locked_and_invalid_targets_atomically`. |
| Panels / presets | Blocks and Command History use in-window docking; F2 keeps its command-line expansion behavior and `ui.dialog.blocks` redirects to the Blocks panel. Sheet Set remains a disabled Planned placeholder. CAD Manager is a local layout, not a company permission service; Minimal uses TabsOnly and SIMPLE remains the safe fallback. | `crates/ui-egui/src/docking.rs`, `crates/ui-egui/src/menus.rs`, `crates/ui-egui/src/wsprofile/presets.rs`; `crates/ui-egui/tests/dockable_panels.rs::legacy_blocks_dialog_id_still_works`, `sheet_set_stays_a_disabled_placeholder`; `crates/ui-egui/tests/ws_profile.rs::cad_manager_preset_uses_only_real_commands`, `minimal_preset_is_tabs_only_with_canvas`, `simple_preset_is_unchanged_and_still_safe`. |

## V2.2/V3.1 web keys (GĐ0 / GĐ3 Task 11)

2026-10-22 · Implementation inventory; browser QA and release acceptance remain pending.

Keys in the static `BROWSER_RESERVED` table are removed from web defaults and saved-profile dispatch, refused by the shortcut editor and hidden from menu hints. The table is an application rule, not a detection of every browser/OS shortcut; real key delivery still needs browser QA. Source: `crates/ui-egui/src/shortcuts.rs` (`BROWSER_RESERVED`, `WEB_ALTERNATES`, `defaults_for`, `active_for`, `bind_for`), with tests in `crates/ui-egui/tests/web_shortcuts.rs` listed below. Customize → Keyboard warns about a browser key before assignment and disables Assign Shortcut until a different key is entered. The web defaults add these alternatives to the filtered desktop table:

| Command / control | Desktop default | Web default / entry point |
|---|---|---|
| Properties (`ui.panel.properties`) | Ctrl+1 | Ctrl+Shift+1; Window → Properties Inspector |
| Command line (`ui.toggle.cmdline`) | Ctrl+9 | Ctrl+Shift+9; Window → Command Line |
| Command reference (`ui.dialog.commands`) | F1 | Shift+F1; Help → UbotCAD Help |
| Command Palette (`ui.palette`) | Ctrl+Shift+P | No key; Help → Command Palette or the button in Command Reference |
| New drawing (`new`) | Ctrl+N | No key; File → New Drawing |
| Copy with base point (`copybase`) | Ctrl+Shift+C | No key; Edit → Copy with Base Point |
| Tool Sets (`ui.toggle.toolsets`) | Ctrl+3 | No key; Window → Panels → Tool Sets |
| Object snap (`osnap`) | F3 | No key; status bar or Command Reference |
| Isoplane (`isoplane`) | F5, Ctrl+E | No key; Command Reference or command line |
| Grid (`grid`) | F7 | No key; status bar or Command Reference |
| Polar tracking (`polar`) | F10 | No key; status bar or Command Reference |
| Object snap tracking (`otrack`) | F11 | No key; status bar |
| Dynamic input (`dynmode`) | F12 | No key; status bar |

Ctrl+O/S/P/Z/Y/A/C/V/X retain their existing web actions. Ctrl+Shift+Z (Redo), Ctrl+Shift+S (Save As), Ctrl+Shift+A (Group Selection), Ctrl+Shift+V (Paste as Block), F2 (Command History), F8 (Ortho) and F9 (Snap) also keep their desktop defaults. In Customize, Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z operate on layout history on both platforms; a focused text editor keeps its own undo keys.

The browser keeps Ctrl+L/T/W/R, Ctrl+N, Ctrl+1…9, Ctrl+E, F1/F3/F5/F6/F7/F10/F11/F12, Shift+F10, Ctrl+Shift+C/P/N/T/W/I, Ctrl+Tab, Ctrl+Shift+Tab and Alt+F4. These cannot be assigned on the web, including through imported profiles or replacement bindings. F9 is deliberately available to Snap. Desktop defaults stay unchanged, including Ctrl+Shift+P for Command Palette.

Regression sources: `web_shortcuts::web_alternates_avoid_browser_keys_h09`, `properties_and_command_line_have_web_keys`, `web_commands_keep_a_way_in`, `ctrl_z_in_customize_undoes_layout_on_both_platforms_tc19`, plus `web_keys` and `ws_ribbon::shortcut_defaults_unchanged`. These are assertions in source; native/wasm CI and real-browser shortcut checks remain for the lead.

Standalone web pages request beforeunload confirmation only while an open drawing has unsaved changes. Browsers require a prior user gesture and control the prompt text; this is not crash recovery or drawing autosave. Embedded `?host=parent` mode delegates leave protection to the parent through dirty reporting. GĐ4 adds IndexedDB drawing autosave and recovery separately; see the limits below.

PUBLISH delivers one download per output file, without a zip in GĐ0. The browser may require permission for multiple downloads. PLOT, EXPORT, PDFBOOK, plotter saves and workspace export also use file delivery; embedded mode delegates delivery to its host. Workspace JSON import has a separate upload route; drawing uploads continue through `open_bytes`. DXF IMPORT still opens the file as a new drawing rather than inserting it (unchanged in GĐ0; deferred for GĐ7 review).

Workspace revision checks detect a stale writer before commit, on desktop as well as web. With IndexedDB, web revision checking, current-set replacement, snapshot creation and pruning share a read/write transaction; BroadcastChannel updates the other tabs. Conflicts offer a choice of layouts through `ui.workspace.resolve`. The localStorage fallback still has a non-atomic check-then-write window. These local conflicts are separate from remote synchronization; real concurrent-tab browser QA remains pending.

PDFBOOK still selects frames by the requested height. Other-height closed outlines are ignored; only requested-height frames that match no paper receive `FRAME_PAPER_MISMATCH`. Invalid coordinates/dimensions report `FRAME_OUT_OF_RANGE`; reaching the model-entity scan cap reports `FRAME_LIMIT`, which does not count as a skipped frame. These diagnostics do not establish general frame-detection parity.

Ukrainian and Vietnamese catalogs include workspace recovery and local-conflict messages; runtime translation and browser QA remain pending.

Browser fullscreen requires a click in the application and Fullscreen API permission. Embedded iframes need `allow="fullscreen"`. If the browser refuses fullscreen, the application enters Focus mode and shows a status message. Programmatic web requests return Unsupported; desktop fullscreen uses the native viewport. Focus and fullscreen are temporary and are never saved in a workspace profile.

## V3.1 GĐ4 local storage and recovery limits

2026-10-23 · Source inventory; browser quota/crash QA is outstanding.

Workspace profiles use IndexedDB database `cadcraft`, currently `DB_VERSION = 2` after
adding GĐ5 `print-jobs`. Workspace data uses `workspace`, `snapshots`, `damaged` and `meta`;
drawings use `autosave`, `autosave_index` and `instances`. Profile sets are bounded by
`MAX_SET_BYTES` (1 MiB). Profile preload is bounded by `MAX_PRELOAD_RECORDS` (10,000) and
`MAX_PRELOAD_BYTES` (32 MiB per store read). Open/preload timeout is
`IDB_OPEN_TIMEOUT_MS` (3,000 ms). Failure falls back to localStorage, then a memory store
that refuses writes. LocalStorage fallback does not provide drawing autosave.

Fonts still use localStorage (3 MiB per font, 4 MiB total); larger fonts work for the
visit only. IndexedDB profiles no longer share that localStorage quota with fonts.
Browser persistent-storage permission is requested, but can be denied; storage can still
be cleared by the user or browser. This is local recovery, not an external backup.

Drawing autosave follows SAVETIME (0 disables it), with urgent requests on visibility
change/pagehide; browser termination may prevent the last pending transaction completing.
`MAX_WEB_AUTOSAVE_BYTES` is 64 MiB per drawing. Recovery skips live/own instances and
fetches DXF bytes on demand. A recovered drawing has no source path and stays dirty until
saved/downloaded. Drawing quota failure never evicts workspace records. Workspace quota
recovery prunes only older valid snapshots, retains the last valid snapshot, current,
damaged evidence and v1 backup, and retries; another quota failure reports an unsaved state.
Real quota exhaustion, private/blocked storage, abrupt termination, two-tab contention
and recovery after reload remain QA work; fake-backend tests are not browser evidence.

Workspace safe attachment uses SIMPLE and suppresses workspace reads, writes and sync
through `WsState`. Ending safe mode discards its edits. On the web, host IndexedDB
open/preload and localStorage migration run before `?safe` is applied, so safe mode does
not currently guarantee that browser storage is untouched during startup.

Cloud workspace synchronization is not implemented. MemoryRemote remains a test double;
`LayoutSave::SyncPending`, `Syncing` and `Synced` have no production transition. Local save
labels explicitly say unsynced. Neither remote `/print-jobs/*` APIs nor tenant job services
are implemented. Batch resume and browser download limits are in
[plotters](../plotters.md#batch-jobs).
