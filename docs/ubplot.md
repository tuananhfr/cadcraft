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
- `paper`, `scale`: optional paper choice (below) and `"fit"` or ratio such as `"1:100"`.
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

Paper choices are strings; the same grammar applies to per-sheet overrides:

| String | Paper |
| --- | --- |
| `"auto"` | Match the frame to a standard or elongated sheet; otherwise use custom millimetres. |
| `"A1"` | A standard paper name or alias from the paper table, including A0–A5. |
| `"A1x4"` | Elongated paper, 841×2378 mm. |
| `"841x2400"` | Custom width×height in millimetres. |
| `"roll:914"`, `"roll:914:5"` | Roll width 914 mm, with a default or explicit 5 mm margin. |

Names and `x` are case-insensitive. Dimensions must be finite and within 1–15000 mm.
Roll margins must be finite, within 0–1000 mm, and leave a printable width. Roll length
is the frame's long edge at the selected scale plus twice the roll margin, rounded up
to the next whole millimetre. `paper_mm` stores portrait dimensions, and `landscape`
follows the frame. A roll too narrow for the short edge plus both margins blocks with
`PAPER_TOO_SMALL`; a roll longer than 15000 mm blocks with `PAPER_TOO_LARGE`.

`paper_name` records the resolved name (or custom dimensions / normalized roll string).
Without a paper override, blocks retain media inferred from the unscaled frame, windows
default to A3, and layouts retain their page setup. `"auto"` on a window uses the frame
at its selected scale; Fit windows use their drawing dimensions converted to mm.
A ratio override never enlarges the block's inferred paper or silently fits its frame.
If the scaled frame exceeds the printable area, scan blocks it with
`SCALE_PAPER_MISMATCH`, including the requested size, scale, paper and printable size.
Matching allows the existing 0.5% or 1 mm tolerance, whichever is greater.
Nonstandard custom sizes carry INFO `PAPER_CUSTOM`.

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
come from each sheet. Ratio sheets print at their stored scale; only Fit sheets are
fitted. Each merged page keeps its own MediaBox, so A1, A3 and long sheets can share a
PDF without shrinking. Margins are zero for PDF/custom paper, use device margins when
available, and use the selected margin on all four edges for rolls.

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
blocks at most 16 levels; 256 pages per merged PDF. Scan accepts paper edges up to
15000 mm; the PDF writer still limits edges to 5080 mm until the separate B4 writer
upgrade. Publishing a larger scanned sheet currently reports `RENDER_FAILED`.
Registry import is limited to 256 KiB, 500 templates, and 64 names per template.

Issue codes:

| Severity | Codes |
| --- | --- |
| BLOCK | `FRAME_AMBIGUOUS`, `FRAME_NOT_FOUND`, `FRAME_MINSERT`, `PAPER_TOO_SMALL`, `PAPER_TOO_LARGE`, `SCALE_PAPER_MISMATCH`, `RENDER_FAILED` |
| WARN | `FRAME_DUPLICATE`, `FRAME_OVERLAP`, `FRAME_NESTED`, `FRAME_MIRRORED`, `FRAME_NONUNIFORM` |
| INFO | `PAPER_CUSTOM`, `SCALE_NOT_SET`, `UNITS_ASSUMED`, `HANDLE_DUPLICATE`, `SHEET_EXCLUDED`, `SOURCE_HASH_WEAK` |

Command errors include `PLAN_STALE` (drawing changed since scan), `PLAN_INVALID`
(invalid or unresolved plan), and `OUTPUT_IS_SOURCE` (output would overwrite the source).
WARN approvals, job queues, resume, real printers and folder publishing are later phases.
