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

## V2.2 GĐ2

- PLOT and EXPORTPDF with an explicit `scale` and no `fit` now preserve that scale on Model instead of silently fitting. PDF page items and PNG use the same default; `fit: true` still requests fitting. UBPLOT publishes Ratio sheets at their stored scale and Fit sheets with fitting, with per-page paper margins (zero for PDF/custom paper; device margins when available).
- UBPLOT scan ratios now follow INSUNITS independently of MEASUREMENT: `1:100` in a metre drawing stores 10 mm on paper per drawing unit. `scale_label` preserves the requested ratio or the frame-derived physical ratio (six significant digits). Unitless and unknown INSUNITS codes assume mm and report plan-level INFO `UNITS_ASSUMED`.
- PLOT and EXPORTPDF accept `scaleMode: "real"` to interpret an explicit `scale` as a physical ratio using INSUNITS. Without it, scale keeps its existing paper-units-per-drawing-unit meaning.
- UBPLOT accepts `auto`, standard/elongated paper names (such as `A1x4`), custom `841x2400` mm and `roll:914[:5]` strings, including per-sheet overrides. Roll lengths follow the scaled frame plus both margins, rounded up to 1 mm. `paper_name` retains the resolved choice. Too-narrow rolls and scale/paper mismatches block publishing; merged pages retain their individual sizes. Scan and plotter configurations accept up to 15000 mm; the writer's 5080 mm edge cap and 256-page cap remain for B4.
