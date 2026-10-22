# UBPLOT PDF publishing

`ubplot.scan` builds an ordered SheetPlan without opening a dialog. `ubplot.templates`
lists or imports frame detection templates. `ubplot.publish` renders the plan into a
merged PDF or one PDF per sheet. These commands are available through JSON, scripts,
CLI and control/MCP callers. Interactive picking and the review dialog are separate work.

```json
{"command":"ubplot.scan","params":{"mode":"block","handles":["A21","A34"]}}
{"command":"ubplot.publish","params":{"output":"merged","path":"drawing-set.pdf"}}
```

Scan parameters:

- `mode`: `block`, `window`, `layout`, `last`, or `plan`.
- `handles`: hexadecimal INSERT handles, in pick order, for block mode.
- `windows`: two corners or four rectangle vertices per window.
- `layouts`: layout names in order, or `["*"]` for layout tab order.
- `paper`, `scale`: optional paper name and `"fit"` or ratio such as `"1:100"`.
- `overrides`: settings keyed by `sheet_id@order`, with `paper`, `scale`, `window`, `sheetId`.
- `exclude`: explicit sheet keys to exclude, reported as `SHEET_EXCLUDED`.
- `plan`: a SheetPlan for `mode: "plan"`. Scan resolves flat block references into outlines.

`last` reuses the last scan request for the current drawing and rescans after edits.
Scan ratios are physical ratios based on `INSUNITS`, independent of `MEASUREMENT`.
For example, `"1:100"` in a metre drawing (`INSUNITS=6`) means 1 m becomes 10 mm on
paper. The stored `plot_scale` remains millimetres on paper per drawing unit, so this
example stores `"1:0.1"`; `scale_label` retains the requested `"1:100"`. Scale overrides
use the same conversion. Frame-derived ratios retain the INSERT scale and get a
`scale_label` of `1:(INSERT scale × millimetres per drawing unit)`, rounded to six
significant digits. Unitless (`INSUNITS=0`) and unknown codes assume millimetres and
add one plan-level INFO issue, `UNITS_ASSUMED`.

Templates take `action: "list" | "import" | "reset"`, and `text` for JSON import.

Publish parameters:

- `plan`: optional full SheetPlan; defaults to the last plan for this drawing.
- `output`: `"merged"` (default) or `"split"`.
- `path`: merged file path, or the base file whose directory and stem name split files.
- `title`: optional PDF document title.

The result is `{plan, files, failed}`. Each file has `name`, `sheets` (sheet keys),
`bytes` (byte count), `sha256` (SHA-256 of the PDF bytes), and either a desktop `path`
or base64 `data` when `path` was omitted. Failed entries have `sheetKey` and `issues`.
The returned plan and the last stored plan carry `DONE`, `BLOCKED`, or `FAILED` statuses.
Sheets always print by numeric `order`, with their sheet ids as PDF page labels.

Block and window pages use their exact rectangle outlines, including rotation and
mirroring. Blocks print in their owning model or layout space. Paper size and landscape
come from each sheet, with zero margins. In this phase, frame crops fit the paper sized
by scan. Applying explicit scales during publishing and extended paper support are
planned separately.

A merged plan containing any blocked sheet is rejected before writing, listing the
blocked sheet keys. Split output renders other sheets and reports every blocked sheet
in `failed`. A merged render error marks all its sheets `FAILED`, adds `RENDER_FAILED`,
and returns an error containing `plan=<JSON>`. Split render or delivery errors affect
only the corresponding sheet. To retry failed rendering, rescan or supply a corrected
plan; BLOCK issues cannot be approved or silently ignored.

Desktop writes use `{out}.{job_id}.part` followed by rename. Existing temporary files
are never truncated. On success no temporary file remains. Normalized output paths
(including split destinations and temporary paths) must differ from the source drawing;
Windows comparison ignores case. Split names replace forbidden characters, preserve
Unicode letters, and append `-{order}` for duplicate ids and Windows device names
(`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`).

On the web, a supplied path provides the download name through the host delivery hook;
there is no filesystem write. Split downloads are limited to 20 files because browsers
may block repeated downloads; use merged output for larger sets. Without a path, both
platforms return base64 PDF data. No command opens a save dialog programmatically.

Limits: 10,000 scan frames/windows/layouts; 200,000 entity leaves per INSERT; nested
blocks at most 16 levels; 256 pages per merged PDF; paper edges at most 5080 mm.
Registry import is limited to 256 KiB, 500 templates, and 64 names per template.

Issue codes:

| Severity | Codes |
| --- | --- |
| BLOCK | `FRAME_AMBIGUOUS`, `FRAME_NOT_FOUND`, `FRAME_MINSERT`, `PAPER_TOO_LARGE`, `RENDER_FAILED` |
| WARN | `FRAME_DUPLICATE`, `FRAME_OVERLAP`, `FRAME_NESTED`, `FRAME_MIRRORED`, `FRAME_NONUNIFORM` |
| INFO | `PAPER_CUSTOM`, `SCALE_NOT_SET`, `UNITS_ASSUMED`, `HANDLE_DUPLICATE`, `SHEET_EXCLUDED`, `SOURCE_HASH_WEAK` |

Command errors include `PLAN_STALE` (drawing changed since scan), `PLAN_INVALID`
(invalid or unresolved plan), and `OUTPUT_IS_SOURCE` (output would overwrite the source).
WARN approvals, job queues, resume, real printers and folder publishing are later phases.
