# UbotBuild changes

This branch (`tuananhfr`) is UbotCAD, the UbotBuild build of [CADCraft](https://github.com/storytold/cadcraft) 0.8.0, based on CADCraft by the ArtCraft team, licensed MIT OR Apache-2.0 (see `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`). Changes from upstream (Apache-2.0 §4b):

- Removed the ArtCraft logos and marks (`docs/brand/` and its `ATTRIBUTION.md` rows) and every link to the ArtCraft Discord and getartcraft.com, as the ArtCraft brand terms require for modified versions: About dialog, the macOS app-menu Discord item and its `ui.discord` handler, README logo, badges, community and Crafting Apps sections, and the Linux/Windows packaging links (now https://ubotbuild.com/).
- About now reads "UbotCAD — based on CADCraft by the ArtCraft team." and links ubotbuild.com. It stays in English until the bundled UI font covers Vietnamese.
- Dropped the translations of the removed strings from every catalog.
- DWG opens and saves in the web build too. Upstream builds the DWG bridge (`crates/dwg`, acadrust) for the desktop only, though it works from bytes in memory and acadrust builds for wasm32; the web build now includes it (about 1 MB more brotli-compressed wasm). To fit large DWGs in wasm32's 4 GB:
  - a picked or dropped file is read directly (`cmd::file::read_data`) instead of through OPEN's base64 `data`;
  - the DWG→DXF text is collected in 8 MB pieces and joined after acadrust's document is freed;
  - the DXF tag list is sized from the line count up front, and the DXF text is freed before the drawing is built from the tags.

  An 86 MB DWG (17,000 entities, 3,055 layers) now opens in the browser in about 70 s at a 3.3 GB peak; before these changes it crashed the tab. A DWG over 90 MB is refused with a message to use the desktop app (`MAX_OPEN_DWG_BYTES`).
