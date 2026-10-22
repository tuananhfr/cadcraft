# UBPLOT PDF publishing

`ubplot.scan` builds an ordered SheetPlan without opening a dialog. `ubplot.templates`
lists or imports frame detection templates. `ubplot.publish` renders the plan into a
merged PDF or one PDF per sheet. These commands are available through JSON, scripts,
CLI and control/MCP callers. Interactive UBPLOT opens picking and a review dialog;
programmatic calls never open dialogs.

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
- `device`: optional device name from PLOTTERMANAGER; UBPLOT still publishes PDF.
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

PLOT and EXPORTPDF keep their legacy paper-units-per-drawing-unit scale unless
`scaleMode: "real"` is supplied. With an explicit scale and no `fit`, PDF and PNG
preserve the scale; `fit: true` explicitly requests fitting. UBPLOT always publishes
Ratio sheets with `fit: false`, so a paper mismatch never silently changes the ratio.

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
matching audit fingerprint; a BLOCK always blocks. Paper/scale BLOCKs from resolution
remain part of the verdict.

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

Approve warnings before publishing:

```json
{"command":"ubplot.approve","params":{"approvals":[{"sheetKey":"S-001@1","code":"FRAME_OVERLAP"}],"by":"user"}}
{"command":"ubplot.approve","params":{"approvals":[{"sheetKey":"*","code":"FONT_FALLBACK"}],"by":"agent:claude"}}
```

`ubplot.approve` takes an optional `plan` (otherwise the last plan), an `approvals`
array of `{sheetKey, code}` items and `by`. It returns the updated SheetPlan and
stores it as `last`. Sheet keys are `sheet_id@order`; plan-level issues use `"*"`.
Only WARN is approvable. BLOCK, INFO, PASS, unknown sheets/codes or an invalid `by`
reject the whole request without changing the plan. Device splits are approved
explicitly through the optional `split` array; resolving the size problem removes
`DEVICE_LIMIT_EXCEEDED` without approving a BLOCK issue.
`by` must be `"user"` or `"agent:<name>"` with a non-empty name, and 1–64 characters
in total. A call accepts at most 10,000 items; a plan holds at most 100,000 audit entries.
Repeated items are deduplicated within a call; multiple issues with the same key/code
are all approved, with one audit entry per distinct fingerprint.

Every approval appends `{at_ms, by, action: "approve", sheet_key, code, fingerprint}`
to `audit`. Old plans default to an empty audit. The fingerprint is the first twelve
hexadecimal characters of SHA-256 over `code + "|" + sheet_key + "|" + message`.
`approved_by_user` only counts with a matching approval audit entry. Rescanning or
using `last` preserves unchanged approvals and audit history; a changed message
expires approval and keeps the old record. Explicit `mode: "plan"` scans use the
supplied approval flags and retain both the session and supplied audit histories.
Native hosts install a Unix millisecond clock; web uses the browser clock. Without
a clock hook, audit timestamps are zero.

Publish parameters:

- `plan`: optional full SheetPlan; defaults to the last plan for this drawing.
- `output`: `"merged"` (default) or `"split"`.
- `path`: merged file path, or the base file whose directory and stem name split files.
- `title`: optional PDF document title.

With `background` absent or `false`, publishing remains synchronous with the same
renderer, result fields and failure handling described below.

The result is `{plan, files, failed}`. Each file has `name`, `sheets` (sheet keys),
`bytes` (byte count), `sha256` (SHA-256 of the PDF bytes), and either a desktop `path`
or base64 `data` when `path` was omitted. Failed entries have `sheetKey` and `issues`.
The returned plan and the last stored plan carry `DONE`, `BLOCKED`, or `FAILED` statuses.
Sheets always print by numeric `order`, with their sheet ids as PDF page labels.
Approved split pieces follow in piece order, labelled `{sheet_id} ({i}/{n})`.

Block and window pages use their exact rectangle outlines, including rotation and
mirroring. Blocks print in their owning model or layout space. Paper size and landscape
come from each sheet. Ratio sheets print at their stored scale; only Fit sheets are
fitted. Each merged page keeps its own MediaBox, so A1, A3 and long sheets can share a
PDF without shrinking. Rolls use the selected margin on all four edges. Other pages
default to zero margins; a matching non-PDF device paper from the sheet's owning
layout supplies its margins. The scan `device` controls size preflight; it does not
supply margins for model-space blocks or windows.

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

Background jobs:

- `background: true` queues the supplied `plan` or the last scanned plan and returns
  `{jobId, status: "QUEUED"}`. The host can take the job through `take_queued_job()`;
  queueing itself does not render or open a dialog.
- `mode`: `"merged"` (default) or `"split"` (one file per sheet); `output` is also accepted.
- `pagesPerFile`: 1–1000, default 256 on desktop or 64 on web. Approved split pieces
  count as pages and stay together, even when one sheet exceeds the chosen group size.
- `retryLimit`: 0–5 retries after the first attempt, default 2.
- `path`: optional base filename whose directory and stem name queue output. Stems
  contain ASCII letters, digits, dots or hyphens, up to 160 bytes. Merged files are
  `{stem}.pdf` or `{stem}-{group:03}.pdf`; split files are `{stem}-{order:04}-{id}.pdf`
  with sanitized ASCII sheet ids. Without a path the store chooses delivery.

Background jobs refuse blocked sheets and plan-level BLOCK issues with `PLAN_HAS_BLOCKED`.
WARN issues still require audited approval and stale plans must be rescanned. A source
path cannot be used as the output path. The store persists a `job.json` definition with
its QUEUED manifest so another session can resume without the previous scan state.
Without a host-installed store, a session uses `MemoryJobStore` and adds INFO
`JOBS_NOT_PERSISTED`; those jobs disappear when the session ends. Queue delivery goes
through the store; it does not add base64 data or a manifest to synchronous publish results.

`ubplot.jobs` accepts `action` and, except for `list`, a `jobId`:

- `list` returns `[{jobId, status, done, failed, total, updatedMs}]`.
- `status` returns the same summary for one job; `manifest` returns `{manifest}`.
- `resume` runs synchronously to completion and returns `{manifest}`. It uses the
  open drawing snapshot and its source hash; changed sources fail pending sheets
  instead of mixing drawings. Both budgeted PDF rendering and PDF merge hooks must
  be installed by the host. Completed outputs are retained without redelivery.
- `cancel` records a request for the host. A queued job becomes CANCELLED and is
  removed from the pending queue; RUNNING jobs stop when their host processes the
  request. `take_cancel_requests()` drains the requests.
- `remove` deletes stored job state and pending requests; RUNNING jobs are refused.
  Delivered output files are retained.

A resume manifest has `status`, ordered `sheets` (attempts, checksums, issues and
output/page indices), and `files` (name, pages, checksum, delivery destination/time).
Statuses are QUEUED, RUNNING, CANCELLED, DONE or FAILED. Timestamps use the installed
clock hook or zero. Repeating an identical publish returns the stored job status;
reusing its id with different plan contents is refused rather than overwriting progress.

```json
{"command":"ubplot.publish","params":{"background":true,"mode":"merged","pagesPerFile":64}}
{"command":"ubplot.jobs","params":{"action":"list"}}
{"command":"ubplot.jobs","params":{"action":"resume","jobId":"<jobId from publish>"}}
```

Limits: 10,000 scan frames/windows/layouts; 200,000 entity leaves per INSERT; nested
blocks at most 16 levels. Each PDF accepts at most 1000 pages and 512 MiB, with paper
edges up to 15000 mm (15 m). The page cap includes approved split pieces. The writer
collects glyphs in a separate pass and releases each page display list after writing it.
Edges over 5080 mm (14400 points) use `/UserUnit = ceil(longest_edge_pt / 14400)`;
MediaBox and content are divided by that unit, preserving physical size, with catalog
version PDF 1.6. Oversized paper is rejected. Jobs over the merged page or byte cap
report an error suggesting publication in parts; they do not silently omit pages.
PNG retains its pixel cap. Registry import is limited to 256 KiB, 500 templates, and
64 names per template.

Issue codes:

| Severity | Codes |
| --- | --- |
| PASS | No issues. |
| BLOCK | `FRAME_AMBIGUOUS`, `FRAME_NOT_FOUND`, `FRAME_MINSERT`, `PAPER_TOO_SMALL`, `PAPER_TOO_LARGE`, `SCALE_PAPER_MISMATCH`, `RENDER_FAILED`, `XREF_MISSING`, `XREF_SCAN_LIMIT`, `DEVICE_LIMIT_EXCEEDED` |
| WARN | `FRAME_DUPLICATE`, `FRAME_OVERLAP`, `FRAME_NESTED`, `FRAME_MIRRORED`, `FRAME_NONUNIFORM`, `XREF_UNLOADED`, `FONT_FALLBACK`, `FONT_MISSING`, `SHEET_ID_DUPLICATE`, `REVISION_OLD`, `TEMPLATE_SUPERSEDED`, `DEVICE_LIMIT_NEAR`, `DEVICE_UNAVAILABLE` |
| INFO | `PAPER_CUSTOM`, `SCALE_NOT_SET`, `UNITS_ASSUMED`, `HANDLE_DUPLICATE`, `SHEET_EXCLUDED`, `SOURCE_HASH_WEAK`, `XREF_MISSING_OUTSIDE`, `FONT_MAPPED`, `REVISION_NEWER`, `PLOT_STYLE`, `FILE_RENAMED` |

Publish validates approval flags before rendering or writing. A flagged BLOCK returns
`BLOCK_NOT_APPROVABLE`, even with a forged audit and regardless of who approved it.
A flagged WARN without matching audit returns `APPROVAL_WITHOUT_AUDIT`. Remaining
unapproved WARNs return `WARN_NOT_APPROVED: sheet_key:code, ...`, including plan-level
warnings under `*`. There is no bypass flag; both merged and split output require approval.

The review dialog shows WARN/BLOCK exceptions by default, grouped by code and severity.
`Show all sheets` includes clean and INFO-only sheets. WARN groups offer `Approve all`
with an audit confirmation modal; approved rows have a locked `Approved` checkbox.
BLOCK groups offer window/paper fixes or exclusion, with no approval control.
The summary shows PASS/INFO/WARN/BLOCK sheet counts. Publish is disabled while WARNs
remain unapproved, or while merged output contains a BLOCK, with a reason tooltip.
Desktop device-size exceptions offer the split controls described below.

Command errors include `PLAN_STALE` (drawing changed since scan), `PLAN_INVALID`
(invalid or unresolved plan), and `OUTPUT_IS_SOURCE` (output would overwrite the source).
Job queues, resume, real printers and folder publishing are later phases.

Device limits and approved splits:

`ubplot.scan` accepts `device: "A1 Plotter"`, resolved through PLOTTERMANAGER.
The optional `SheetPlan.device` retains that request when a plan is supplied later.
Limits are the greatest short and long sides among the device's paper sizes;
the built-in `CADCraft PDF` supports both sides up to 15000 mm. Paper reaching
95% of either limit reports WARN `DEVICE_LIMIT_NEAR`. Exceeding either limit
reports BLOCK `DEVICE_LIMIT_EXCEEDED` and, when possible, an unapproved
`sheet.split` proposal. The short side must fit; otherwise choose PDF.

```json
{"command":"ubplot.approve","params":{"split":[{"sheetKey":"S-001@1","overlapMm":20}],"by":"user"}}
```

`overlapMm` defaults to 20 mm and accepts 0–200 mm. A long edge L is divided into
`ceil((L - overlap) / (max_long - overlap))` equal pieces in the outline's frame.
WARN approvals and split approvals in one call succeed atomically. Split approval
appends an audit entry with `action: "split"`, binding the sheet geometry, paper,
scale, device limits and overlap. Only an approved split with matching audit is
published; changed inputs require approval again. Unchanged rescans retain approval
and the append-only audit. Layouts without an outline cannot be split; choose PDF.
Each piece is an exact borderless PDF crop, preserving its geometry without inherited
roll margins clipping the overlap. No pieces are published before explicit approval.
With `output: "split"`, each original sheet produces one PDF containing all its approved
pieces; this output mode alone does not approve or create a device split.

Split PDF pages show 0.25 mm dashed seam lines at their overlap edges and two
10 mm end ticks. JSON PDF `pages` items accept `seams: [left,bottom,right,top]`
and optional `seamOverlapMm` (20 mm by default). Pages without seams retain their
previous bytes.

UBPLOT on the web publishes PDF only; device-limit splitting is a desktop workflow.
An unknown or removed device produces WARN `DEVICE_UNAVAILABLE` and falls back to
PDF after warning approval. Web contexts treat every device except `CADCraft PDF`
as unavailable and produce no device BLOCK or split proposal. Publishing rechecks
capabilities so a removed device cannot silently use an old split or another printer.

## Headless CLI

```text
cadcraft-cli ubplot --folder DIR --out DIR [--mode merged|per-sheet] [--pages-per-file N] [--max-files N] [--max-depth N] [--jobs-dir DIR] [--layouts]
cadcraft-cli ubplot FILE.(dwg|dxf) --out OUT.pdf [--mode merged|per-sheet] [--pages-per-file N] [--jobs-dir DIR]
```

The CLI uses the engine's folder scan rule: select model-space frames matching a
registered non-generic template. `--layouts` also includes paper-space layouts in
folder mode. The current standalone CLI starts with only the generic template and
has no registry-loading interface, so registered model-space frame selection is
pending that interface. Layout publishing through `--folder ... --layouts` is available.

`merged` is the default. `per-sheet` selects the engine's `split` mode. Folder output
uses the stem `plot`; single-file output uses the stem and directory of `OUT.pdf`.
Merged jobs split into numbered files when `--pages-per-file` is exceeded. The page
limit is 1–256 (default 256), `--max-files` is 1–5000 (default 1000), and
`--max-depth` is 0–32 (default 16). The last two options and `--layouts` require
folder mode. Paths can contain spaces; quote them in the shell.

`--jobs-dir` defaults to `.ubplot-jobs` inside the output directory (the parent of
`OUT.pdf` in single-file mode). Repeat the same command to resume the persisted job.
Completed outputs retain their checksums and modification times. Jobs store their
source hashes, plans and progress; pending sheets reread and verify source files.

Stdout contains the manifest JSON only. Exit status is 0 for `DONE`, 1 for failed
or cancelled jobs and operational errors, and 2 for invalid arguments. Diagnostics
go to stderr. Arguments are validated before source loading or filesystem writes.
