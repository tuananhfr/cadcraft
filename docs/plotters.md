# Plotters

For ordered frame, window and layout PDF publishing, see [UBPLOT](ubplot.md).

A plotter (output device) decides what a plot writes and on which paper sizes. Page setups
and plots pick a device and one of its papers. Each paper carries margins, which bound its
printable area. Layouts show the printable area as a dashed outline on the sheet.

## Devices

The built-in devices come first:

| Device | Output |
|---|---|
| `CADCraft PDF` | vector PDF |
| `CADCraft PNG` | raster PNG on white paper at the device resolution (300 dpi) |
| `CADCraft SVG` | vector SVG |

They offer the standard sizes:

- ISO 216 A0–A6 and B0–B5
- ANSI A–E and Legal
- ARCH A–E and E1

ISO sheets have 5 mm margins. Inch sheets have quarter-inch (6.35 mm) margins. A page setup
with the device `None` plots to PDF on the page setup's paper, as before.

User devices are files in our own format, kept in the `plotters` folder of the CADCraft
settings directory:

| System | Folder |
|---|---|
| Linux and FreeBSD | `$XDG_CONFIG_HOME/cadcraft/plotters` or `~/.config/cadcraft/plotters` |
| macOS | `~/Library/Application Support/CADCraft/plotters` |
| Windows | `%APPDATA%\CADCraft\plotters` |

`CADCRAFT_CONFIG_DIR` replaces the settings directory. On the web, devices last for the
session unless the host provides storage (`cmd::plotters::set_plotter_store`). Binary `.pc3`
files are not read; we haven't found a public specification of that format.

## Files

`<name>.plotter.json`, UTF-8 JSON with a format marker:

```json
{
  "cadcraftPlotter": 1,
  "name": "Office A3",
  "description": "Raster proofs",
  "output": "png",
  "resolution": 150,
  "orientation": "landscape",
  "defaultPaper": "A3 borderless",
  "papers": [
    { "name": "A3 borderless", "width": 297, "height": 420, "margins": [0, 0, 0, 0] },
    { "name": "ISO A4 (210.00 x 297.00 MM)", "width": 210, "height": 297, "margins": [5, 5, 5, 5] }
  ]
}
```

- `output` is `pdf`, `png` or `svg`.
- `resolution` is in dots per inch (50–2400) and applies to PNG.
- `orientation` (`portrait` or `landscape`) is used when a page setup switches to the device.
- Paper sizes are in millimetres on the portrait sheet. `margins` are left, bottom, right and
  top, in mm.

Reading fixes hostile values:

- Sizes are kept within 1–15000 mm, and width and height are swapped to portrait.
- Margins are kept within 0–1000 mm. Margins that leave no printable area are dropped.
- Paper names are unique, and a device keeps at most 512 papers.
- A missing or unknown `defaultPaper` becomes the first paper.
- Files over 1 MB are refused.

PDF publications accept at most 1000 pages and 512 MiB. Sheets with a side over 14400 points
use `/UserUnit` and catalog version PDF 1.6; each side must be at most 15000 mm.

In the file name, characters other than letters, digits, spaces and `-_()+,` become `_`.

## Commands

- `plottermanager {action, …}` (also **File ▸ Plotter Manager...**) manages the devices:
  - `list` (the default) shows the built-in devices first.
  - `get {name}` returns a device.
  - `new {name, from?, output?, …}` copies `from`, or the built-in device for `output` with
    the standard papers.
  - `set {name, description?, output?, resolution?, orientation?, defaultPaper?, paper?: {name,
    width, height, margins?}, removePaper?}` edits a device. `paper` adds a paper or replaces
    the one of that name.
  - `delete {name}` removes a device.
  - `load {text, name?}` stores a device file's contents.
  - `save {name, path?}` writes the file to `path`, or returns it as `text`.

  Built-in devices can't be edited or deleted.
- Typed `PLOTTERMANAGER` or `-PLOTTERMANAGER` prompts for an option:
  - `?` lists the devices.
  - `New` asks for a name and an output (`Pdf`, `PNg`, `Svg`).
  - `Delete` asks for a device.
  - `Paper` asks for a device, a paper name and `width,height` in mm, and adds that paper.
- `pagesetup {layout, device, paper?}` sets the layout's device. `paper` names one of the
  device's papers, or a standard size by its short name (`A3`, `Letter`). The paper's
  margins become the layout's printable area. Without `paper`, a device change keeps the
  current paper if the device has it, else takes the device's default paper. A device change
  also takes the device's orientation unless `landscape` is given. `device: "None"` clears it.
- `plot {device?, paper?, …}` plots with `device`, or with the layout's page setup device.
  - The device sets the output and resolution.
  - `paper` (one of the device's papers) sets the size and margins for this plot.
  - A model-space plot without `paper` uses the device's default paper and orientation.
  - For Model plots, `center` defaults to `true`, preserving the existing centered output. Set
    `center: false` with `offset: [x, y]` to place the plotted bounds' lower-left corner from the
    printable area's lower-left origin; offsets are millimetres, default to `[0, 0]`, and are
    bounded to ±100,000 mm. PDF and PNG support these controls. SVG plots keep their extents view;
    an active offset is rejected rather than ignored. Layout plots do not use model offsets.
  - `scale` is a positive number or ratio such as `"1:100"`. By default it means paper
    units per drawing unit: paper units are millimetres with `MEASUREMENT=1`, inches
    otherwise. This legacy meaning is unchanged.
  - `scaleMode: "real"` interprets `scale` as a physical ratio using `INSUNITS`,
    independently of `MEASUREMENT`. For example, a metre drawing (`INSUNITS=6`) at
    `"1:100"` prints 1 m as 10 mm. Unitless or unknown INSUNITS codes assume mm.
  - With `scale` and no `fit`, PDF and PNG preserve the requested scale. Model plots
    without a scale fit by default; layouts default to 1:1. `fit: true` explicitly fits even
    when a scale is supplied; `fit: false` without a scale uses 1:1.
  - PDF `pages` items accept their own `scale`, `fit` and `margins`; an item without `fit`
    fits only when it has no scale. UBPLOT Ratio sheets publish with `fit: false`, Fit
    sheets with `fit: true`. UBPLOT defaults to zero margins except for rolls or a
    matching non-PDF device paper from the sheet's owning layout. The scan `device`
    controls size checks; it does not supply model-space block/window margins.
  - `exportpdf` takes the same options and always writes PDF.

PDF and PNG plots apply the paper's margins to their printable area. SVG plots show a
layout's whole sheet, or the model-space extents.

## UBPLOT device limits and splits

`ubplot.scan {device: "A1 Plotter", …}` checks the greatest short and long sides among
that device's papers. `CADCraft PDF` allows 15000 mm on either side. At least 95% of
either limit produces WARN `DEVICE_LIMIT_NEAR`; exceeding either produces BLOCK
`DEVICE_LIMIT_EXCEEDED`. Device definitions describe capabilities; real printer
submission and driver queries remain pending. PDF batch queues are described below.

An oversized long sheet can have an unapproved overlap split proposal. The short side
must fit the device width; otherwise choose PDF. Scan and Publish never split it
automatically. On desktop, the review dialog shows the piece count, `Overlap (mm)`
(0–200, default 20) and `Approve split`. Programmatic approval uses:

```json
{"command":"ubplot.approve","params":{"split":[{"sheetKey":"S-001@1","overlapMm":20}],"by":"user"}}
```

Approval records an audit entry tied to geometry, paper, scale, device limits and overlap.
It resolves the size problem; BLOCK issues themselves cannot be approved. Approved pieces
preserve scale and rotation, publish in sheet/piece order, and use page labels such as
`S-001 (1/3)`. Dashed 0.25 mm seams and 10 mm end ticks mark overlap edges. Pieces count
toward the 1000-page PDF cap. `output: "split"` means one PDF per original sheet, containing
all its approved pieces; it does not authorize a device split.

UBPLOT on the web publishes PDF only. Every device other than `CADCraft PDF` produces
WARN `DEVICE_UNAVAILABLE`, with PDF fallback after approval and no device split proposal.
An unknown or removed desktop device has the same warning and fallback. Publishing
rechecks capabilities and never silently selects another printer. This restriction applies
to UBPLOT; the built-in PNG/SVG outputs remain available through PLOT.

See [UBPLOT](ubplot.md) for paper strings, physical scale, preflight and approval details.

## Batch jobs

2026-10-23 · Scoped runtime QA: web `a664cc8`, re-check `4ea3c75`, resumed an interrupted
100-sheet job with the source open to Done 100/100, producing 64 + 36 pages without
duplicates; without the source, Resume reported `SOURCE_CHANGED`. The left Plot Jobs panel
showed progress/current sheet and Cancel, then Interrupted with Resume/Discard after reopen.
`UBPLOT.JOBS` focused the first action, Enter resumed, and the status bar showed Done.
Windows 11 local release `d490620` published 20 sheets as 8 + 8 + 4 pages, and Folder mode
published three DXFs × four approved A1 frames as 12 split PDFs with no drawing open.
Linux gate `4ea3c75`, run 39874361985, was green (fmt, clippy -D warnings, tests, wasm).
Its Windows desktop CI job was cancelled after "repository invocation blocked"; there is
no Windows CI evidence. See [phase QA](../UBOTBUILD-CHANGES.md#smart-plotter-batch-jobs).
Real browser quota exhaustion, real printer submission, T12 printer survey (user decision)
and benchmark numbers remain pending; T13 is not merged, awaiting the T13B user decision.

The synchronous `ubplot.publish` API retains its existing result; add `background: true`
to queue a job from the current scan or a supplied `plan`:

```json
{"command":"ubplot.publish","params":{"background":true,"path":"set.pdf","mode":"merged","pagesPerFile":256,"retryLimit":2}}
```

The response is `{"jobId":"…","status":"QUEUED"}`. Omitting `path` uses stem `plot` and
no output directory. `OutputSpec::default()` uses `merged`, 256 pages per file on desktop
and 64 on web. Command `mode: "split"` selects manifest/job output mode `per_sheet`.
Merged output is `<stem>.pdf`, or `<stem>-001.pdf`, `<stem>-002.pdf`, … for multiple groups;
per-sheet names include sheet order and ID, plus source index for multi-source jobs.
Approved split pieces stay together and count toward the page limit.

### Manifest and resume

The manifest is schema `"1.0"`, independent of workspace schema 3. This excerpt follows
`crates/engine/tests/ubplot_queue.rs::manifest_json_has_contract_keys`, constructed with
`PublishManifest::new(&job(), 1234)`: first sheet only, generated job/source hashes and
source entries omitted. Unset artifact fields come from `PublishManifest::new`.

```json
{
  "schema_version": "1.0",
  "profile_id": null,
  "profile_version": null,
  "status": "QUEUED",
  "created_ms": 1234,
  "updated_ms": 1234,
  "sheets": [
    {
      "sheet_key": "KT-01@1",
      "source_index": 0,
      "sheet_id": "KT-01",
      "order": 1,
      "status": "READY",
      "attempts": 0,
      "sha256": null,
      "output_file": null,
      "page_index": null,
      "finished_ms": null
    }
  ],
  "files": [],
  "issues": [],
  "skipped_files": []
}
```

Full manifests also carry `job_id`, `source_hash`, `sources` and per-sheet `issues`.
Each source records path/hash, sheet count and source-level issues. Job IDs are the first
16 hex characters of SHA-256 over the source hash plus canonical settings; changing output
settings changes the ID. The first source's sheet key is `<sheet_id>@<order>`; later sources
prefix it with `<source_index>:`. Sheets follow source index then order. Delivered files
record name, page count, SHA-256, `delivered_to` and `delivered_ms`; each delivered sheet's
`page_index` is one-based within that file.

- States are `QUEUED`, `RUNNING`, `CANCELLED`, `DONE`, `FAILED`. BLOCKs and unapproved WARNs
  are refused before enqueue; audited WARN approval still uses `ubplot.approve`.
- Resume checks the stored manifest against the job. It skips recorded delivered groups
  and reuses `DONE` sheet artifacts when their SHA-256 matches. Missing or changed artifacts
  for undelivered sheets become `READY` with `ARTIFACT_REBUILT`; failed sheets become ready
  with attempts reset. Reopening a `DONE` job delivers nothing again.
- Rendering allows the initial attempt plus `retry_limit` retries (default 2, maximum 5).
  Exhaustion marks the sheet failed with `RENDER_FAILED`, `RENDER_TIMEOUT` or
  `SHEET_TOO_LARGE`. A changed/unavailable source fails its pending sheets with
  `SOURCE_CHANGED`/`SOURCE_UNAVAILABLE`; other sources can continue.
- A group containing failed sheets is not delivered (`FILE_WITHHELD`); other groups can
  finish. A failed merge withholds that group (`MERGE_FAILED`). The job ends `FAILED` if
  sheets failed or manifest BLOCK issues remain. No partial group is delivered as complete.
- Artifacts are committed before the manifest. Desktop uses exclusive `.part` files and
  rename, publishing the manifest last; web uses one IndexedDB transaction for artifacts
  and manifest, then waits for completion. Storage/delivery failure retries within the
  limit, then reports `STORE_WRITE_FAILED`. The app remains usable.

Delivery precedes recording its receipt. A crash in that interval can leave a delivered
file without a recorded receipt; the manifest cannot guarantee exactly-once external
delivery. Already recorded browser downloads are not repeated on resume, but the browser
may require permission for multiple downloads. Removing a job removes stored progress and
artifacts, not PDFs already delivered.

### Commands and workers

`ubplot.jobs` takes an `action`; all actions except `list` require `jobId`:

| Action | Result / behavior |
|---|---|
| `list` | Array of summaries: `jobId`, status, done/failed/total and `updatedMs` |
| `status` | One summary |
| `manifest` | Full manifest under `manifest` |
| `resume` | Runs the stored job synchronously and returns `manifest`; requires the same current drawing for a drawing job, or rereads files for a job with `folder.json` |
| `cancel` | Cancels a queued job or requests cancellation of a running worker |
| `remove` | Removes a job that is not `RUNNING` |

Programmatic `resume` remains synchronous even with `background: true`; it does not open
a dialog. The UI progress panel has its own worker resume path. Desktop renders off the UI
thread and supports cooperative cancellation/deadlines. Interrupted jobs offer Resume or
Discard at startup; they do not resume automatically. Web calls at most one sheet step per
frame, waits for pending storage, and requests repaint. A single sheet cannot be preempted
on web; exceeding its time budget adds WARN `SHEET_SLOW`. After reload, reopen the same
source drawing/hash before choosing Resume.

### Storage and limits

Desktop GUI jobs use `<settings dir>/jobs/<job_id>/manifest.json`; `sheets/` holds
`job.json` (plans/settings) and per-sheet PDFs. The settings root is the same as for
plotter definitions above. CLI defaults to `<OUT directory>/.ubplot-jobs`, overridable
with `--jobs-dir`. Web uses database `cadcraft`, version 2, store `print-jobs`, with
`<job_id>/manifest` and `<job_id>/artifacts/<name>` keys. Its memory mirror is preloaded
before use; pending writes are acknowledged only after IndexedDB completes. Without a
persistent host store, engine jobs use memory and report `JOBS_NOT_PERSISTED`; reload
resume is unavailable. If web IndexedDB cannot open, only that in-session store remains.

| Constant / setting | Limit |
|---|---|
| `queue::types::MAX_JOB_SOURCES` | 5,000 sources/job |
| `queue::types::MAX_JOB_SHEETS` | 20,000 sheets/job; also bounds queued/listed jobs |
| `queue::types::DEFAULT_RETRY_LIMIT` / `MAX_RETRY_LIMIT` | 2 / 5 retries |
| `queue::types::MAX_MANIFEST_BYTES` | 64 MiB manifest or stored job definition |
| `cadcraft_doc::MAX_PLOT_PAGES` | 1,000 pages/PDF; `pages_per_file` must be 1–1,000 |
| `cadcraft_doc::MAX_PDF_BYTES` | 512 MiB/PDF; also bounds combined merge inputs |
| `cadcraft_doc::MAX_PAPER_MM` | 15,000 mm per paper edge |
| `SheetBudget::desktop()` | `max_prims` 50,000,000; `max_bytes` 512 MiB; `time_ms` 300,000 |
| `SheetBudget::web()` | `max_prims` 10,000,000; `max_bytes` 128 MiB; `time_ms` 300,000 |
| Web `job_store::MAX_RECORDS` / `MAX_BYTES` | 50,000 preloaded records / 128 MiB; bounded writes and mirror |
| Folder `MAX_DEPTH` / `MAX_SCAN_ENTRIES` | 32 levels / 50,000 scanned entries |
| Folder `MAX_MODEL_ENTITIES` | 1,000,000 model entities considered for registered frames |
| Folder `MAX_DWG_BYTES` / `MAX_DXF_BYTES` | 512 MiB DWG / 1 GiB DXF source |

Primitive/output-byte budgets are memory proxies, not a per-thread RSS limit. Folder and
CLI `pagesPerFile`/`--pages-per-file` currently accept only 1–256, default 256, even though
the generic queue supports 1,000. Grouping keeps a sheet's approved split pieces together,
so they can exceed the requested `pages_per_file` target; the PDF page cap still applies.

### Folder mode and CLI

Desktop/CLI `ubplot.folder` scans DWG/DXF names case-insensitively in relative-path order
(lowercase byte order, original path as tie-breaker), then sheet order. It takes `folder`,
`out`, optional `recursive` (true), `maxDepth` (16), `maxFiles` (1,000), `mode` (`merged` or
`split`/`per_sheet`), `pagesPerFile` (256), `layouts` (false) and `background` (false).
`out` must differ from the root; an output subtree is excluded from scanning. Links and
Windows reparse points, empty/invalid files, read errors and scan limits appear in
`skipped_files` with reasons. The skipped list is bounded; its final entry reports when
further omissions are not listed. Drawings without registered frames receive
`NO_FRAMES_FOUND`. Default Block scans require registered non-generic frame templates;
`layouts: true` appends paper-space layouts after the registered Block sheets.

A folder job has one manifest and `job.json`, plus a `folder.json` artifact containing
JSON `true`. This is a loader marker, not a folder configuration file. Synchronous Folder
execution and `ubplot.jobs resume` use `FolderLoader`, retaining only the current drawing
and checking its bytes against the queued hash before render. Sources are read-only;
output cannot overwrite a source. The desktop UI background pump picks `FolderLoader` for
jobs that carry `folder.json` (`worker.rs`, `ubplot_worker::folder_job_resumes_with_folder_loader`)
and a current-drawing snapshot otherwise, so a Folder job also runs and resumes with no
drawing open. Web Folder mode returns a desktop-only error.

```text
cadcraft-cli ubplot --folder DIR --out DIR [--mode merged|per-sheet] [--pages-per-file N] [--max-files N] [--max-depth N] [--jobs-dir DIR] [--templates FILE] [--layouts]
cadcraft-cli ubplot FILE.(dwg|dxf) --out OUT.pdf [--mode merged|per-sheet] [--pages-per-file N] [--jobs-dir DIR] [--templates FILE]
```

`--templates FILE` reads strict `FrameRegistry` JSON, capped at 2 MiB
(`MAX_TEMPLATE_BYTES`), and overrides saved templates for that run without saving the
registry. Single-file CLI jobs also store `folder.json` so resume rereads the source.
Repeating an unchanged command resumes the same job; delivered files retain their mtime.
The CLI prints manifest JSON to stdout. Exit is 0 for `DONE`, 1 for a failed/cancelled job
or execution error, and 2 for invalid arguments or an invalid template file.

Source coverage includes `ubplot_queue::manifest_json_has_contract_keys`,
`ubplot_jobs::queued_job_resumes_to_done_with_manifest`,
`ubplot_folder::sp06_small_folder_resume`, web
`job_store::tests::reload_resumes_without_redelivering_files` and CLI
`ubplot_cli::cli_ubplot_rerun_resumes_and_does_not_duplicate`. These names describe source
assertions; this documentation update did not execute them.

## Printers

Built-in PDF/PNG/SVG and user `.plotter.json` devices produce files. UBPLOT batch jobs
produce PDFs; `DirJobStore::deliver` writes files and the web job store downloads them.
No system-printer host, installed-printer enumeration or OS spool submission is wired in
this checkout, and `ubplot.printers` is not registered. The `PrintJob.printer` field does
not implement printing. Device limits above come from plotter definitions, not a driver
capability query. No different printer is selected silently.

The GĐ5 T12 printer capability survey is pending a user decision. Driver paper sizes,
roll lengths, OS PDF-handler behavior and real-printer smoke evidence have not been
established; no capability survey table or spool-success claim is available.

## Benchmark

No batch benchmark numbers are available. The GĐ5 T13 plan names the ignored
`ubplot_bench::queue_bench_1000_sheets` (1,000 sheets) for an explicit release performance
run in CI's `perf` job, along with `sp06_folder_1000_files_5000_sheets`. In this checkout,
`crates/engine/tests/ubplot_bench.rs` is absent and
`.github/workflows/ubotbuild-ci.yml`'s `perf` job runs workspace restore/switch/commit
measurements only. The batch test and its CI invocation must be integrated before that
run can be claimed. Machine, revision, timings, bytes and RSS remain unmeasured here.

## Plot transparency

Objects plot with their transparency (their own, their layer's when ByLayer, the block
reference's when ByBlock) only when asked, as in AutoCAD:

- `pagesetup {layout, plotTransparency: bool}` stores the layout's "Plot transparency" (off by
  default). The Page Setup Manager and the Plot dialog have the checkbox.
- `plot {plotTransparency?: bool}` (and `exportpdf`, `preview`) overrides it for one plot.
  Model-space plots have it off unless asked.
- `PLOTTRANSPARENCYOVERRIDE` decides first: 0 never plots transparency, 1 (default) uses the
  setting above, 2 always plots it.

PDF plots write it as alpha (`/ExtGState` with `/ca` and `/CA`), PNG plots blend it, SVG plots use
`opacity`. A plot without transparency is written exactly as before. The DXF plot layout flags
have no bit for the option, so it isn't saved in DXF or DWG files.

On screen, `TRANSPARENCYDISPLAY` (on by default; the status bar's Transparency button) shows
transparency; PNG and SVG exports of a drawing always draw it.

## Print profiles

UBPLOT resolves settings in this order, with later levels overriding unlocked values:
Platform → Company → Project → SheetSet → Drawing → User. Platform is built in.
The current host chain loads Company/User `default`, the active drawing's profile,
and its optional `project_id`. SheetSet exists in the schema but has no host binding yet.
Profiles are separate settings documents, never drawing data or bytes written into DWG.

Locks are JSON Pointers such as `/device`; a later level cannot change a locked field
or replace/delete its ancestor. Invalid level patches are skipped as a whole with
INFO `PROFILE_LEVEL_SKIPPED`. Effective results include per-field origins, lock owners
and a 16-character settings version. Company locks are local policy, not a security
boundary against someone who can change the settings store.

`ubplot.profile` supports `effective`/`explain`, `get`, `set`/`unset`, `lock`/`unlock`,
`import` and `export`. Set/unset accept User, Drawing or Project; locks are managed at
Company. Imports contain a profile document. For example:

```json
{"command":"ubplot.profile","params":{"action":"set","level":"user","key":"naming","value":"{stem}-{sheet_id}"}}
```

Naming accepts `{stem}`, `{sheet_id}`, `{order}` and `{part}`. Scan/publish explicit
parameters and imported profiles obey locks too. The Print Profile dialog shows origins
and disables locked editors; JSON calls never open it. MCP and LISP cannot change
Company policy. Other MCP profile changes still obey inherited locks.

Desktop stores schema-1 documents under `<settings>/print-profiles/<level>/`, with
atomic replacement and a last-good copy. Invalid/future documents are preserved when
possible and recovery reports a notice; without a valid backup the level is skipped.
The web host currently uses memory and reports **Print settings last for this visit**.
Profile documents are limited to 256 KiB, depth 32 and 256 locks per level.
Unavailable devices use the existing PDF fallback with WARN `DEVICE_UNAVAILABLE`.

## Templates and baseline import

`ubplot.templates` lists versioned frame templates. Registry schema `"2"` stores
`id@version`, Draft/Approved/Superseded status, source filename/SHA-256/import time,
and a successor link. Legacy schema `"1"`/`"1.0"` migrates with a notice. Matching uses
approved templates, preferring the highest matching version of the selected id.
Older versions remain addressable; sheets using a superseded version warn
`TEMPLATE_SUPERSEDED`. The desktop host persists the registry separately from drawings.

`ubplot.templates {action: "audit", path}` reads a template without rewriting it;
web callers can supply `{name, data}` (base64) instead. Reports contain source hash,
units, blocks/attribute definitions, candidate closed outlines and frame rules,
layouts and missing fonts. Baseline import requires `action: "import"`, the same
source, the audited `sha256`, explicit `picks` and `approve: true`. Each pick provides
`block`, `template_id`, `sheet_id_tags`, `revision_tags` and `revision`.
Unchanged id/source pairs are retained. Changed sources add a version and supersede
prior active versions without deleting them. Template input is capped at 64 MiB,
10,000 blocks and 200,000 scanned entities; the registry has at most 1000 templates.
Ordinary JSON registry import is also supported and merges by id/version.

`verify` reports registry issues; `exportLisp` returns approved template definitions.
`ubplot.report {plan, format: "json"|"csv"}` reports sheet revisions, template ids,
status and issue codes; CSV includes a UTF-8 BOM. LPC fixture and AutoCAD round-trip
validation remain external QA, not guarantees made by an imported baseline.
