# Interface languages

Choose **Edit → Interface Language** and select **Auto (system language)**, **English**,
**Українська**, **Português (Brasil)**, **Deutsch**, **Español**, **Čeština** (Czech), **Русский**, **Français**, **简体中文**, **Italiano**, **한국어** (Korean), **Svenska** (Swedish), **العربية** (Arabic), or **Tiếng Việt** (Vietnamese).
The interface updates without restarting, including the native macOS menus.
The choice is saved with the other interface preferences (`cadcraft.prefs`, beside the theme):
eframe storage on the desktop, browser local storage on the web. Installations without a saved
choice use Auto.

Auto uses the first supported language in the operating system's preferred locales, or the
browser's `navigator.languages` on the web. BCP 47 and POSIX tags select their primary language:
`uk-UA` → `uk`, `de_AT.UTF-8` → `de`, `es-MX` → `es`, `pt_BR.UTF-8` → `pt-BR`,
`cs-CZ` and `cs_CZ.UTF-8` → `cs`, `it-IT` and `it_CH.UTF-8` → `it`, `ko-KR` and `ko_KR.UTF-8` → `ko`,
`sv-SE`, `sv-FI` and `sv_SE.UTF-8` → `sv`; `vi-VN` and `vi_VN.UTF-8` → `vi`. Unsupported languages fall back to English.
Set `CADCRAFT_LOCALE=cs-CZ` to override the desktop's locale for a launch.
An explicit saved language choice takes precedence over that override.

Automation can select the language through the existing control channel:

```json
{"id":1,"method":"engine.execute","params":{"command":"ui.language","params":{"lang":"cs"}}}
```

The `ui.set` method also accepts `{"interfaceLanguage":"cs"}`. Supported values are `auto`,
`en`, `uk`, `pt-BR`, `de`, `es`, `cs`, `ru`, `fr`, `zh`, `it`, `ko`, `sv`, `ar`, and `vi`; invalid values are rejected without changing the current
selection. Resetting palettes keeps the language choice.

## Translation coverage

The catalogs cover command and menu labels, tool groups and tooltips, the Start page,
file and model-space tabs, status controls, object snaps, drafting prompts, property labels,
dialogs, and the About window's controls. The command reference can be searched by English
command ID, alias, English label, or translated label. Ukrainian and Vietnamese have enforced
coverage for commands, menus, tool sets, snaps, marked UI literals and preset ribbon labels
(with ordinary-context fallback). Vietnamese additionally requires every discovered engine prompt;
Ukrainian prompt coverage is not enforced. Other catalogs report gaps and fall back to English.

Vietnamese has **1,831 catalog rows**, including the U-F ribbon terminology, Quick Plot and
command prompt templates. Translations follow `vi_glossary.tsv`; **native review pending** by
the user/LPC. The P0 regression checks catalog-backed painted text at 1366×768 on the Start
page, AUTOCAD_FAMILIAR with LINE running, Customize, Fonts, Plot, Command Palette and Quick Plot.
It excludes drawing names, command identifiers and English input keywords; dynamic strings
without a catalog key remain outside that check. Native/wasm gates, browser layout QA and
desktop smoke for this milestone are pending lead execution.

Engine prompts use `Prompt::tf` / `PromptTemplate` for named arguments; the UI translates
the template and preserves inserted user values. Literal prompts use the same catalog.
Command registry metadata and stable namespaced IDs are documented in
[command-ids.md](workspace/command-ids.md) and [command.schema.json](workspace/command.schema.json);
legacy IDs remain accepted and dispatch reports outcome codes.

Search accepts Vietnamese without accents (`lop`), English (`layer`) and aliases (`LA`),
with bounded fuzzy matching. Input normalization composes Vietnamese and Latin-1 combinations;
it is not a full Unicode NFC implementation. IME composition suppresses drafting aliases and
empty-Enter repetition; platform Telex/VNI acceptance still needs native/browser QA.

Czech has 1,202 catalog rows, covering the reference catalog and every marked UI
literal, including Options. Four `menu` entries distinguish menu headings from object/command
labels: **Zobrazení / Pohled**, **Kreslení / Kreslit**, **Kótování / Kóta**, and **Modifikace / Upravit**.
The translation is AI-assisted work by @Losminek, reviewed against Czech desktop and CAD UI
conventions and an independent review pass; it has not yet been proofread by a native speaker.

Korean has **1,235 rows**: 1,229 ordinary labels, four contextual menu headings, and
two single-form count templates. It covers the union of the existing catalogs, including
the Ukrainian reference, the current Czech catalog, Options, prompts, dialogs, and tooltips.
The translations are original, AI-assisted Korean CAD terminology (for example, 도면층,
치수, 모깎기, 모따기, 간격띄우기 and 객체 스냅). Independent native proofreading and
platform-specific layout/IME validation remain follow-up work.

Brazilian Portuguese has 1,231 catalog rows and also covers the reference catalog and every marked UI
literal: commands, menus, tool groups and tooltips, object snaps, drafting prompts, dialogs and
Preferences; its gaps are now zero. Its plural rule is `one|other` (0 and 1 are `one`), as CLDR defines
for `pt`, and the catalog's accented letters (`Á Â Í Ó Ú à á â ã ç é ê í ó ô õ ú`) render in the
bundled fonts. The original catalog was contributed by @WladmirJunior; the remaining rows were completed
by @RicSchonfelder with AI assistance and have not yet been proofread by a native speaker.

Drawing titles, layer/layout/block/style names, parameter names and expressions, text in a
drawing, and command-line input are user data and retain their original spelling. Clickable
command keywords retain their English spelling so the text shown matches the input accepted
by the engine. CLI/MCP/control responses, engine diagnostics and command history remain
English. Uncatalogued text also falls back to English. Number formatting and the operating
system's own file-picker controls follow the existing behavior.

The bundled egui fonts cover Ukrainian (`Ґґ Єє Іі Її`), German (`ÄÖÜ äöü ß`), and Czech
(`ÁČĎÉĚÍŇÓŘŠŤÚŮÝŽ áčďéěíňóřšťúůýž`, including typographic quotes `„…“`).
The translations add no font assets and work on the web without system fonts.

Korean needs a font that covers **every Hangul character in its catalog** and the language
picker label. The CJK font loaded in every language (Noto Sans CJK, Microsoft YaHei, PingFang)
usually covers Hangul already, and then nothing more is loaded. Otherwise, only when Korean is
chosen, desktop builds look for a `Hang`/`Kore` face from the optional `CRAFT_FONTS_DIR` input,
then installed fonts: Malgun Gothic/Gulim on Windows, Apple SD Gothic Neo/AppleGothic on macOS,
and Nanum Gothic or Noto CJK on Linux/FreeBSD. Collection faces are checked individually, and
file reads/face scans are bounded. No system fonts or new font assets are redistributed.

Web builds cannot read installed fonts and embed no Korean face (to stay within hosting size
limits), so the web interface stays English when Korean is chosen. Without a font covering the
catalog, the saved `ko` preference is kept but the interface falls back to English; the language
command reports that a font is needed, and the picker uses the readable label **Korean** wherever
Hangul can't be shown. This change does not fix platform IME issues or localize engine output.

Vietnamese requires a system UI font with Vietnamese glyphs on desktop, or optional Noto Sans
from `CRAFT_FONTS_DIR` on the web. No new font assets are bundled by the translation. Without
coverage, the saved `vi` choice is retained and the interface displays English with a reason.

For layout checks, `ui.language {"lang":"qps"}`, `?lang=qps` on the web, or
`CADCRAFT_LOCALE=qps` on desktop enables pseudo-localization. It expands display text while
preserving placeholders and ellipses, stays out of the language picker, and is never saved.

## Adding translations

The implementation follows the display-time catalog pattern used by
[VectorCraft #205](https://github.com/storytold/vectorcraft/pull/205),
[PdfCraft #106](https://github.com/storytold/pdfcraft/pull/106), and
[FilmCraft #238](https://github.com/storytold/filmcraft/pull/238). Its code is local to CADCraft.

- `crates/ui-egui/src/i18n/*.tsv` are UTF-8 with three tab-separated columns:
  `context`, `English source`, and `translation`. Ordinary labels have an empty context.
  Optional `menu` entries translate menu headings via `i18n::menu` / `i18n::tr_menu`;
  without one, the ordinary translation is used. `ribbon` entries use `i18n::ribbon` /
  `i18n::tr_ribbon` with the same fallback; Vietnamese ribbon rows must use glossary terms.
- `@plural` entries hold the language's forms for integer counts: Ukrainian `one|few|many`,
  German and Spanish `one|other`, Brazilian Portuguese `one|other` (0 and 1 are `one`),
  Czech `one|few|other` (1; 2–4; everything else, including 0, 11, 21 and 22).
  Korean and Vietnamese have one form (`other`) for all integer counts.
  The API takes `u64`, so fractional counts are not accepted.
- Register a new catalog in `i18n::LANGUAGES`, add its preference variant and code/name,
  and set its integer plural rule and form count on the `Language` row.
- Use `crate::tl!("English label")` for a literal, `i18n::t(label)` for a known dynamic UI
  label, and `crate::tf!("Version {version}", version = value)` for named placeholders.
  Translate only at the display boundary; never look up arbitrary user names or identifiers.
- Windows retain explicit IDs based on their English source, and tool groups retain their
  source names as keys, so switching languages preserves their state.
- Add a translation attribution row in `ATTRIBUTION.md`, update the language lists, and add
  locale, plural, font-glyph and live-switch tests. Add other catalogs to `REPORTED` in
  `i18n/tests.rs`. `REQUIRED = ["uk", "vi"]`: new UI labels need both rows; new engine
  prompts need a Vietnamese row. Keep template placeholders and English input keywords unchanged.
- Catalogs are embedded and parsed once. Missing or malformed rows cannot crash the app;
  tests reject invalid rows, duplicate keys, changed placeholders, and mismatched ellipses.
- `cargo test -p cadcraft-ui-egui i18n` checks menu/command/tool/snap coverage, every marked
  UI literal, locale resolution, preferences, integer plurals, bundled glyphs and actual
  egui renders with language switching. `cargo xtask assets` checks asset attribution;
  `cargo xtask ci` runs the workspace quality gates.

Translations are original work under `MIT OR Apache-2.0`, written from the meaning of
CADCraft's English strings. Do not copy proprietary localization resources.
