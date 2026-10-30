# UbotBuild changes

This branch (`tuananhfr`) is UbotCAD, the UbotBuild build of [CADCraft](https://github.com/storytold/cadcraft) 0.8.0, based on CADCraft by the ArtCraft team, licensed MIT OR Apache-2.0 (see `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`). Changes from upstream (Apache-2.0 §4b):

- Removed the ArtCraft logos and marks (`docs/brand/` and its `ATTRIBUTION.md` rows) and every link to the ArtCraft Discord and getartcraft.com, as the ArtCraft brand terms require for modified versions: About dialog, the macOS app-menu Discord item and its `ui.discord` handler, README logo, badges, community and Crafting Apps sections, and the Linux/Windows packaging links (now https://ubotbuild.com/).
- About now reads "UbotCAD — dựa trên CADCraft của nhóm ArtCraft." ("UbotCAD — based on CADCraft by the ArtCraft team.") and links ubotbuild.com.
- Vietnamese text:
  - the stroke font draws the 134 Vietnamese marked letters (its letters plus the tone marks, breve, circumflex, horn and the bar of Đ);
  - a text style keeps the TrueType family of its `ACAD` xdata (`TextStyle::family`), and text typed for a legacy 8-bit Vietnamese font (TCVN3 `.Vn*`, VNI `VNI-*`, or a `vn*`/`vh*` file) is shown in Unicode when neither the font file nor the family is installed (`cadcraft_fonts::LegacyVi`). This is display only, so the saved text is unchanged;
  - the web build embeds Noto Sans Regular from our craft-fonts fork as its first interface font, since egui's own faces have no Vietnamese. Desktops keep using an installed system font. Build the web app with `CRAFT_FONTS_DIR` set to a checkout of [tuananhfr/craft-fonts@b7dfde9](https://github.com/tuananhfr/craft-fonts/tree/b7dfde98c6d9d5db761c405732f9ef6d58bec192) (branch `fonts/noto-sans`); without it the interface falls back to egui's faces.
- Dropped the translations of the removed strings from every catalog.
- DWG opens and saves in the web build too. Upstream builds the DWG bridge (`crates/dwg`, acadrust) for the desktop only, though it works from bytes in memory and acadrust builds for wasm32; the web build now includes it (about 1 MB more brotli-compressed wasm). To fit large DWGs in wasm32's 4 GB:
  - a picked or dropped file is read directly (`cmd::file::read_data`) instead of through OPEN's base64 `data`;
  - the DWG→DXF text is collected in 8 MB pieces and joined after acadrust's document is freed;
  - the DXF tag list is sized once from a count of the tags (not the lines, so blank lines or junk after EOF reserve nothing), and the DXF text is freed before the drawing is built from the tags.

  An 86 MB DWG (17,000 entities, 3,055 layers) now opens in the browser in about 70 s at a 3.3 GB peak; before these changes it crashed the tab. A DWG over 90 MB is refused with a message to use the desktop app: by the file picker before reading (`MAX_OPEN_DWG_BYTES`), and by the reader on the web whatever the path (`cadcraft_dwg::Limits::WEB`).
- Fonts (Tools ▸ Fonts..., the `fonts` command), on the desktop and the web:
  - the dialog lists the fonts the open drawing's text styles ask for, which of them this computer lacks, and lets the user pick what to draw instead; a drawing that opens with missing fonts in use says so on the command line. The replacement table (keyed by font file name), FONTALT and FONTFALLBACK are user settings saved in the profile; they only decide what is drawn and never change the drawing;
  - the user can add font files (`cadcraft_fonts::ttf::add_user_font`, found by file name or by family): the desktop copies them to `<settings dir>/fonts`, the web keeps them in localStorage (3 MB per font, 4 MB in all; a larger one works for that visit only). Fonts never leave the computer. The web build also lists installed fonts where the browser has `queryLocalFonts` (Chromium);

- V2.1 workspace client (acceptance and release gate pending; see `docs/workspace/acceptance.md`):
  - nine presets, layered profiles/policy, versioned local persistence, first-run picker, clone/save/reset/import/export, migration and layout undo; four-edge/floating toolbars, ribbon/context tabs, Quick Access, favorites, Customize, shortcuts and aliases;
  - command-line docking, status-item persistence, interface scale and screen remap; dockable Font/LISP panels and auto-hide. Floating controls stay inside the main window; discipline presets await CAD Manager review;
  - offline/conflict sync is exercised through MemoryRemote only. OpenAPI/schema, security notes and migration documentation are present; real HTTP, tenant authorization, signed publication, LPC and business registries remain outstanding. Performance numbers, browser reload/quota/DPI captures and human sign-off are pending.
- SHX drawing fonts (user-supplied files; no SHX assets bundled):
  - shape/unifont readers and drawing-codepage glyph lookup, including legacy `%%c`/`%%d`/`%%p` symbol slots; added files feed the existing font selection/rendering path;
  - bigfont is unsupported; six known files are rejected as truncated. Output has not been black-box compared with AutoCAD, and Vietnamese precomposed letters are not decomposed under cp1258.
- Desktop builds are UbotCAD (names, settings directory, icon and a new UpgradeCode); `ubotcad-desktop.yml` builds unsigned Windows installers and releases them on `ubotcad-v*` tags. The app and web pages call themselves UbotCAD in every UI language; file formats, plotter names, fonts and MCP keep CADCraft identifiers. The macOS icns is not updated.
- Since 0.10.0 the released Windows installers are built with the `licensing` feature: drawing and editing need a PI ID sign-in and a device lease from production PI (`https://pi-platform.tekshot.vn`, lease key `lease-2026-10` pinned in erpcons-account 0.4.2); opening, viewing, saving and exporting stay free. The web build has no licence gate.

## V2.2/V3.1 GĐ0

2026-10-22 · Fixes T1–T8 of the V2.2/V3.1 plan's release-blocker phase.

- Browser keys (web): defaults and saved-profile dispatch exclude browser-reserved keys, the shortcut editor refuses them with "The browser uses this key", and menus hide their hints. This includes F5/Ctrl+E for ISOPLANE, Shift+F10 and Ctrl+Shift+C; F8/F9 remain available. Replacement defaults wait until GĐ3; desktop bindings are unchanged.
- Leaving unsaved drawings (web): standalone pages request the browser's standard beforeunload confirmation when any open drawing is dirty; clean drawings do not request it. A prior user gesture is required. In `?host=parent` mode the parent remains responsible through dirty reporting. This does not provide drawing autosave.
- Customize undo/redo (web and desktop): routes Ctrl+Z, Ctrl+Shift+Z and Ctrl+Y to layout undo/redo while Customize is open, including when a page button keeps focus. A focused text field keeps those keys; other shortcuts remain blocked while any Customize widget has focus. Outside Customize, drawing undo/redo keeps its existing behavior.
- File delivery (web): PLOT, PUBLISH, EXPORT, PDFBOOK, plotter configuration saves and workspace export use the shared file writer to deliver a named file with its MIME type instead of rejecting paths. Standalone pages download files; embedded mode uses host delivery. PUBLISH downloads each output separately, with no zip in GĐ0. Desktop keeps its temporary-file-and-rename writes.
- Workspace import (web): the existing `pick_open_kind(&FileKind)` API routes `WORKSPACE_FILE` selections to a JSON upload and `ui.workspace.import`; invalid UTF-8 or oversized uploads are rejected. Drawing uploads still use `open_bytes`. DXF IMPORT still opens a new drawing rather than inserting it; GĐ0 does not change that behavior.
- Apply to Layout (web and desktop): the Plot dialog now retains the device, plot-style table, paper or custom width/height, margins, orientation, centering, fit or explicit scale, lineweights, display plot styles and transparency setting when applying page setup.
- PDFBOOK omissions (web and desktop): results include BLOCK issues for unplottable frames (`FRAME_PAPER_MISMATCH`, `FRAME_OUT_OF_RANGE`) and the model-entity scan cap (`FRAME_LIMIT`). Issues identify frame windows and sizes where applicable; the summary counts omitted frames, excluding the scan-limit issue. With no printable frames, the error lists up to ten issues and counts the rest. The requested-height filter is retained: other-height outlines are ignored; only requested-height frames without matching paper get `FRAME_PAPER_MISMATCH`.
- Damaged workspace profiles (web and desktop): startup tries up to 20 snapshots newest first, skips invalid copies and restores the latest valid one with a revision notice, before falling back to SIMPLE. The damaged bytes are retained; if preservation fails, writes wait until they can be kept. The next commit rewrites the restored set. A newer-major profile still falls back to SIMPLE without snapshot recovery.
- Concurrent workspace writers (web and desktop): commits compare the stored revision before writing. A stale writer reports "Changed in another tab" and stops automatic retries; UI/autosave offers "Keep my layout" or "Use the other tab's layout" through `ui.workspace.resolve`. Programmatic conflicts return an error without opening a dialog. Web storage events adopt external changes when no local edits are pending and report "Updated from another tab". This is local conflict handling; the localStorage check and write are not atomic until GĐ4 IndexedDB transactions.

## V2.2 GĐ1A

- UBPLOT selection follows AutoCAD: Shift removes, Enter finishes, Esc cancels; this replaces the spec’s “Ctrl add/remove, ESC finish”.

## V3.1 i18n và registry

- Added Tiếng Việt with saved locale selection, live switching, U-F ribbon terminology and glossary-checked translations. Ukrainian and Vietnamese are required for UI catalogs; Vietnamese also covers discovered engine prompts. The catalog includes Quick Plot and 31 remaining ribbon labels (1,831 rows total); native review pending (user/LPC).
- Registry metadata exposes stable namespaced IDs, label keys, contexts, platform availability and version. Legacy command IDs remain accepted; dispatch/control results carry outcome codes. Programmatic calls retain their JSON behavior.
- Command Palette shares accent-insensitive Vietnamese/English/alias search with Command Reference, Quick Access and Customize. Vietnamese/Latin-1 normalization and IME guards protect typing; desktop Ctrl+Shift+P opens the palette, while web uses the UI without that reserved shortcut.
- Added temporary qps pseudo-localization and a seven-screen Vietnamese catalog-text regression at 1366×768, including Quick Plot. Plot translates the built-in Model label while preserving user layout names. Engine history, diagnostics and CLI/MCP output remain English.
- T13 source work is complete; native/wasm tests, build gates, browser visual QA (including 200% zoom), desktop smoke and native terminology/IME review remain with the lead. No execution pass is claimed here.

## V2.2 GĐ2

Paper, physical scale, preflight and audited approval (B1–B9). Gate 2 results are below; the benchmark and one 200% re-check remain open.

- UBPLOT ratios use `INSUNITS`, independently of `MEASUREMENT`: `1:100` in a metre drawing prints 1 m as 10 mm. `scale_label` preserves the requested or frame-derived physical ratio. Unitless and unknown codes assume millimetres with INFO `UNITS_ASSUMED`. PLOT and EXPORTPDF retain their legacy paper-units-per-drawing-unit meaning unless `scaleMode: "real"` is supplied.
- PLOT and EXPORTPDF with an explicit `scale` and no `fit` now preserve that scale on Model instead of silently fitting. PDF page items and PNG use the same default; `fit: true` remains an explicit request to fit. UBPLOT Ratio sheets always publish with `fit: false`; only Fit sheets are fitted. Paper/scale mismatches block instead of shrinking the drawing.
- Paper strings accept `auto`, standard A0–A5 and elongated names such as `A1x4`, custom `841x2400` mm, and `roll:914` or `roll:914:5`, including per-sheet overrides. Roll length follows the scaled frame plus both margins, rounded up to 1 mm. Too-narrow rolls block with `PAPER_TOO_SMALL`; oversized rolls use `PAPER_TOO_LARGE`; fixed-paper mismatches use `SCALE_PAPER_MISMATCH`. Merged pages retain their own MediaBox and scale.
- PDF limits are 1000 pages per file, 512 MiB and 15000 mm (15 m) per paper edge. The two-pass writer gathers glyphs separately and releases each page display list after writing it. Edges over 5080 mm (14400 points) use `/UserUnit`, scaled MediaBox/content and catalog PDF 1.6. Oversized paper is rejected; merged jobs over the page cap suggest split output. PNG retains its pixel cap.
- Preflight reports essential missing xrefs as BLOCK `XREF_MISSING`, unloaded xrefs as WARN `XREF_UNLOADED`, and missing xrefs outside all sheets as plan-level INFO `XREF_MISSING_OUTSIDE`. Nested transforms, visibility and layout viewport model bounds are considered. Empty definitions without usable bounds remain plan-level diagnostics; traversal limits block with `XREF_SCAN_LIMIT`. Web messages explain that external references are not loaded there.
- Fonts used on planned sheets produce plan-level WARN `FONT_MISSING` / `FONT_FALLBACK` or INFO `FONT_MAPPED`; cached `last` scans refresh font settings. Duplicate ids warn on every copy (`SHEET_ID_DUPLICATE`). The revision register uses natural order: old/missing revisions warn (`REVISION_OLD`), newer revisions report INFO (`REVISION_NEWER`), and superseded templates warn (`TEMPLATE_SUPERSEDED`). `PLOT_STYLE` and renamed split deliveries (`FILE_RENAMED`) are INFO. The full code/severity table is in [UBPLOT](docs/ubplot.md).
- Clean and INFO-only sheets are `READY`; unapproved WARNs are `NEEDS_PREVIEW`; BLOCKs are `BLOCKED`. Scan summary counts sheet verdicts under `pass`, `info`, `warn`, `block`; plan-level issues do not inflate those counts. The dialog defaults to exceptions, groups by code/severity, offers confirmed bulk WARN approval, and disables Publish for unapproved warnings or BLOCKs in merged output. BLOCK groups offer fixes or exclusion and have no approval control.
- `ubplot.approve` atomically approves WARNs for `user` or named agents and appends fingerprinted audit records with host timestamps. Only WARNs can be approved; BLOCKs can never be approved, including via JSON or an agent. Unchanged rescans preserve approvals and audit history; changed messages require approval again. Publishing rejects forged flags and unapproved warnings with `BLOCK_NOT_APPROVABLE`, `APPROVAL_WITHOUT_AUDIT` or `WARN_NOT_APPROVED`; there is no bypass.
- Device limits use the greatest short and long paper sides: at least 95% warns (`DEVICE_LIMIT_NEAR`), exceeding either blocks (`DEVICE_LIMIT_EXCEEDED`). A split proposal requires explicit approval through `ubplot.approve {split}` and a geometry/device-bound audit entry; this resolves the device-size cause rather than approving a BLOCK. Overlap defaults to 20 mm (0–200 mm). Approved pieces preserve scale/rotation, follow sheet/piece order, use numbered page labels, and show 0.25 mm dashed seams with 10 mm end ticks. A short side beyond the device width stays blocked; choose PDF.
- UBPLOT on the web publishes PDF only. Unknown desktop devices and non-PDF web devices fall back to PDF after WARN `DEVICE_UNAVAILABLE` approval; another printer is never selected silently. Desktop UI offers overlap and split approval. Real printers, driver queries, queues/resume and folder publishing remain GĐ5 work; LPC templates and PrintProfile remain GĐ6 work.

### Gate 2 results (lead)

2026-10-23 · Lead results. The benchmark was not run; 200% web re-check of the Exceptions fix is pending.

| Check | Result / evidence |
| --- | --- |
| `sp05_mixed_a1_a3_long_in_one_pdf_each_own_mediabox` | Pass — Linux gate `4ea3c75`, run 39874361985 |
| `roll_914_length_follows_frame` | Pass — Linux gate `4ea3c75`, run 39874361985 |
| `sheet_15m_uses_user_unit` | Pass — Linux gate `4ea3c75`, run 39874361985 |
| `pages_with_scale_are_not_fitted` | Pass — Linux gate `4ea3c75`, run 39874361985 |
| `insunits_meters_plot_1_100` | Pass — Linux gate `4ea3c75`, run 39874361985 |
| SP07: `essential_missing_xref_blocks_the_sheet` | Pass — Linux gate `4ea3c75`, run 39874361985 |
| SP07: `font_missing_warns_at_plan_level` | Pass — Linux gate `4ea3c75`, run 39874361985 |
| SP11: `duplicate_sheet_ids_warn_on_every_copy` | Pass — Linux gate `4ea3c75`, run 39874361985 |
| SP11: `old_revision_against_register_warns` | Pass — Linux gate `4ea3c75`, run 39874361985 |
| `block_is_never_approvable_even_by_json` | Pass — Linux gate `4ea3c75`, run 39874361985 |
| Native/wasm clippy, fmt, affected crate tests, layers | Pass — Linux gate `4ea3c75` (run 39874361985); Windows desktop job green at `ff1b42d` (run 39831171062), cancelled at `4ea3c75` when Actions blocked the repository |
| `bench_thousand_dense_pages`: bytes/page, total bytes, time | Not measured — the test is `#[ignore]` and was not run; no numbers are claimed |
| Independent PDF CLI checks on mixed-size and 15 m output | Pass (pypdf strict; qpdf/mutool not installed) — web output: A3 420×297 mm and 841×2400 mm pages with labels S-002/S-003; `ROLL15` page 3 has UserUnit 3 → 841×15000 mm |
| Web release build and visual QA: 1366×768, 1920×1080, 200% zoom | Partial — artifact `39b8b32`: 1366×768 Block ALL → 3 sheets, `XREF_MISSING` BLOCK disables Publish, excluding it publishes 2 pages; 1920×1080 `ROLL15` publishes 3 pages. At 200% the Exceptions body was not visible and did not scroll; fixed in `bedb169`, re-check pending |
| Desktop smoke: approved three-piece split and PDF inspection | Pass — Windows artifact `459b1a2`: A1-only device, 594×2400 frame → `DEVICE_LIMIT_EXCEEDED`, split panel "3 pieces", Approve split clicked → no BLOCK, audit records the split; merged PDF has 3 pages 594×813.3 mm labelled S-001 (1/3)…(3/3). The UI Publish button (native save dialog) was not clicked |

## V3.1 chrome, focus, keys

2026-10-23 · GĐ3 source changes with scoped web QA and Windows desktop smoke recorded
below. Evidence and remaining checks are in `docs/workspace/acceptance.md`.

- Ribbon modes/flyout (desktop/web): Expanded, Compact, TabsOnly and Hidden, height fallback and a bounded scrolling flyout; `crates/ui-egui/src/ribbon/render.rs` and `src/wsprofile/schema.rs`.
- Tab overflow and menus (desktop/web): `»` keeps overflow tabs reachable; context controls hide/show tabs, select modes, open Customize and reset only the ribbon; `crates/ui-egui/src/ribbon/render.rs` and `src/ribbon/mod.rs`.
- Ribbon ordering and toolbar conversion (desktop/web): Customize supports drag or Alt+Up/Down command ordering; dropping a toolbar creates a group, hides its source and supports layout undo; `crates/ui-egui/src/customize/ribbon_tab.rs` and `src/toolbars/drag.rs`.
- Quick Access (desktop/web): pin/unpin from toolbars and Tool Sets, drag or Alt+Left/Right ordering, persisted title/above/below placement independent of menu visibility; `crates/ui-egui/src/quick_access.rs`.
- Fullscreen (desktop/web): desktop uses native viewport requests with Ctrl+Shift+F; web needs a click and Fullscreen API permission, falls back to focus on refusal and reports Unsupported for programmatic calls; `crates/ui-egui/src/fullscreen.rs` and `apps/cadcraft-web/src/fullscreen.rs`.
- Focus mode (desktop/web): temporary hidden chrome with a floating/hidden command line, Escape priority and exact layout restoration; desktop Ctrl+0, web menu/button entry; `crates/ui-egui/src/focus.rs`.
- Density/contrast (desktop/web): Compact, Normal and Comfortable metrics, ≥32-point sampled Comfortable controls and light/dark text/focus contrast assertions; `crates/ui-egui/src/theme.rs` and `src/wsprofile/schema.rs`.
- Web key table: Properties Ctrl+Shift+1, Command Line Ctrl+Shift+9 and Help Shift+F1, with browser-reserved defaults/profile bindings filtered; desktop keeps its bindings plus the focus/fullscreen entries; `crates/ui-egui/src/shortcuts.rs`.
- Alias editor (desktop/web): ASCII validation, conflict confirmation, keyboard entry, reset and JSON import/export; `crates/ui-egui/src/customize/alias_tab.rs`.
- Block context and replacement (desktop/web): INSERT-only context with the same commands for U-BOT; `blockreplace` preserves transforms, matches attributes by tag and rejects xrefs/locked layers; `crates/ui-egui/src/ribbon/contextual.rs` and `crates/engine/src/cmd/blocks/replace.rs`.
- Blocks/History panels (desktop/web): dockable block insertion and command replay, saved placement and legacy Blocks ID; F2 retains command-line history expansion; `crates/ui-egui/src/docking.rs`, `src/dialogs.rs` and `src/cmdline.rs`.
- CAD Manager/Minimal presets (desktop/web): local management layout and TabsOnly canvas layout; SIMPLE stays the safe fallback, with no company authorization service; `crates/ui-egui/src/wsprofile/presets.rs` and `src/wsprofile/presets/{CAD_MANAGER,MINIMAL}.json`.
- Screen remap (desktop/web): proportional dock/floating geometry with split/panel clamps on resize/scale changes; desktop floats remain inside one main window, browser owns web monitor placement; `crates/ui-egui/src/wsprofile/apply.rs` and `src/dock_clamp.rs`.

### QA 2026-10-23

Web QA used integration wasm artifacts in headless Chromium, real device-scale factors 1/2,
EN/VI: first pass `0055a51`, re-check `4ea3c75`. Linux gate `4ea3c75`, run 39874361985,
was green (fmt, clippy -D warnings, tests, wasm). Its Windows desktop CI job was cancelled
when GitHub Actions blocked the repository ("repository invocation blocked"); there is
no Windows CI evidence. Desktop smoke used a local `cargo build --release -p cadcraft`
at `d490620` on Windows 11, isolated `CADCRAFT_CONFIG_DIR`, a control port and
`ui.screenshot`, at 1366×768 logical / 150% display scale.

| Check | Result / evidence |
| --- | --- |
| Web ribbon and chrome | Four modes/flyout at 1366×768, 1920×1080 and 200% (683×384 CSS, DPR 2); Display Mode lists all four. Compact text buttons and ‹ › paging reach Clipboard at 1366. Density modes, Block context and alias page render; Customize has four columns and lists ≥5 rows; Options closes on Esc. |
| Focus / fullscreen | Web Focus exit is top-centre, clear of ViewCube; Esc restores layout. Same-origin click enters browser fullscreen; cross-origin iframe without `allow="fullscreen"` falls back to Focus with a blocked status. Desktop fullscreen covers 2560×1600 px; Esc returns to the window. Desktop Focus hides chrome and Esc exits; TabsOnly is accepted. |
| Vietnamese layout | Checked at 1366/900/600/450 px; Properties chips/selection combo do not overlap at 900. Pre-existing untranslated Properties labels, missing CJK/Arabic language-name glyphs, clipped menu and overlapping status bar at ≤600 px remain. |
| Open observations / remaining QA | Default desktop workspace has no ribbon tabs: TabsOnly/Compact show empty strips (~40/95 px). Physical monitor removal / OS relocation (TC15, UX07, H13), real Telex/VNI typing, screen readers and browser-owned fullscreen Esc remain unverified; headless input cannot deliver the latter. Other row-specific checks remain in acceptance. |

## V3.1 GĐ4: local persistence v3

2026-10-23 · Source inventory with recovery, migration and safe-mode smoke recorded below;
real browser quota exhaustion and concurrent-tab race QA remain pending.

- Workspace sets and exports write numeric `schemaVersion: 3`, camelCase schema keys and
  unchanged snake_case enum values. Canonical SHA-256 checksums cover stored sets, snapshots
  and exports. Readers migrate 1.x profiles, retain unknown fields and keep the exact v1
  original before replacing it. `ui.workspace.rollback` restores that original and suspends
  workspace writes until restart; see the [migration runbook](docs/workspace/migration-runbook.md).
- Web workspace storage preloads an in-memory mirror from IndexedDB database `cadcraft`.
  A commit checks the stored revision and writes current, snapshot and pruning in one
  transaction. Accepted writes remain pending until the transaction completes. Quota recovery
  retries after pruning older valid snapshots, retaining current, the latest valid snapshot,
  damaged evidence and the migration backup. The old localStorage current key is retained
  for one release. IndexedDB failure falls back to localStorage, then a store that reports
  failed saves if localStorage is unavailable.
- A Device layer between User and Session owns interface scale, screen and layout geometry,
  including the dock tree. Exports omit it; imports keep this device's geometry. Drawing and
  layout save states have separate status labels; workspace changes debounce for 600 ms.
- Workspace safe mode uses SIMPLE without workspace-store reads or writes through `WsState`:
  desktop `--safe-mode`, web `?safe`, or `ui.workspace.safemode {on: true}`. Ending it discards
  temporary layout edits and reloads the stored workspace. Web host preload/migration still
  precedes safe attachment; browser storage isolation needs further QA.
- With IndexedDB available, web dirty drawings autosave as DXF through the existing SAVETIME
  planner, independently of profiles (64 MiB per drawing). Visibility/pagehide events request
  urgent saves. Drawing Recovery offers stale-instance drawings; recovered drawings are dirty
  and have no source path. `ui.recovery.status` reports drawing and workspace recovery separately.
- `ws_sync` uses user-owned `/command_line/lines` in place of device-only `/ui_scale` for its
  remote conflict probes; `ui_scale_is_not_synced` covers local scale retention. These are
  MemoryRemote tests, not a cloud-sync implementation. Fonts still use localStorage; real
  quota exhaustion and concurrent-tab races remain unverified. Scoped crash recovery is
  recorded below. Test mappings are in
  [acceptance](docs/workspace/acceptance.md#v31-gđ4-local-persistence-source-inventory).

### QA 2026-10-23

Web `0055a51` and Windows `d490620` smoke supplement the Linux gate recorded under GĐ3.

| Check | Result / evidence |
| --- | --- |
| Separate status labels | Untruncated at EN 1366/1920 and VI 1366/200%. |
| Web recovery / safe mode / import | After a renderer crash with heartbeat older than five minutes, Drawing Recovery lists the autosaved drawing; layout restores separately. `?safe` shows the safe preset and unsaved-layout notice. Import opens a JSON-pointer changes preview with Import/Cancel before applying. |
| Desktop migration / rollback / safe mode | `aged-5y.json` migrates 1.0.0 → schemaVersion 3 with byte-identical `backup-v1.json`. Rollback returns `{"restored":"1.0.0"}`, restores exact original bytes and reports saving suspended until restart. `--safe-mode` reports Safe mode; a ribbon change leaves the stored current set untouched. |
| Remaining QA | Real concurrent-tab IndexedDB races, quota exhaustion, transaction abort/fallback, browser startup storage isolation and asynchronous rollback durability remain unverified. Recovery edge cases remain in acceptance. |

## Smart Plotter: batch jobs

2026-10-23 · GĐ5 queue, workers, Folder mode and CLI source coverage, with scoped runtime
QA below. T12 adds desktop printer discovery and checked queue submission with PDF
fallback; physical-printer QA and T13 benchmarks remain pending (T13 is not merged).

- `ubplot.publish {background: true}` queues a persisted job and returns `jobId`/`QUEUED`;
  the default synchronous publish retains its existing result. Desktop runs a worker thread;
  web advances at most one sheet per frame and commits progress to IndexedDB `print-jobs`.
  Interrupted UI jobs require Resume or Discard; web resume requires the same source drawing.
- Manifests record ordered sheet status, attempts, checksums, output file/page indices and
  delivery destinations. Resume reuses valid completed artifacts and skips recorded deliveries;
  missing or changed undelivered artifacts are rebuilt. Failed sheets retry within the job's
  limit; a file group containing failures is withheld while other groups can be delivered.
- `ubplot.jobs` lists, inspects, resumes, cancels and removes jobs. Desktop stores them under
  `<settings dir>/jobs`; CLI defaults to `<OUT directory>/.ubplot-jobs`. Folder mode bounds
  recursive DWG/DXF discovery, records skipped paths/reasons and rereads source hashes before
  rendering. `folder.json` selects its file loader on resume.
- `cadcraft-cli ubplot` accepts a drawing or `--folder`, output grouping and job-store options;
  `--templates FILE` supplies a strict registry for that run only. CLI has no printer host.
  See [plotters](docs/plotters.md#batch-jobs) for the manifest excerpt, limits, storage and
  CLI syntax; no batch performance numbers are claimed.
- `ubplot.printers {refresh?: false}` lists desktop printers with a 60-second discovery cache.
  System-printer papers feed existing device checks; built-ins and plotter files keep precedence.
  Background publish records the system printer. Missing/offline devices and missing PDF
  PrintTo handlers retain PDFs with manifest WARNs; successful submissions record `printer:<name>`.
  Windows uses CIM paper names and PrintTo; CUPS uses `lpstat`/`lpoptions`/`lp`. Profiles are
  unchanged. Processes are bounded at 20 seconds and 4 MiB output. Added tests await CI;
  the surveyed Edge handler lacks PrintTo, and no physical-print pass is claimed.

### QA 2026-10-23

Web `a664cc8`, re-checked at `4ea3c75`, and Windows `d490620` smoke supplement the Linux
gate recorded under GĐ3. These are correctness checks, not benchmark measurements.

| Check | Result / evidence |
| --- | --- |
| Web interruption / resume | Block → all → Quick Plot 100 sheets, merged at 64 pages/file. Plot Jobs shows progress, current sheet and Cancel. Closing mid-job and reopening shows Interrupted with Resume/Discard. Resume without the source reports `SOURCE_CHANGED`; with it open, Done 100/100 yields 64 + 36 pages without duplicates. |
| Job panel / keyboard | `UBPLOT.JOBS` opens the left panel clear of ViewCube and focuses the first job action; Enter resumes, providing the Cancel/Resume keyboard path. Status bar shows Done after resume. |
| Desktop publish / Folder | Background publish of 20 sheets at 8 pages/file finishes DONE 20/20, PDFs 8 + 8 + 4. Folder mode on three DXFs × four approved A1 frames finishes DONE 12/12 with 12 split PDFs and no drawing open. Jobs panel is on the left with a keyboard hint and per-file Open folder. |
| Remaining gates | Real browser quota exhaustion, physical-printer submission and benchmark numbers remain pending. T13 is not merged, awaiting the user decision on T13B. |

## V2.2 GĐ6

- UBPLOT print profiles merge Platform → Company → Project → SheetSet → Drawing → User,
  preserving inherited locks. The Print Profile dialog shows origins and locked fields;
  `ubplot.profile` supports JSON edits/import/export without dialogs. SheetSet has no host
  binding yet. Desktop keeps last-good copies; browser print settings last for the visit.
- Frame templates have versioned ids, approval status and source hashes. Read-only audits
  feed explicitly approved baseline imports; changed sources supersede prior active versions
  while retaining them. Revision/duplicate/template reports are available as JSON or CSV.
- LISP adds `getpropertyvalue` and `ubplot-*` adapters. General VLA/ActiveX and AutoCAD `-plot`
  prompt compatibility remain unavailable.
- APPLOAD asks before loading unreviewed source; JSON APPLOAD and `(load)` require an approved
  digest. The LISP panel shows Reviewed/Unreviewed and eight digest characters. Desktop
  **Allowed folders…** controls manage write roots through LISPSANDBOX; web shows a read-only
  notice because LISP file streams cannot write there. SAVE/SAVEAS/WBLOCK still use host I/O
  outside this policy. MCP cannot change permissions or Company print policy.
- Editing routines retain one undo step and produce audit records. The panel shows the latest
  20, newest first; desktop rotates JSONL logs and web retains the session ring. Company locks
  and write restrictions are local policy, not an OS security boundary.

### QA 2026-10-23

Web `4ea3c75` QA and Windows `d490620` smoke supplement the Linux gate recorded under GĐ3.
`d490620` (Print Profile column fix) was verified locally only:
`cargo clippy --release -p cadcraft-ui-egui --all-targets -D warnings` clean and
`cargo test --release -p cadcraft-ui-egui --test ubplot_profile_ui` 2/2 pass.

| Check | Result / evidence |
| --- | --- |
| Web LISP panel | Empty state, Allowed folders… read-only notice and Load Application… reachable at 1366×768 and 200%. Web APPLOAD asks for a file name without a picker; loaded-file state was checked on desktop. |
| Print Profile | Value column was ~40 px wide at 1366/200%, cutting values. Fixed in `d490620`; desktop shows wide values in full ("CADCraft PDF", "{stem}"). Web re-check of the fix remains pending. |
| Desktop APPLOAD | Unreviewed file prompts with digest `ede1f430` and Yes/No/Always (default No). N refuses it and its command remains unknown; Y loads it, its command runs and the panel shows "Unreviewed ede1f430". |
| Open observations / remaining gates | P3: a long APPLOAD path overflows the floating command line. The LPC template audit step remains blocked on LPC-KHUNG TEN.dwg / LPC-ISO.dwt; AutoCAD round-trip acceptance is not claimed. |

### LPC samples (2026-10-23)

- Native golden checks read the local reviewed manifest and four DWGs through
  `UBOTCAD_LPC_DIR`; no sample data is committed. They check source hashes, sheet counts,
  exact issue-code counts, attributes, paper sizes and unchanged originals after scans.
- The three KC samples have 4, 8 and 15 `Khungten` INSERTs respectively. Each produces one
  297×420 mm sheet with `KIEU=A3`; all sheets correctly carry `FRAME_AMBIGUOUS` BLOCK because
  their closed outlines belong to missing xrefs (each frame also reports `XREF_MISSING_OUTSIDE`).
  PHỤ TRỢ produces 12 layout sheets at 297×420 mm with `PAPER_CUSTOM`, `FONT_MISSING` (17, SHX
  not available) and `PLOT_STYLE`, without BLOCK issues.
- `ubplot.templates audit` refuses these project drawings with `AUDIT_LIMIT`; one KC sample
  pins that refusal. The reviewed template uses plain registry import (approved, version 1).
  The plan's template audit step remains **blocked on LPC-KHUNG TEN.dwg / LPC-ISO.dwt**,
  which were not supplied; project-sample checks do not replace that baseline audit.
- Attribute text was observed in legacy TCVN3 (.VnTime), rather than Unicode. These checks
  pin the reviewed attributes; they do not establish legacy text conversion or font parity.

## KCS Plotter areas (KP / KL)

2026-10-26 · Print the way LPC prints with KCS Plotter (KetcauSoft) and a PDF printer.

- UBPLOT scan mode `kcs`: each Model-space LINE on layer `KCS_Plotter` (any case) is the diagonal of one window sheet; `handles` limits it to selected markers. Sheets default to ISO A3, orient by the window's long side and fit to paper, with `scale_label` `1:n` snapped to a whole `n` within 0.5 %. Lines with no width or height are skipped with plan-level WARN `KCS_LINE_DEGENERATE`; none at all is an error ("Không tìm thấy dấu hiệu của các vùng in!" in Vietnamese).
- Order replays KCS's own `Sap_Xep_Window_List` (decoded read-only from the plug-in's IL): an exchange sort over the lines in drawing order, top edge descending, then left edge ascending within a row of tolerance first-window-height / 12.5.
- The marker layer never plots on KCS sheets, whatever its plot flag: sheets carry `exclude_layers` and publish/background jobs plot from a copy with those layers set not to plot. Publish already merges into one PDF by default.
- `KP` (`kcsplotter`) scans all markers and opens the Quick Plot review with the new Source row on "Vùng in KCS (layer KCS_Plotter)"; `KL` (`kcsarea`) draws one marker from two points ("Chọn điểm đầu" / "Chọn điểm cuối"), creating `KCS_Plotter` with plotting off when missing, in one undo step. Typed UBPLOT gains the `Kcs` keyword. Fixed 0°/90° orientation is not offered (UBPLOT has no forced orientation); KPP (several files) is not ported.
- Checked on LPC's drawing with its xref: 17 A3 sheets in KCS order (2 x 1:100, 8 x 1:110, 7 x 1:50), one merged 17-page PDF without the marker layer.

## Xref files on the web

- The browser can't follow an xref's saved path, so a drawing opened on the web lost its xrefs (the LPC title-block frames are one). Now, after such a drawing opens, the command line names the missing xrefs ("Xref chưa nạp: … — mở bảng External References (XREF) và bấm Chọn file…"), and the External References palette shows **Choose file…** (vi "Chọn file…") for an xref that is Not Found or Unresolved. The picked DWG or DXF is loaded into that xref as a Reload from disk would load it: same parsing and binding (`xref/load.rs`), status Loaded, one "External References" undo step. A file of another name is accepted and the echo names it. A file that can't be loaded (not DWG/DXF, unreadable, over the size limits) leaves the drawing unchanged with a message. The picked file's own xrefs are not looked for; they stay unloaded and are listed on the command line.
- Engine: `Session::reload_xref_from_bytes` (`cmd/blocks/xref/picked.rs`) runs through `Session::execute_as`, the body of `execute` with the command's JSON form swapped for a call that takes bytes, so no base64 copy of the file is made. UI: `Services::pick_xref_file` (set only by the web app; the desktop palette is unchanged) and `CadApp::attach_xref_file`. Web: upload purpose `Purpose::Xref`.
- Nothing is kept: after a reload the file has to be picked again (IndexedDB storage is future work). One pick loads every xref of the drawing that is Not Found or Unresolved and whose saved path has the same file name (folders and case ignored, `\` or `/`), in the same undo step, with one "loaded from" line each; xrefs already loaded or unloaded are left alone.
- To avoid picking again on every visit: after the xref loads, **Bind** (or **Insert**) it in the palette and save the drawing. The frame then becomes part of the drawing, and the saved file keeps it.
- The command-line hint and the "drawing no longer open" error are logged in English and translated when shown (`echo` context), like engine messages: on the web a drawing can open before the UI language applies, which left the hint in English on a Vietnamese UI.

### XREF palette and -XREF on the web

- Status table (`cmd/registry.rs`): on the web only `xattach` and `xopen` are unsupported (they read files from disk); `xref`, `-xref` and `xbind` are available. `option_status_on` marks -XREF's Reload, Path, Attach and Overlay options unsupported on the web; `-xref` refuses them there with a message pointing to Choose file… (JSON and typed forms alike). Unload, Detach, Bind, Insert (bind keeping layer names) and Type work in memory. The desktop is unchanged.
- Palette (`ui-egui/src/xref_controls.rs` decides what shows): on the web, Attach DWG…, Reload All and Open are hidden; Reload shows only for an xref that is not loaded and opens Choose file…; the saved path is read-only with the note "On the web, use Choose file… to load" (vi "Trên web, dùng Chọn file… để nạp"), and Apply is hidden.

### Quick Plot: why Publish is disabled

- The reason now shows as a line next to Publish instead of only in a tooltip. Unapproved warnings read "Approve or exclude the warnings under Exceptions to publish" (vi "Chấp nhận hoặc loại trừ các cảnh báo ở mục Ngoại lệ để xuất bản"); the other reasons are "No sheets to publish" and "Fix or exclude blocked sheets before publishing a merged PDF".

2026-10-29 · Hovering over block references no longer stalls (UbotCAD V5.2-C, lag on Ryzen 5 2600 + GT 730).

- Picking under the cursor (rollover highlight and single-click pick) builds only the block contents within the pick box (`render::Options::near`) and takes solid hatch fills as their boundary loops (`Options::fill_loops`, `DisplayList::hit_fills`) instead of triangulating them.
- Hatch extents (ZOOM Extents, the spatial index, window selection, picks) come from the boundary loops directly (`geom::loop_bounds`, the same value as before bit for bit) instead of copying every loop into segments: 0.47 to 0.04 ms for a 3000-point hatch.
- Native release, 15 title blocks with 7 solid hatches of 3000 points each (`engine/tests/pick_insert.rs`, ignored timing tests): 0.5 / 1.0 ms per pick away from / on a hatch in a drawing past the spatial-index threshold; 4-8 ms in a drawing too small to index, where every pick measures all 15 blocks.
- Web build in Chrome, moving the mouse over those title blocks for 4 s (machine loaded by other builds): main thread busy 85% → 40% and long tasks 6 → 0 at normal CPU speed; at 4x CPU throttling, long tasks 33 (up to 176 ms) → 2-14 (up to 55-105 ms) and pointer moves handled 40 → 85-120. The long tasks left also happen over blank canvas: they are redraw cost, not picking.
- A point inside a solid hatch of a block now picks the block, and a point in that hatch's island doesn't, as for a hatch outside a block. Trim/extend, hatch boundaries and other callers of `hit_polylines`/`entity_distance` are unchanged.
