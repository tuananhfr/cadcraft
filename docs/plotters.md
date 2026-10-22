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
submission, driver queries and print queues remain GĐ5 work.

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
