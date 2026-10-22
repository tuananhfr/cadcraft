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
- `revisions`: optional sheet-id-to-revision register, for example `{"KT-01": "C"}`.
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

Preflight runs after frame detection, overrides, exclusions and paper/scale resolution.
Each sheet gets its highest issue severity as its verdict: `PASS` when there are no
issues, then `INFO`, `WARN`, or `BLOCK`. Scan returns `summary` with exactly
`pass`, `info`, `warn`, and `block` counts, one verdict per sheet. Plan-level issues
remain in `issues` and do not inflate those sheet counts.

Clean and INFO-only sheets are `READY`; an unapproved WARN leaves a sheet in
`NEEDS_PREVIEW`; any BLOCK makes it `BLOCKED`. Status derives from issues rather
than a supplied status flag. A WARN approval requires both `approved_by_user` and a
matching audit fingerprint; a BLOCK always blocks. Audit storage and the approval
command arrive in B7. Paper/scale BLOCKs from resolution remain part of the verdict.

Preflight checks xref INSERT bounds in each sheet's space, including nested block
transforms and dynamic visibility. A block/window sheet uses its exact outline;
a layout sheet also checks the model AABB visible through each enabled viewport.
`NotFound`, `Unresolved`, and `NotLoaded` references intersecting a sheet add BLOCK
`XREF_MISSING`, naming the reference and its path. On the web this also explains that
external references are not loaded on the web. `Unloaded` adds WARN `XREF_UNLOADED`.
Missing references outside all sheets add plan-level INFO `XREF_MISSING_OUTSIDE`.
The AABB rule needs cached geometry: an empty definition or zero-area bound cannot
establish an overlap, so it is reported at plan level rather than assigned to a sheet.
Traversal is bounded to 16 nested levels and 200,000 visited entities; reaching a
limit adds BLOCK `XREF_SCAN_LIMIT` to every sheet. Repeating preflight replaces its
own issues and recomputes statuses without duplicating diagnostics.

Font preflight considers text styles used by visible entities on the planned sheets,
including nested INSERT transforms and layout viewport model bounds. The current text
style and unused definitions alone do not trigger issues. Missing fonts produce plan-level
WARN `FONT_FALLBACK` naming the replacement, or `FONT_MISSING` naming the built-in stroke
font. An explicit usable mapping produces INFO `FONT_MAPPED` instead. Font diagnostics
have `sheet_id: null`; no fonts are bundled. A `last` scan refreshes font diagnostics even
when drawing geometry is cached. Font traversal follows the renderer's block-depth limit
and rejects scans exceeding 200,000 visited entities/copies/cells.

Every sheet sharing an exact sheet id receives WARN `SHEET_ID_DUPLICATE`. Revisions use
the template's `attributes.revision` tag list and compare with `revisions[sheet_id]` by
natural order: `B < C`, `9 < 10`, `A2 < A10`, ignoring letter case and numeric leading
zeroes. An older or missing revision adds WARN `REVISION_OLD`; a newer revision adds
INFO `REVISION_NEWER`. A template's optional `superseded_by` adds WARN
`TEMPLATE_SUPERSEDED` naming its successor. Plan-level INFO `PLOT_STYLE` names each
selected layout plot-style table, or `colors as displayed` for pages without a table.

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
(`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`). Successful split delivery adds
INFO `FILE_RENAMED` to a sheet when the actual output filename stem differs from its
sheet id, including the base-name prefix, sanitization and collision suffixes. The message
names the delivered file. Repeating publish replaces this diagnostic; merged publish and
rescanning remove stale rename diagnostics.

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
| BLOCK | `FRAME_AMBIGUOUS`, `FRAME_NOT_FOUND`, `FRAME_MINSERT`, `PAPER_TOO_SMALL`, `PAPER_TOO_LARGE`, `SCALE_PAPER_MISMATCH`, `RENDER_FAILED`, `XREF_MISSING`, `XREF_SCAN_LIMIT` |
| WARN | `FRAME_DUPLICATE`, `FRAME_OVERLAP`, `FRAME_NESTED`, `FRAME_MIRRORED`, `FRAME_NONUNIFORM`, `XREF_UNLOADED`, `FONT_FALLBACK`, `FONT_MISSING`, `SHEET_ID_DUPLICATE`, `REVISION_OLD`, `TEMPLATE_SUPERSEDED` |
| INFO | `PAPER_CUSTOM`, `SCALE_NOT_SET`, `UNITS_ASSUMED`, `HANDLE_DUPLICATE`, `SHEET_EXCLUDED`, `SOURCE_HASH_WEAK`, `XREF_MISSING_OUTSIDE`, `FONT_MAPPED`, `REVISION_NEWER`, `PLOT_STYLE`, `FILE_RENAMED` |

Command errors include `PLAN_STALE` (drawing changed since scan), `PLAN_INVALID`
(invalid or unresolved plan), and `OUTPUT_IS_SOURCE` (output would overwrite the source).
WARN approvals, job queues, resume, real printers and folder publishing are later phases.
