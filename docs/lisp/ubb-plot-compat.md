# UBB_PLOT compatibility (SP10)

SP10 is partial: the UbotCAD regression tests are supplied, pending CI. The AutoCAD
comparison requires the user and AutoCAD; no AutoCAD behavior has been observed for
this task. This is an original reference routine, with synthetic test drawings.

| Function or behavior | AutoCAD | UbotCAD |
|---|---|---|
| `ssget` with INSERT filter | not observed yet | native; selection order retained |
| `entget` | not observed yet | native DXF groups; group 50 is **degrees** |
| `entnext` through ATTRIB to SEQEND | not observed yet | adapter: `ubplot-attribs`; ATTRIBs have no entity names |
| `vl-load-com` | not observed yet | native no-op |
| `vlax-ename->vla-object` | not observed yet | unavailable; use native adapter |
| `vla-get-EffectiveName` | not observed yet | adapter: `getpropertyvalue` with `EffectiveName` |
| `vla-get-Rotation` | not observed yet | adapter: `getpropertyvalue` with `Rotation` (radians) |
| `getpropertyvalue` | not observed yet | native limited property set |
| `ubplot-scan` | not observed yet | native adapter to `ubplot.scan` |
| `ubplot-publish` | not observed yet | native adapter to synchronous `ubplot.publish` |
| `ubplot-templates` | not observed yet | native adapter to approved FrameRegistry templates |
| `ubplot-attribs` | not observed yet | native ordered tag/value adapter |
| `(command "-plot")` | not observed yet | unavailable; use `ubplot-publish` |
| `open` / `write-line` / `close` | not observed yet | native; writes require an allowed LISPSANDBOX folder |
| `(ver)` / `getvar "ACADVER"` | not observed yet | native; absent version reports `unknown` |
| Function-value environment detection with `eval` | not observed yet | native; detects `ubplot-scan` |
| `getkword` All / Select | not observed yet | native; All also works through JSON `inputs` |

The group-50 angle convention still needs black-box observation on AutoCAD (Q-G6-2).
The routine avoids group 50 and gets rotation through the appropriate adapter.
No new runtime builtins are introduced.

## Using the reference routine

Review `UBB_PLOT.lsp`, approve its digest through the existing LISP trust workflow,
then APPLOAD it. On UbotCAD the active registry supplies the templates. The default
path scans, writes `UBB_PLOT.json`, and publishes through the native adapter. The
JSON publish manifest is retained in `*ubb-manifest*`. Existing WARN approvals are
required; the routine never approves warnings or bypasses BLOCK issues.

Set `*ubb-output-dir*` to an existing allowed LISPSANDBOX folder on UbotCAD, or
answer the allowed-folder prompt. The routine does not change write permissions.
On AutoCAD the JSON goes beside the drawing (`DWGPREFIX`). The AutoCAD branch only
exports JSON and prints `Plot on AutoCAD: not verified yet`.

`UBB_PLOT` asks All / Select, then filters INSERTs by the template effective names.
This small LISP prompt lets programmatic callers use `inputs: ["All"]`: UbotCAD's
interactive `ssget` uses SELECT and does not consume the LISP `inputs` queue.
`UBB_PLOT_CHECK` prints environment, runtime version, ACADVER, and missing functions.

For portable geometry set `(setq *ubb-portable* T)`. `ubb:plan-json` accepts a
selection set, transforms the four template corners by INSERT scale, rotation and
position (accounting for block base point), reads the sheet ID from template tags,
and emits SheetPlan 1.0 with `job_id: "lisp"` and an empty `source_hash`.

Portable geometry covers approved local rectangles, including rotated, scaled and
mirrored INSERTs. It does not traverse nested or dynamic visibility geometry, detect
layer/enclosing rectangles, or run native preflight. A matching template without
four corners, or a singular/nonuniform INSERT, produces a BLOCKED sheet with
`FRAME_AMBIGUOUS`; it is never silently dropped. Use the default native adapter for
those cases. The selection is capped at 10,000 INSERTs. Native preflight remains
necessary before publishing a portable plan.

`ubplot.templates {"action":"exportLisp"}` returns `{text}` for
`UBB_PLOT_TEMPLATES.lsp`. Save that text and load it on AutoCAD before the routine.
The existing per-block association lists retain their geometry/revision/tag fields
and add `frame` and `paper` entries. Only the latest approved template versions are
exported. No Autodesk assets, fonts, templates or generated drawing files are included.

## Comparison contract

`ubplot.compare {a, b, tol?}` accepts two full SheetPlan JSON objects and returns
`{same, diffs: [{index, field, a, b}]}`. `index` is zero-based. A count mismatch uses
index 0 and field `count`; common sheets are still compared. Each differing field
produces one diff, so an outline change produces one diff for that sheet.

Comparison preserves array order and does not match or sort sheets. Compared fields
are count, sheet_id, mode, block_handle, effective_name, outline, paper_mm, landscape
and frame_template_id. Only outline coordinates use tolerance: each coordinate must
have absolute error at most `tol` (default 1e-6; finite, 0 < tol <= 1). Paper dimensions
are exact. Schema, hashes, status, issues, audit, order numbers, rotation, plot scale,
attributes and layout names are not compared. Plans still undergo schema and input
validation. Diff values are diagnostic strings. This is a scan comparison, not a
rendering or print-equivalence check.

## How to run the AutoCAD comparison

1. Export the active registry with `ubplot.templates {"action":"exportLisp"}` and
   save the returned text as `UBB_PLOT_TEMPLATES.lsp` outside the repository.
2. On AutoCAD, load that file and the reviewed `UBB_PLOT.lsp`, open the same synthetic
   drawing, run `UBB_PLOT_CHECK`, then `UBB_PLOT` with the same INSERT selection order.
   Keep `UBB_PLOT.json` outside the repository.
3. On UbotCAD, save the `ubplot.scan` JSON result for the same drawing bytes,
   registry and selection order as `native.json`, also outside the repository.
4. Run:

   ```text
   cadcraft-cli ubplot --compare UBB_PLOT.json native.json --tol 0.000001
   ```

Exit 0 means equal under the comparison contract; exit 1 prints one JSON diff per
line; exit 2 means unreadable/invalid input or bad arguments. The CLI reads at most
64 MiB per plan. Record the AutoCAD version, date, selection order and diff results
here after observation. Never commit AutoCAD-produced files or screenshots.
