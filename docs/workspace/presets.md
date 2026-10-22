# Workspace presets

Nine presets ship inside the binary (`crates/ui-egui/src/wsprofile/presets/*.json`, loaded by
`wsprofile::presets::preset(id)`). They are the Platform layer of a workspace profile
(`docs/workspace/workspace-profile.schema.json`). Every command id in them is a real engine command
(`command_specs()`) or UI command (`menus::UI_COMMANDS`); a test checks this. No preset references a
Fonts or LISP panel.

`SIMPLE` is the fallback whenever a profile cannot be used. `LPC_COMPANY` is not shipped.

## Status

| Preset | Status |
|---|---|
| `SIMPLE` | published: reproduces the current CADCraft window |
| `AUTOCAD_FAMILIAR` | published |
| `PLANNING_GIS`, `ARCHITECTURE`, `STRUCTURE`, `MEP_FIRE`, `INFRA`, `QS_QA`, `SHEET_PUBLISH` | **draft — needs CAD Manager review** |

The discipline presets are starting points assembled from commands that exist today. Which commands a
discipline needs in front of it, and how the groups are named and ordered, are decisions for each
office's CAD Manager. The profile `status` of these seven is `draft`.

## Conventions

- Panel ids come from `docking::Panel`: `toolSets`, `layers`, `properties`, `externalReferences`,
  `quickCalc`. `canvas` is never in a profile. `dock_tree` is `null` in every preset; the layout is
  built from `panels[]` (all positions are ratios, 0..1).
- A tool bar's `commands` is a flat list. Separators and button groups are not part of it; the renderer
  decides spacing. `SIMPLE.standard` lists the ids of `chrome::TOOLBAR` in display order with the
  group breaks dropped (a test compares the two).
- `ui.toggle.palettes` is dispatched by `menus::run_ui_command` but is not listed in `UI_COMMANDS`; the
  tool bar's Properties button uses it, so `SIMPLE` does too.
- `shortcuts`, `aliases` and `favorites` are empty: the Platform shortcut table is separate data.
- Presets other than `SIMPLE` hide Tool Sets (`toolsets.visible = false`, `toolSets` panel hidden) and
  show the ribbon plus a Quick Access list: `new open qsave saveas plot u redo`.

## SIMPLE

The window as it is today.

| Part | Content |
|---|---|
| Ribbon | hidden, no tabs |
| Tool bars | `standard`, top, 25 commands (see `chrome::TOOLBAR`) |
| Panels | `toolSets` left (dock); `layers` and `properties` right (dock); `externalReferences` hidden |
| Tool Sets | visible, tab Drafting |
| Command line | floating, 3 lines |

## AUTOCAD_FAMILIAR

Ribbon visible, no tool bars, Tool Sets hidden.

Standing ribbon tabs (D6), in order:

| Tab | Groups |
|---|---|
| Home | Draw, Modify, Annotation, Layers, Block, Properties, Utilities, Clipboard |
| Insert | Block, Reference, Data, Count |
| Annotate | Text, Dimensions, Leaders, Tables, Markup, Annotation Scaling |
| Layers | Layers, Visibility, Manage |
| View | Navigate, Viewports, Display |
| Layout | Layouts, Layout Viewports, Page |
| Tools | Inquiry, Select, Settings, Cleanup |
| Output | Quick Plot (`ubplot plot publish pdfbook preview`) |

Contextual tabs, shown only for a matching selection: `Dimension` (`contextual: "dimension"`, group
Edit Dimension) and `Hatch` (`contextual: "hatch"`, group Hatch Edit).

Panels: `properties` and `layers` docked right; `externalReferences`, `quickCalc` and `toolSets` hidden.

## Discipline presets (all draft)

Each discipline preset is AUTOCAD_FAMILIAR plus the additions below, a left-docked tool bar, and the
panel changes listed. The tool bar id equals the column "Tool bar".

| Preset | Added ribbon groups | Tool bar (left) | Extra visible panel |
|---|---|---|---|
| `PLANNING_GIS` | Home: Planning and Survey (`boundary region divide measure measuregeom area`); Insert: Base Maps (`attach xattach xref`) | `survey`: `dist area id list measuregeom count` | `externalReferences` |
| `ARCHITECTURE` | Home: Walls and Openings (`mline mlstyle mledit offset wipeout`); Annotate: Schedules (`table tablestyle dataextraction`) | `architecture`: `mline offset trim extend hatch insert dimlinear mtext` | none |
| `STRUCTURE` | Home: Structural Grid (`xline rectang polygon arrayrect arraypolar arraypath`); Tools: Constraints (`autoconstrain geomconstraint gcfix gchorizontal gcvertical gcparallel gcperpendicular dimconstraint delconstraint constraintbar`) | `structure`: `pline rectang arrayrect dimlinear qdim count` | none |
| `MEP_FIRE` | Home: Routing (`pline 3dpoly spline arc divide measure`); Insert: Symbols (`insert block attdef attedit attsync`); Tools: Take-off (`count countlist counttable dataextraction`) | `mep`: `pline insert count dimaligned leader` | `externalReferences` |
| `INFRA` | Home: Alignment (`xline pline spline arc lengthen align divide measure`); Annotate: Stations (`dimordinate qdim`) | `infra`: `pline xline spline lengthen align dimordinate area` | `externalReferences` |
| `QS_QA` | Tools: Quantities (`count countlist counttable dataextraction eattext datalink datalinkupdate`) and Checks (`compare compare.next compare.previous drawing.inspect`); Annotate: Review Markup (`revcloud wipeout`) | `qa`: `dist area list massprop count qselect` | `quickCalc` |
| `SHEET_PUBLISH` | Layout: Publish (`plot preview publish exportpdf plotstyle plottermanager scalelistedit`) | `sheets`: `layout.new layout.set mview vpclip pagesetup plot publish` | `externalReferences` |

Panels common to the discipline presets: `properties` and `layers` docked right, `toolSets` hidden.

## Changing a preset

Edit the JSON, keep every id real, run `cargo test --release -p cadcraft-ui-egui --test ws_profile`. If
`chrome::TOOLBAR` changes, `SIMPLE.standard` (and `WorkspaceLayout::default()` in `schema.rs`) must follow.
