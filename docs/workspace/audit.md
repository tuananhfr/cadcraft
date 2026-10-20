# Workspace and personalization baseline audit

Audit date: 2026-10-20. Task: W1 / WP01, UbotCAD Workspace V2.1.

Repository HEAD: `d1c5b36d2dd852fba05a3b7e3b3679dcf5a2552a` (`git rev-parse HEAD`).
The observed branch is `tuananhfr`, although the worker assignment names `ws-v21`.
The working tree also carries uncommitted/just-committed phase-2 Fonts changes;
at inspection time they were uncommitted. This audit describes that working tree,
not HEAD alone. No branch changes or commits were made for this audit.

The starting inventory is implementation-plan section A, checked against source.
Spec references below refer to *UBOTBUILD CAD FREE — V2.1*, dated 2026-10-20.
`present` means the named capability exists in source; `partial` means only part
of the requirement exists; `absent` means the audited implementation has no such
facility. These are source findings, not runtime acceptance results. Task IDs
identify planned remediation, not completed work.

## Capability matrix

Paths are relative to this repository unless marked `craft-ui` or `ubot-build`.
Line numbers refer to the inspected working tree and may move as other tasks land.

| Spec area | Status | Verified baseline and gap | Source path:line | Plan task / boundary |
|---|---|---|---|---|
| §3 Top / Quick Access: file actions | partial | New/Open/Save and Undo are available in the fixed top toolbar. There is no independently configurable Quick Access strip with pin/unpin/reorder. | `crates/ui-egui/src/chrome.rs:12`, `crates/ui-egui/src/chrome.rs:65` | W8: Quick Access and Customize. |
| §3 Top / Quick Access: workspace selector and search | absent | No workspace preset picker or Quick Access command search. The status-bar Workspace Switching icon dispatches `ui.noop`. Command-line autocomplete is a separate existing facility. | `crates/ui-egui/src/chrome.rs:345`, `crates/ui-egui/src/menus.rs:17`, `crates/ui-egui/src/cmdline.rs:111` | W3: presets; W6: picker/switch; W8: customization/search. |
| §3 Ribbon | absent | No ribbon tabs, groups, contextual tabs, or ribbon customization in the current chrome. | `crates/ui-egui/src/chrome.rs:65`, `crates/ui-egui/src/lib.rs:647` | W8: seven tabs, contextual tabs, customization. SIMPLE remains without a ribbon (D4/D6). |
| §3 Toolbar | partial | `TOOLBAR` is a fixed icon/group list with a visibility toggle. It already handles overflow with a menu or horizontal scroll. No four-edge docking, floating, locking, or user reordering. | `crates/ui-egui/src/chrome.rs:12`, `crates/ui-egui/src/chrome.rs:98`, `crates/ui-egui/src/menus.rs:36` | W7: data-driven toolbars and gestures; W5: persistence. |
| §3 Panels: docking mechanics | partial | Four edge zones plus center/tab placement, splits, resizing, in-window floating, permissions, pane limits, drop preview, Escape cancellation, and viewport clamping exist. Auto-hide and native floating windows across monitors do not. | `crates/ui-egui/src/docking.rs:60`, `crates/ui-egui/src/docking.rs:151`, `crates/ui-egui/src/docking.rs:339`; craft-ui `crates/ui/src/docking/model.rs:13`, `model.rs:148`, `surface.rs:86`, `surface.rs:257`, `surface.rs:834`, `surface.rs:878`, `surface.rs:897`, `targets.rs:63` | W10: auto-hide; W9: geometry remap. D2 prohibits craft-libs changes; UX07 remains partial. |
| §3 Panels: available identities | partial | Canvas, ToolSets, Layers, Properties, ExternalRefs, plus on-demand QuickCalc exist. Font, LISP, and Sheet Set are not docking identities. | `crates/ui-egui/src/docking.rs:11`, `crates/ui-egui/src/docking.rs:22`, `crates/ui-egui/src/docking.rs:54`, `crates/ui-egui/src/docking.rs:346` | W10: Font/LISP panels and unknown-capability placeholders. Sheet Set implementation remains blocked (D17). |
| §3 Panels: Tool Sets customization | partial | Drafting/Modeling tabs, collapsed categories, and per-command hiding/restoration exist. This is separate from toolbar customization. Hidden tools are runtime state but omitted from the native workspace snapshot. | `crates/ui-egui/src/palettes.rs:127`, `crates/ui-egui/src/palettes.rs:198`, `crates/ui-egui/src/palettes.rs:255`, `crates/ui-egui/src/lib.rs:95`, `apps/cadcraft/src/workspace.rs:45` | W5/W7/W8: capture and customizable controls. |
| §3 Command Line: drafting input | present | Alias completion, history, autocomplete, and Escape cancellation are implemented. | `crates/ui-egui/src/cmdline.rs:111`, `crates/ui-egui/src/cmdline.rs:305`, `crates/ui-egui/src/cmdline.rs:398`, `crates/ui-egui/src/cmdline.rs:632` | W8/W9 preserve these behaviors; W13 verifies acceptance. |
| §3 Command Line: placement | partial | The bar is drawn at a fixed bottom-center canvas position. Dragging adjusts history line count; it does not move or dock the bar. | `crates/ui-egui/src/cmdline.rs:398`, `crates/ui-egui/src/cmdline.rs:589`, `crates/ui-egui/src/cmdline.rs:598` | W9: dock/float/height; W5: persistence. |
| §3 Status bar | partial | Drafting toggles, coordinates, annotation scale, and viewport scale/lock exist. Profile sysvars preserve drafting settings; native workspace preserves bar visibility. Web preferences omit bar visibility. Workspace Switching is a no-op; workspace save/sync status is absent. | `crates/ui-egui/src/chrome.rs:298`, `crates/ui-egui/src/chrome.rs:332`, `crates/ui-egui/src/chrome.rs:350`, `crates/ui-egui/src/chrome.rs:362`, `crates/ui-egui/src/chrome.rs:377`, `crates/engine/src/cmd/options.rs:15`, `apps/cadcraft/src/workspace.rs:51`, `crates/ui-egui/src/lib.rs:371` | W6: selector/status; W9: status persistence; W11: simulated sync states. |
| §3 Canvas | partial | Pan, wheel/pinch zoom, selection, and crosshair are implemented. Canvas is protected against close/float/tab sharing. Floating panels and the command line can overlay drawing space; full no-inaccessible-control behavior at all required viewports/DPI is not established by this audit. | `crates/ui-egui/src/canvas.rs:1226`, `crates/ui-egui/src/canvas.rs:1290`, `crates/ui-egui/src/canvas.rs:1302`, `crates/ui-egui/src/canvas.rs:1411`, `crates/ui-egui/src/canvas.rs:1843`, `crates/ui-egui/src/docking.rs:108` | W9: scale/remap; W10: CAD integrity; W13: viewport/DPI QA. |
| Cross-cutting §4–7: native persistence | partial | `workspace.json` version 1 has a 64 KiB cap, validation, temp-file write, `sync_all`, and rename. Snapshot changes trigger writes without debounce. Invalid/newer files are preserved and writes disabled; an invalid dock subtree can fall back independently. No versioned snapshot history, layered policy, or migration pipeline. | `apps/cadcraft/src/workspace.rs:12`, `workspace.rs:34`, `workspace.rs:64`, `workspace.rs:100`, `workspace.rs:118`, `workspace.rs:134`, `workspace.rs:149`; `apps/cadcraft/src/main.rs:54`, `main.rs:90` | W3: schema; W4: migration/backup; W5: store, debounce, snapshots, policy/history. |
| Cross-cutting §4–7: web persistence | partial | The eframe save hook writes profile/options and interface preferences, including phase-2 font settings, but not docking/layout. It uses eframe's default 30-second autosave interval; there is no workspace drag-end commit. | `apps/cadcraft-web/src/main.rs:53`, `main.rs:135`; `crates/ui-egui/src/options/mod.rs:307`; `crates/ui-egui/src/lib.rs:371`; eframe 0.36.2 `src/epi.rs:234` | W5: localStorage store shared through the client profile abstraction; W11: fake remote. |
| Cross-cutting §5: command registry | partial | `CommandSpec` has id, label, menu, shortcut, aliases, parameters, enabled/run handlers, undoable, interactive, and transparent flags. Category, capability, permission, and risk metadata are absent. UI commands are a separate list. The current list has 56 entries, not section A's 57; engine registry count was not runtime-enumerated. | `crates/engine/src/cmd/mod.rs:72`, `mod.rs:185`, `mod.rs:218`; `crates/ui-egui/src/menus.rs:17` | W2: derived metadata/stable-ID snapshot/mapping. Existing engine IDs such as `line` remain authoritative. |
| §3 Command Line / §9: shortcuts and aliases | partial | Keyboard shortcuts are hard-coded. ALIASEDIT/REINIT use a process-local alias map and remember the loaded file for that session; no workspace alias/shortcut persistence. | `crates/ui-egui/src/menus.rs:597`, `crates/engine/src/cmd/file/pgp.rs:25`, `pgp.rs:33`, `pgp.rs:125` | W8: user bindings, conflicts, alias capture; W5: storage. |
| §3 / §9 Accessibility | partial | Both hosts enable AccessKit; custom controls supply widget information and tab roles. Keyboard focus tests cover toolbar, Tool Sets, and document tabs. No persisted `ui_scale` option exists. Full keyboard customization, 200% zoom, and IME acceptance are unverified. | `apps/cadcraft/Cargo.toml:17`, `apps/cadcraft-web/Cargo.toml:30`, `crates/ui-egui/src/icons.rs:934`, `crates/ui-egui/tests/workspace_focus.rs:37`, `workspace_focus.rs:55`, `workspace_focus.rs:84`, `crates/ui-egui/src/lib.rs:72` | W8: keyboard/accessibility; W9: `ui_scale`; W13: QA. |
| §3 Panels / §8 Font integration | partial | Phase-2 Fonts dialog, `fonts` command, font substitutions/fallbacks, user font management, and host persistence are present in the working tree. This is not a dockable panel or tenant Font Registry. | `crates/ui-egui/src/dialogs.rs:32`, `crates/ui-egui/src/fontmgr.rs:18`, `fontmgr.rs:54`, `crates/engine/src/cmd/fonts.rs:18`, `fonts.rs:50`, `crates/ui-egui/src/options/mod.rs:311` | W10: dockable Font panel; F1–F6: SHX track. Tenant Font Registry blocked pending source/backend. |
| §3 Panels / §8 LISP integration | partial | APPLOAD and LISP runtime exist with cell/string/stack/depth limits. Startup files come only from configured support, not the drawing folder. Native explicit file reads/writes are not confined to a sandbox root. There is no dockable LISP panel. | `crates/engine/src/cmd/lisp.rs:12`, `lisp.rs:35`, `crates/engine/src/lisp/startup.rs:16`, `startup.rs:32`, `crates/engine/src/lisp/value.rs:44`, `value.rs:48`, `crates/engine/src/lisp/eval.rs:163`, `crates/engine/src/lisp/builtins.rs:964`, `builtins.rs:978` | W10: session-loaded-file panel; W3/W5: reject executable profile payloads; W12: threat model. Native path sandbox is a separate blocked security track. |
| Cross-cutting §6: web bridge | partial | Same-origin parent bridge supports `cadcraft:ready/open/save/dirty/failed`, checking both origin and sender. This transfers drawings; it is not workspace-profile synchronization. | `apps/cadcraft-web/src/host.rs:1`, `host.rs:45`, `host.rs:69`, `host.rs:99`, `host.rs:106` | W11: MemoryRemote only; W12: API contract. Real host/API wiring remains out of scope. |

The craft-ui paths above were read from the local dependency checkout at revision
`5c874d51eeb3de70e735c8ce87a59a7dedd47675`, pinned by `Cargo.toml:51`.
The eframe default interval was checked in the locally cached 0.36.2 source.
No dependency files were changed.

## Out of repo

Deployment facts here are the controller's section-A baseline, corroborated where
possible by local `G:/workspace/ubot-build` documentation/configuration. No live
deployment, authentication, database, or tenant-isolation test was performed.

| Capability | Status | Evidence / boundary | Plan handling |
|---|---|---|---|
| ubotbuild.com backend and PI ID | absent | The current UbotBuild deployment is static-only on Apache; PI ID login is a mock, not a deployed authentication service. `ubot-build/CLAUDE.md:9` and `:61` describe the deployed frontend; `ubot-build/frontend/app/src/pages/LoginPage.tsx:33` handles login by navigation alone. | D1: no server code. W12 documents the contract; UX10 remains blocked. |
| Drupal ERPCons | partial | Drupal 11 exists in `ubot-build/backend`, with session/CSRF, roles, company taxonomy, and `/api/v1/...`; it is not deployed for ubotbuild.com. `ubot-build/CLAUDE.md:10`, `:11`, `:22`, `:30` describe the separate ERP app and missing DB/settings. `backend/web/modules/custom/erp_api/erp_api.routing.yml:527` provides a role gate and `:542` a CSRF requirement. This does not establish a working workspace-profile backend. | Need backend host, DB/test environment, proxy/CORS, and real identity decisions before server implementation. |
| Existing workspace API name | present | `ubot-build/backend/web/modules/custom/erp_api/erp_api.routing.yml:517` identifies NAS browsing; `:522` maps `/api/v1/workspace`, with upload and other NAS operations below it. It is not the spec's CAD profile resource. | W12 documents the collision and proposes `/api/v1/cad-workspaces`; that prefix remains a proposal requiring approval. |
| Host embedding | absent | UbotBuild opens `/cad/` in a new tab, rather than embedding the bridge in an iframe (`ubot-build/CLAUDE.md:56`). | Real remote/iframe integration requires a host architecture decision; W11 tests only MemoryRemote. |
| LPC company preset and business registries | absent | LPC source, Master Layer Registry, tenant Font Registry, and Sheet Set integration are not supplied for this plan (spec §14 / plan B). Existing drawing palettes do not establish those services. | No LPC_COMPANY preset shipped. W10 preserves unavailable IDs via placeholders; W12 documents server debt. |

Server ETag/412 handling, tenant filtering, RBAC/ABAC enforcement, signed company
presets, publish audit logs, and the eight §6 endpoints are not delivered by the
existing drawing bridge or local workspace file. W12 is documentation only;
W11's MemoryRemote cannot prove production authorization or synchronization.

## Evidence and acceptance limits

Screenshots: pending (added by controller after the first trunk build)

The controller will add the capture date and scratchpad filenames for Start,
Drawing with Tool Sets, and Options at 1366×768. No screenshots or screenshot
assets were created or committed by W1.

This is a documentation-only source audit. No Cargo commands, runtime tests,
screenshots, performance measurements, or UX01–UX16 acceptance runs were executed.
W13 owns the acceptance matrix and SLO evidence; native two-monitor testing,
crash/restart video, and human sign-off remain required separately. In-window
floating is not evidence of native multi-monitor support.
