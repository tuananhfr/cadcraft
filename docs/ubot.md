# U-BOT toolkit

The U-BOT toolkit lays out, swaps, counts and checks LPC U-BOT boxes on 2D slab plans. Batch 1
adds eight commands to UbotCAD, a shared Rust core and a sidecar program for other CAD hosts:

- `crates/ubot` (`cadcraft-ubot`, layer L1) is the deterministic core: catalog, ring and footprint
  geometry, grid fill, check, count and replace planning. It depends only on `cadcraft-geom`,
  `serde`, `serde_json` and `sha2`, and knows nothing about drawings.
- `crates/engine/src/cmd/ubot/` holds the UbotCAD commands. They read the drawing, call the core
  and write on a clone of the drawing that becomes current in one step, so one Undo restores the
  drawing and a failure leaves it untouched. No existing command changed behaviour.
- `apps/ubot-core` is the sidecar: one JSON request on stdin, one JSON response on stdout, the same
  core. The AutoCAD adapter is an external .NET add-in in the ubot-build repo; it calls this
  sidecar and writes the same drawing data, so a drawing can move between the two CADs.

Coordinates in the core are millimetres. Commands convert with `unit_scale`, millimetres per
drawing unit (see [Units](#units)). Layouts are 2D in the WCS; sloped or stepped slabs and UCS are
out of scope.

## Commands

All eight are in **Tools ▸ U-BOT** and on the U-BOT ribbon tab. Command-line prompts and
history are English, like the rest of the engine; ribbon labels are translated.

| Command | Command id | Does | Writes |
| --- | --- | --- | --- |
| `UBLIB` | `cad.tools.ublib` | Lists the catalog in use; loads a catalog JSON over the built-in one; resets | No |
| `UBZONE` | `cad.tools.ubzone` | Tags closed polylines and circles as zones | Zone xdata |
| `UBFILL` (alias `UBOT`) | `cad.tools.ubfill` | Lays boxes in a slab boundary, with preview | Boxes, layout tag |
| `UBREPLACE` | `cad.tools.ubreplace` | Swaps boxes to another product at the same centre | Boxes |
| `TUBAB` | `cad.tools.tubab` | LPC's TUBxy: swaps references of one block for another | Boxes |
| `UBCOUNT` | `cad.tools.ubcount` | Counts boxes per product | No |
| `UBLEGEND` | `cad.tools.ublegend` | Inserts the count as a TABLE | One TABLE |
| `UBCHECK` | `cad.tools.ubcheck` | Reports box, layout and zone issues | No |

### Preview and commit

The writing commands (`UBFILL`, `UBREPLACE`, `TUBAB`) share one contract:

1. Select. Esc writes nothing.
2. Read the selection into a core request and hash it (`inputHash`, SHA-256 of the canonical JSON:
   sorted keys, numbers rounded to 1e-6).
3. Compute the changes deterministically in the core.
4. Preview. UBFILL draws the footprints of new boxes and marks invalid and uncertain cells with
   their first reason, and prints `UBFILL: +N boxes, -M boxes, U updated, K invalid cells, C
   uncertain cells`.
5. `Apply?` — UBFILL offers `Yes/No/List` (List prints the problem cells with their reasons
   and centres); UBREPLACE and TUBAB offer `Yes/No`. Esc or No writes nothing.
6. Write everything in one commit, after planning again. If the plan's hash differs from the
   preview, the command fails with `CONFLICT` and writes nothing.

From JSON (`engine.execute`, scripts, MCP) the same commands take `mode: "preview"` (the default;
returns the plan, writes nothing) or `mode: "commit"` with an optional `expectHash`, the
`inputHash` of the preview. A mismatch fails with `CONFLICT`, like `If-Match`. Running a fill again
on unchanged input gives an empty diff and writes nothing (`UBFILL: nothing to change.`, no undo
step).

```json
{"command":"ubfill","params":{"boundary":"2F","product":"ubot-h13-7","units":"mm"}}
{"command":"ubfill","params":{"boundary":"2F","product":"ubot-h13-7","units":"mm","mode":"commit","expectHash":"<inputHash>"}}
```

### UBLIB

`{action?: list|load|reset, path?, text?}`. `load` takes a catalog file (`path`, desktop only, at
most 16 MiB) or its JSON (`text`) and lays it over the built-in catalog: entries with the same id
replace the built-in ones, new ones are appended. The loaded catalog lasts for the session; it is
not saved in the drawing or the profile. Typed at the prompt, UBLIB lists the products.

### UBZONE

Prompts: `Select closed polylines or circles`, `Zone type [Exclusion/Solid/Column/Wall/Opening/AltProduct]`
(default Exclusion), and `Alternative product id` for an alt-product zone. JSON:
`{handles?, type, clearanceMm?, altProduct?, units?}`; `clearanceMm` is 0–10 000 mm, and without
it the zone uses the rulepack's exclusion clearance. The zone id is 16 hex of SHA-256 over the
handle and ring, so a copied or reshaped zone gets a new id. Zones must be closed LWPOLYLINEs or
CIRCLEs in model space, off locked layers. A ring that would fail the core (self-intersecting,
zero area) is refused when tagging.

Zone types: `exclusion`, `solid`, `column`, `wall` and `opening` keep boxes out (with the zone's
clearance); `alt_product` changes valid cells inside it to the zone's product, for example low
boxes around a column head.

### UBFILL / UBOT

Prompts: `Select slab boundary` (a closed LWPOLYLINE in model space), `Select holes and zones`
(Enter for none; closed polylines or circles inside the slab), `Drawing units [MM/CM/M/IN]` when
INSUNITS cannot give them, `Box type` (a product id), `Rib` (the rulepack's choices), then the
preview and `Apply? [Yes/No/List]`.

JSON: `{boundary, holes?, product, rib?, pitchX?, pitchY?, angle?, origin?, edgeClearance?,
exclusionClearance?, units?, reset?, mode?, expectHash?}` →
`{mode, inputHash, layoutId, previousLayoutId, reset, layout, summary, add, update, remove,
problems, warnings, written?}`.

- **Grid.** Pitch defaults to box size plus rib (520 + 140 = 660 mm). The default origin is the
  top-left corner of the boundary's bounds in the rotated grid frame, moved half a rib inwards.
  Cells run top-down, then left to right; this order fixes the write and report order. Cell
  `(0, 0)` is centred at `origin`.
- **Cells.** A cell is valid when its rotated footprint is inside the boundary and outside every
  hole, keeps the edge clearance, and keeps each zone's clearance without touching it. Invalid
  reasons: `OUTSIDE_BOUNDARY`, `IN_HOLE`, `EDGE_CLEARANCE`, `IN_ZONE`, `ZONE_CLEARANCE`,
  `OVERLAPS_EXISTING`. Uncertain reasons: `CURVED_EDGE_TOLERANCE` (within the 0.5 mm arc
  subdivision band) and `NEAR_LIMIT` (within 0.5 mm of a threshold). Cells straddling the
  boundary are reported as `OUTSIDE_BOUNDARY`.
- **Zones.** Every tagged zone in model space is used; the core keeps those whose extent comes
  within 10 000 mm of the slab, so zones of other floors neither change nor block this layout.
  A broken zone in reach fails the fill with an error naming the zone; it is never dropped
  silently. Selected untagged holes are tagged as `opening` zones on commit, keeping this run's
  edge clearance, so later fills and UBCHECK see the same geometry. The `zones` parameter is
  ignored with the warning `ZONES_PARAM_IGNORED`.
- **Existing boxes.** A tagged box belongs to the layout when its layout id is this layout's (or
  the copied boundary's earlier one) and its centre is inside the boundary. A box of the layout
  moved by more than 1 mm or turned is kept and reported `MANUAL_MOVED` (info); `reset: true`
  pulls it back. A deleted box's cell is filled again; draw an exclusion zone to keep it empty.
  Every other box — untagged `UB-1..UB-4` or `UBOT_<SKU>` references, boxes with malformed
  U-BOT data, other layouts' boxes, every cell of a tagged MINSERT — is only an obstacle and is
  never changed.
- **Re-runs.** Origin, angle and pitch default to the grid stored on the boundary (the stored
  pitch only while product and rib stay the same). When the grid changes, the run resets and says
  `Grid changed: boxes will be re-laid on the new grid (manual moves are lost).` (warning
  `GRID_CHANGED`). An explicit `reset: false` keeps the old boxes where they are, and every one of
  them then reads as moved by hand.
- **Copied floors.** A boundary whose stored handle is another object's is a copy: it gets a new
  layout id, and copied boxes inside it are moved to the new id. Boxes of the original floor
  outside the new boundary are not touched.
- **Blocks.** A product mapped to a block the drawing has uses that block and its anchor. Otherwise
  UBFILL defines `UBOT_<SKU>` (SKU uppercased, characters outside `[A-Z0-9]` become `_`, for
  example `UBOT_H13_7`): the footprint square drawn in millimetres with its origin at the centre,
  inserted at scale 1/unit_scale. Boxes go on layer `UBOT` (colour 55), created when missing.
  If the boundary, a hole to tag or a box to change is on a locked layer, the command refuses
  before it writes anything.

### UBREPLACE

Prompts: `Select boxes`, `Drawing units` when needed, `Replace with` (a product id), summary,
`Apply? [Yes/No]`. JSON: `{handles?, to, allowResize?, units?, mode?, expectHash?}`. Targets are
U-BOT boxes only: tagged, legacy-mapped or `UBOT_<SKU>` references. The new box keeps the centre,
orientation, mirror signs, layer, colour, attributes and instance tag, and is drawn at true size.
A product of another size is refused unless `allowResize: true`; then the warning `box size
changed: run UBCHECK to re-check cells` asks for a check, because cells are not re-checked by
UBREPLACE. A legacy block without a product is assumed to be the size of the new product
(`LEGACY_BLOCK` warning). A stretched box whose old or new block has an anchor offset is refused,
since its centre is ambiguous. Any refusal stops the commit; nothing is replaced.

### TUBAB

The LPC TUB12…TUB43 commands as one command. Prompts: `Select boxes`, `Replace block` and `With
block` (both default to the last run's), summary, `Apply? [Yes/No]`. JSON:
`{handles?, from, to, mode?, expectHash?}`; `from` is a block name (any case) or product id, `to`
an existing block name or a product id (its mapped block, else `UBOT_<SKU>`, defined when missing).
References of other blocks in the selection are left out. Insert point, rotation, raw scale,
layer and attributes are kept, and a box's product tag follows the new block. When the two blocks
anchor differently the box centre is kept instead of the insert point. External references and
locked layers are refused.

### UBCOUNT

`{boundary?, units?}` → `{rows: [{productId, name, tagged, legacy, total}], total, boundary?}`.
Counts model space through nested blocks (32 levels) and every MINSERT cell, multiplying through
nested inserts; a cluster block holding boxes is never counted itself. Tagged boxes count under
their product; `UBOT_<SKU>` and legacy-mapped blocks count under their block's product, or under
the block name when the mapping has none. With `boundary` (a closed polyline handle), only boxes
whose footprint centre is inside count, which is how a floor's "N hộp" note is made. Rows follow
catalog order, then unknown blocks by name. Typed at the prompt, UBCOUNT counts all of model
space.

### UBLEGEND

Prompts: `Drawing units` when needed, then `Insertion point`. JSON: `{at, textHeight?, boundary?,
units?}` → `{handle, rows, total}`. Inserts a TABLE in the current space and table style: title
`THỐNG KÊ HỘP UBOT`, header `Loại hộp | Số lượng (hộp)`, one row per counted box type and `Tổng`.
These strings are drawing content in the LPC sheet convention and are not translated. Default
text height is 250 mm in drawing units. With nothing to count it writes nothing.

### UBCHECK

`{units?}` → `{issues: [{code, severity, handle, at, message}], counts: {CODE: n}, total}`. Prints
one line per issue (up to 1 000; the response has all) and a summary. Issues are sorted top-down,
then left to right; `handle` and `at` locate them. Each bad object is reported once.

| Code | Severity | Meaning |
| --- | --- | --- |
| `DUPLICATE_ID` | error | Two boxes carry the same instance id |
| `DUPLICATE_LAYOUT` | error | Two boundaries carry the same layout id (the lowest handle owns it) |
| `XDATA_INVALID` | error | U-BOT data that cannot be read or used: wrong field count or type, unplaceable box, broken ring, unusable stored grid |
| `UNKNOWN_PRODUCT` | error | A box or alt-product zone names a product the catalog lacks |
| `OVERLAP` | error | Two box footprints overlap |
| `OUTSIDE_BOUNDARY` | error | A layout's box is not inside its slab, or overlaps a hole |
| `IN_ZONE` | error | A box touches a keep-out zone |
| `CLEARANCE` | error, or warning within 0.5 mm | A box is closer than the clearance to a slab edge, hole or zone |
| `STALE_LAYOUT` | warning | Slab, holes or zones changed since the layout was filled |
| `LEGACY_BLOCK` | warning | A legacy-mapped block without U-BOT data |
| `MANUAL_MOVED` | info | A layout's box was moved by hand from its grid cell |

## Units

`unit_scale` comes from INSUNITS: 1 → 25.4, 4 → 1, 5 → 10, 6 → 1000. Any other INSUNITS,
including 0, needs `units: mm|cm|m|in` (or the `Drawing units` prompt); units are never guessed.
UBFILL, UBREPLACE and UBCHECK always need them. UBCOUNT needs them only with `boundary`, and
UBLEGEND only with `boundary` or without `textHeight`. A plain UBCOUNT needs no units, because
counting does not depend on them. UBZONE checks ring shapes at 1 mm per unit when INSUNITS gives
no scale and `units` is absent; tagging does not depend on units.

## Catalog and rulepack

The built-in catalog `lpc-builtin` version `draft-1` is a draft (`approvalStatus: "draft"`):

| Product id | SKU | Footprint (mm) | Slab (mm) | Source |
| --- | --- | --- | --- | --- |
| `ubot-h13-7` | H13+7 | 520×520 | 270 | 260626_LPC_ST_TKBVTC.pdf p.43 (KC-UB-20) |
| `ubot-h10-7` | H10+7 | 520×520 | 240 | 260626_LPC_ST_TKBVTC.pdf p.43 (KC-UB-20) |
| `ubot-h20-7` | H20+7 | 520×520 | 350 | LPC-250811-BVTC… p.60 (KC-UB-2) |
| `ubot-h17-7` | H17+7 | 520×520 | 320 | LPC-250811-BVTC… p.60 (KC-UB-2) |

Box height is not set. Block mappings `UB-1`…`UB-4` (the LPC "THAY HOP UBOT" template) use anchor
`top_center` and have no product yet. Rulepack `lpc-draft-1` version 1: rib 140 mm by default,
choices 120 and 140, edge clearance 0, exclusion clearance 0, `approvalStatus: "draft"`. Every
layout records `rulepack@version` (`lpc-draft-1@1`). Catalog JSON uses schema `ubot.catalog/1`
with camelCase fields; anchors are `center`, `top_center` or `{"offset": {"xMm", "yMm"}}`.

## Drawing data

U-BOT data is entity xdata of the registered application `LPC_UBOT`. Fields are read by position
in this fixed order; a wrong count or type makes the record invalid (`XDATA_INVALID`), never
guessed. Other applications' xdata keeps its place. A drawing opened without the toolkit stays
editable and the data follows its object. Strings are at most 255 bytes; a longer id is refused,
never cut.

| Carried by | Fields after `1001 LPC_UBOT` |
| --- | --- |
| Box (INSERT) | `1000 "ubot/1"`, `1000 "I"`, `1000 instance_id`, `1000 product_id`, `1000 layout_id`, `1070 count_multiplier` |
| Slab boundary (closed LWPOLYLINE) | `1000 "ubot/1"`, `1000 "L"`, `1000 layout_id`, `1071 revision`, `1000 product_id`, `1000 rulepack@ver`, `1040 origin.x`, `1040 origin.y`, `1040 angle_deg`, `1040 pitch_x_mm`, `1040 pitch_y_mm`, `1040 unit_scale`, `1000 geometry_hash`, `1000 boundary_handle` |
| Zone (closed LWPOLYLINE or CIRCLE) | `1000 "ubot/1"`, `1000 "Z"`, `1000 zone_id`, `1000 zone_type`, `1040 clearance_mm` (−1 = the rulepack's), `1000 alt_product_id` (`""` for none) |

- `instance_id` is `<layout_id>:<i>:<j>`, the grid cell, not the DWG handle, so a re-run finds the
  same box. `count_multiplier` is at least 1.
- `layout_id` is 16 lowercase hex of SHA-256 over the boundary handle, its geometry hash and the
  clock, made at the first fill and read back afterwards. `revision` goes up by one per commit
  that changes something. `origin` is in drawing units; pitches are millimetres.
- `geometry_hash` covers the boundary and the zones within reach, in any order; UBCHECK compares
  it to report `STALE_LAYOUT`.
- `ubot_xdata_survives_dxf_round_trip` (`crates/engine/tests/ubot_infra.rs`) saves a drawing to DXF
  and reopens it with the data intact. No automated test covers DWG.

## Sidecar protocol

`ubot-core` reads one request from stdin (at most 64 MiB; one byte more is refused) and writes one
response line to stdout. It keeps no state between calls.

```json
{"schema":"ubot.req/1","op":"fill","catalog":{…optional overlay…},"request":{…}}
{"schema":"ubot.res/1","ok":true,"result":{…}}
{"schema":"ubot.res/1","ok":false,"error":{"code":"LIMIT","message":"…"}}
```

| `op` | `request` | `result` |
| --- | --- | --- |
| `version` | none | `{core, schema, catalog: "<id>@<version>"}` |
| `catalog` | none | the effective catalog |
| `fill` | `FillRequest` | `FillResult`: `inputHash`, `layout`, `add`, `update`, `remove`, `keep`, `problems`, `summary`, `warnings` |
| `check` | `CheckRequest`: `unitScale`, `instances`, `layouts`, `zones` | the issue list |
| `replace` | `ReplaceRequest`: `unitScale`, `targets`, `toProductId`, `allowResize` | `{changes, refused, warnings}` |
| `count` | `{unitScale, items: [{item, center: [x, y]}], boundary?}` | `{rows, total}` |

`catalog`, when present, is laid over the built-in catalog as UBLIB does. Field names are
camelCase. Zone types, count sources, severities and approval statuses are snake_case. Cell
statuses, reasons and issue codes are SCREAMING_SNAKE_CASE. Rings are `{"vertices": [[x, y, bulge], …]}` in drawing units, closing
implicitly. Error codes: `BAD_JSON` (not JSON or not UTF-8), `BAD_SCHEMA`, `UNKNOWN_OP`,
`BAD_INPUT`, `LIMIT`, `UNKNOWN_PRODUCT`, `GEOMETRY`. The exit code is 0 when the response is
`ok` and was written, otherwise 1.

## Limits

Errors are explicit; nothing is cut silently.

| Limit | Value |
| --- | --- |
| Grid cells tried per fill | 100 000 |
| Boundary, hole and zone vertices after arc subdivision | 20 000 |
| Existing boxes per fill (MINSERT cells counted) | 100 000 |
| Boxes checked by UBCHECK | 100 000 |
| Edge, exclusion or zone clearance | 0–10 000 mm |
| Block nesting followed by counts | 32 levels |
| Block references a located count expands | 1 000 000 |
| Sidecar request | 64 MiB |

Arcs are subdivided with a 0.5 mm tolerance. The performance target is 10 000 boxes in under 2 s
in a release build. The ignored tests hold it:
`cargo test --release -p cadcraft-ubot --test fill_perf -- --ignored` and
`cargo test --release -p cadcraft-engine --test ubot_count_check -- --ignored`. The CI gate does
not run them.

## Ribbon

The UbotBuild workspace has a `ubot` tab ("U-BOT") right after Home, in
`crates/ui-egui/src/wsprofile/presets/UBOTBUILD.json`:

| Group (English source) | Vietnamese | Commands |
| --- | --- | --- |
| Box Layout | Bố trí | `ubfill`, `ubzone` |
| Replace Boxes | Thay hộp | `ubreplace`, `tubab` |
| Box Quantities | Thống kê | `ubcount`, `ublegend` |
| Check | Kiểm tra | `ubcheck`, `ublib` |

Icons are drawn in code (`crates/ui-egui/src/icons.rs`). Labels are translated in the Vietnamese
and Ukrainian catalogs. A profile saved by an earlier build gets the tab, because the store keeps a
patch over the preset; see [Limitations](#limitations) for the exceptions.

## Capability matrix

AutoCAD support comes from the external .NET add-in in the ubot-build repo. Nothing there has run
in a real AutoCAD yet; the first test is planned for 2026-10-26. "Not yet verified" below means
built, not run in a real AutoCAD.

| Capability | UbotCAD | AutoCAD |
| --- | --- | --- |
| `UBLIB` list / load / reset | Yes | Not yet verified in a real AutoCAD |
| `UBZONE` (closed polylines and circles) | Yes | Not yet verified in a real AutoCAD |
| `UBFILL` / `UBOT` with preview and one-step commit; re-run unchanged | Yes | Not yet verified in a real AutoCAD |
| `UBREPLACE` (refuses another size; keeps centre and mirror) | Yes | Not yet verified in a real AutoCAD |
| `TUBAB` (replaces LPC TUB12…TUB43) | Yes | Not yet verified in a real AutoCAD |
| `UBCOUNT` through nested blocks and MINSERT | Yes | Not yet verified in a real AutoCAD |
| `UBLEGEND` TABLE | Yes | Not yet verified in a real AutoCAD |
| `UBCHECK` | Yes | Not yet verified in a real AutoCAD |
| `UBVERSION` (adapter, core and catalog versions) | No (adapter only) | Not yet verified in a real AutoCAD |
| `LPC_UBOT` xdata kept through save and reopen | Yes for DXF (tested); DWG not tested | Not yet verified in a real AutoCAD |
| Command-line prompts | English | English, same prompts |
| U-BOT ribbon tab | Yes (UbotBuild workspace) | No |
| LPC TKTHEP, SLD and the StructuralApps/TktNET commands | No (out of batch 1) | No (out of batch 1) |

The adapter does some checks itself until the core has ops for them: ring validation, zone errors
by id, count centres, the UBFILL re-run rules and the per-zone UBCHECK errors. Each core call
blocks AutoCAD's interface until it answers, for at most 60 s.

## Limitations

- **Draft rules.** The catalog, ribs and clearances are provisional numbers, marked draft (chưa
  duyệt), and every layout records the draft rulepack it used. Edge and exclusion clearances of 0
  follow LPC drawings, where boxes sit against solid strips and column heads. The toolkit does
  no structural, punching-shear or deflection design and never decides it.
- **Custom clearances are not stored.** A custom `edgeClearance` or `exclusionClearance` passed to
  UBFILL is not saved in the layout record (§4.1 has no field for it), so UBCHECK checks the
  layout with the rulepack's values. Selected holes keep the run's edge clearance on their zone
  tag.
- **Tagged MINSERT.** UBCHECK checks a tagged MINSERT at its first cell `(0, 0)` only, so issues of
  its other cells go unreported. UBCOUNT counts every cell; UBFILL treats every cell as an obstacle.
- **Units.** UBCHECK, UBFILL and UBREPLACE require units. UBCOUNT with a boundary and UBLEGEND
  (default text height, or a boundary) require units. A plain UBCOUNT needs none.
- **Interactive gaps.** UBCOUNT has no prompts: the boundary is JSON only. Interactive UBZONE does
  not ask for a clearance; `clearanceMm` is JSON only. Interactive UBFILL uses angle 0 and the
  default origin unless the boundary has a stored grid. There is no four-step dialog.
- **Shapes.** Slab boundaries are closed LWPOLYLINEs; zones are closed LWPOLYLINEs or CIRCLEs. HATCH,
  ELLIPSE, SPLINE and column blocks are not zones yet; trace them with BOUNDARY. A tilted OCS
  polyline is not a ring. A box reference with a flipped extrusion is read in WCS coordinates.
- **Boxes inside cluster blocks** are counted by UBCOUNT but are not obstacles for UBFILL, which
  sees model-space references only.
- **A selected hole beyond the slab's reach** is still tagged as an opening zone on commit.
- **Ribbon and saved profiles.** (1) A full-layout or imported workspace without the tab drops it:
  an exported workspace file imported back, or a profile JSON holding `layout` instead of
  `layout_patch`. Reset the Ribbon, or select the UbotBuild workspace again. (2) A profile whose
  tabs were reordered in an old build keeps the old `order` numbers, so U-BOT (order 1) can tie
  with one of them and sit next to it rather than right after Home; it stays visible. In Compact
  and Expanded ribbon modes the U-BOT buttons are text-only, like other ribbon-only commands.
- **Text.** UBCHECK messages are English data and are not translated. The Vietnamese and
  Ukrainian strings need a native review.
- **Not in batch 1.** The other 24 commands of the LPC specification, interactive grid shifting
  and zone locking, approved revision diffs, server calls from CAD (the UbotBuild server has the
  catalog, layout and job API only), offline sync and signed publication.
