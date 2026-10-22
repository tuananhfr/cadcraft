# Workspace limitations

Floating panels, toolbars and the command line stay inside the main application window. Native floating windows on a second monitor are not supported. On a PC, when a second monitor is disconnected, the operating system moves the main window back to an available screen; floating controls follow that window and are clamped to its work area. This monitor-disconnection behavior has not been tested on physical hardware (UX07 partial).

Status-bar item visibility belongs to the workspace profile. Hiding a drafting toggle does not change its system variable; those settings continue to persist through `PROFILE_VARS`. The schema IDs `lineweight`, `transparency`, `quickprops`, `annotation`, `workspace`, and `settings` address the existing lineweight, transparency, Quick Properties, annotation visibility/scale, workspace picker, and Drafting Settings controls. `layouts` hides the layout tabs and their add/list buttons; `coordinates` hides the coordinate readout. Unknown status-bar IDs have no control to render: they are retained in the profile and ignored without a placeholder.

SIMPLE has neither a ribbon nor Quick Access buttons. Its default command line floats at the bottom centre with the existing 48% width rule (360–760 points, bounded by the canvas). Top and Bottom command lines reserve their own row, sized by `height_ratio` and constrained to leave room for the drawing. Interface scale is 0.75–2.0; the Options slider uses steps of 0.05, and egui keyboard zoom is saved through the normal workspace debounce.

The in-window menu bar scrolls horizontally when its menus and Quick Access exceed the available width. When zoom or a shorter window leaves insufficient height for the drawing, the ribbon renders as tabs only. The saved expanded/collapsed preference is preserved, so groups reappear when the window has enough room.


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

## V2.2/V3.1 web keys (GĐ0 / GĐ3 Task 11)

2026-10-22 · Implementation inventory; browser QA and release acceptance remain pending.

Browser-reserved keys are removed from web defaults and saved-profile dispatch, refused by the shortcut editor and hidden from menu hints. Customize → Keyboard warns about a browser key before assignment and disables Assign Shortcut until a different key is entered. The web defaults add these alternatives to the filtered desktop table:

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

Standalone web pages request beforeunload confirmation only while an open drawing has unsaved changes. Browsers require a prior user gesture and control the prompt text; this is not crash recovery or drawing autosave. Embedded `?host=parent` mode delegates leave protection to the parent through dirty reporting. Web drawing autosave is deferred to GĐ4.

PUBLISH delivers one download per output file, without a zip in GĐ0. The browser may require permission for multiple downloads. PLOT, EXPORT, PDFBOOK, plotter saves and workspace export also use file delivery; embedded mode delegates delivery to its host. Workspace JSON import has a separate upload route; drawing uploads continue through `open_bytes`. DXF IMPORT still opens the file as a new drawing rather than inserting it (unchanged in GĐ0; deferred for GĐ7 review).

Workspace revision checks detect a stale writer before commit, on desktop as well as web. The web listens for localStorage changes and adopts them when there are no local edits pending; otherwise commit reports a local conflict and offers a choice of layouts. This does not provide cross-tab atomicity: simultaneous check-then-write operations can still overwrite one another. GĐ4 replaces that window with an IndexedDB transaction. These local conflicts are separate from remote synchronization.

PDFBOOK still selects frames by the requested height. Other-height closed outlines are ignored; only requested-height frames that match no paper receive `FRAME_PAPER_MISMATCH`. Invalid coordinates/dimensions report `FRAME_OUT_OF_RANGE`; reaching the model-entity scan cap reports `FRAME_LIMIT`, which does not count as a skipped frame. These diagnostics do not establish general frame-detection parity.

GĐ0 shortcut, upload, recovery and local-conflict messages have Ukrainian catalog entries. Vietnamese translations, including the recovery revision notice and the conflict choices, remain a GĐ1B handoff; no `vi.tsv` exists in this branch.

Browser fullscreen requires a click in the application and Fullscreen API permission. Embedded iframes need `allow="fullscreen"`. If the browser refuses fullscreen, the application enters Focus mode and shows a status message. Programmatic web requests return Unsupported; desktop fullscreen uses the native viewport. Focus and fullscreen are temporary and are never saved in a workspace profile.
