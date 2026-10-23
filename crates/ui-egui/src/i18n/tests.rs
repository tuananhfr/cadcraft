use std::collections::BTreeSet;

use super::*;
use crate::{CadApp, Services};
use cadcraft_engine::Session;
use serde_json::json;

fn placeholders(text: &str) -> BTreeSet<&str> {
    text.split('{').skip(1).filter_map(|s| s.split_once('}').map(|(key, _)| key)).collect()
}

#[test]
fn catalogs_are_nfc() {
    for language in &LANGUAGES {
        assert!(unicode_normalization::is_nfc(language.source), "{} catalog is not NFC", language.code);
    }
    assert!(unicode_normalization::is_nfc(include_str!("vi_glossary.tsv")), "Vietnamese glossary is not NFC");
}

#[test]
fn catalog_is_valid_and_preserves_placeholders_and_ellipses() {
    for language in &LANGUAGES {
        let (catalog, errors) = parse(language.source);
        assert!(errors.is_empty(), "{}: {errors:?}", language.code);
        for ((context, key), value) in catalog {
            assert!(["", "menu", "ribbon", "@plural", "echo"].contains(&context), "unknown context: {context}");
            let forms: Vec<_> = if context == "@plural" { value.split('|').collect() } else { vec![value] };
            if context == "@plural" {
                assert_eq!(forms.len(), language.plural_forms, "{}: {key}", language.code);
            }
            for form in forms {
                assert_eq!(placeholders(key), placeholders(form), "{key}");
                for suffix in ["...", "…"] {
                    assert_eq!(key.ends_with(suffix), form.ends_with(suffix), "{key}");
                }
            }
        }
    }
    let (parsed, errors) = parse("broken\n\tOpen\tВідкрити\n\tOpen\tВідкрити\n\tEmpty\t\n");
    assert_eq!(parsed.len(), 1);
    assert_eq!(errors.len(), 3);
    assert_eq!(tr("uk", "a future untranslated label"), "a future untranslated label");
    assert_eq!(tr("unknown", "Open"), "Open");
}

/// Rows of the Vietnamese glossary: (context, English, Vietnamese); the note column is for reviewers.
fn glossary() -> Vec<(&'static str, &'static str, &'static str)> {
    let rows = include_str!("vi_glossary.tsv").lines().map(|line| line.trim_end_matches('\r'));
    rows.filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let cols: Vec<_> = line.split('\t').collect();
            assert_eq!(cols.len(), 4, "glossary rows need context, English, Vietnamese and a note: {line}");
            (cols[0], cols[1], cols[2])
        })
        .collect()
}

#[test]
fn glossary_is_valid_and_carries_the_uf_tab_table() {
    let rows = glossary();
    assert!(rows.len() >= 140, "{}", rows.len());
    for (en, vi) in [
        ("Home", "Vẽ"),
        ("Insert", "Chèn"),
        ("Annotate", "Ghi chú"),
        ("Layers", "Lớp bản vẽ"),
        ("View", "Hiển thị"),
        ("Layout", "Bố cục in"),
        ("Properties", "Thuộc tính"),
        ("Xref", "Tham chiếu ngoài"),
        ("Plot", "In bản vẽ"),
    ] {
        assert!(rows.contains(&("ribbon", en, vi)), "{en}");
    }
    let mut seen = BTreeSet::new();
    for (context, en, vi) in &rows {
        assert!(["", "menu", "ribbon"].contains(context), "unknown context: {context}");
        assert!(seen.insert((*context, *en)), "duplicate glossary row: {context} {en}");
        assert!(!vi.is_empty(), "{en}");
    }
}

// Both reference catalogs must cover UI labels; engine prompts are required only in Vietnamese.
const REQUIRED: &[&str] = &["uk", "vi"];
const REPORTED: &[&str] = &["pt-BR", "de", "es", "cs", "ru", "fr", "zh", "it", "ko", "sv", "ar"];

fn check(language: &str, what: &str, missing: Vec<String>) {
    if REQUIRED.contains(&language) {
        assert!(missing.is_empty(), "{language}: missing {what}: {missing:?}");
    } else if !missing.is_empty() {
        eprintln!("{language}: {} {what} fall back to English: {missing:?}", missing.len());
    }
}

#[test]
fn translations_cover_commands_menus_tools_and_snap_labels() {
    for language in REQUIRED.iter().chain(REPORTED) {
        check(language, "translations", missing_commands_menus_tools_and_snap_labels(language));
    }
}

fn command_label_sources() -> BTreeSet<String> {
    let mut sources: BTreeSet<_> = crate::menus::MENUS.iter().map(|name| (*name).to_string()).collect();
    for command in cadcraft_engine::command_specs() {
        sources.insert(command.label.to_string());
        sources.extend(command.menu.iter().map(|name| (*name).to_string()));
    }
    for (_, label, path, _) in crate::menus::UI_COMMANDS {
        sources.insert((*label).to_string());
        sources.extend(path.iter().map(|name| (*name).to_string()));
    }
    for groups in [crate::palettes::DRAFTING, crate::palettes::MODELING] {
        for (name, large, small) in groups {
            sources.insert((*name).to_string());
            sources.extend(large.iter().chain(*small).map(|(_, _, label)| (*label).to_string()));
        }
    }
    sources.extend(cadcraft_engine::snap::mode::ALL.iter().map(|(_, name)| (*name).to_string()));
    sources.remove("");
    sources
}

fn marked_ui_sources(file: &str, source: &str) -> BTreeSet<String> {
    let mut sources = BTreeSet::new();
    let tokens = production_ui_tokens(source);
    for i in 0..tokens.len() {
        for (position, translated) in text_slots(&tokens, i, &TextHelpers::new()) {
            if translated
                && let Some(range) = ui_arguments(&tokens, text_call_open(&tokens, i)).get(position)
                && let Some(key) = tokens[range.start].literal.as_ref()
                && !["X", "Y", "B", "UbotCAD"].contains(&key.as_str())
            {
                sources.insert(key.clone());
            }
        }
    }
    if file == "dialogs.rs" {
        // Unit choices and shared style titles use dynamic translation to retain their source values.
        sources.extend(
            crate::dialogs::LENGTH_TYPES
                .iter()
                .chain(&crate::dialogs::ANGLE_TYPES)
                .map(|(_, label)| *label)
                .chain(crate::dialogs::INSERTION_UNITS)
                .chain(["Text Style", "Table Style", "Multileader Style"])
                .map(str::to_string),
        );
    }
    if file == "toolbars/mod.rs" {
        for (id, label) in crate::toolbars::BUILTIN {
            sources.insert((*label).to_string());
            if let Some(groups) = crate::toolbars::builtin_groups(id) {
                sources.extend(groups.into_iter().flatten().map(|(_, _, label, _)| label.to_string()));
            }
        }
        for id in crate::wsprofile::presets::PRESET_IDS {
            if let Some(profile) = crate::wsprofile::presets::preset(id) {
                sources.extend(profile.layout.toolbars.into_iter().map(|bar| bar.label));
            }
        }
    }
    if file == "viewcube.rs" {
        sources.insert(crate::viewcube::NO_TWIST.to_string());
    }
    if file == "wsprofile/mod.rs" {
        sources.extend(
            [
                crate::wsprofile::WORKSPACE_FILE.label,
                crate::wsprofile::startup::FALLBACK_MESSAGE,
                crate::wsprofile::startup::NOT_KEPT_MESSAGE,
                crate::wsprofile::startup::POLICY_MESSAGE,
                crate::wsprofile::store::TAB_CONFLICT_MESSAGE,
            ]
            .map(str::to_string),
        );
    }
    sources
}

fn ribbon_label_sources() -> BTreeSet<String> {
    let mut sources = BTreeSet::new();
    for id in crate::wsprofile::presets::PRESET_IDS {
        let profile = crate::wsprofile::presets::preset(id).unwrap();
        for tab in profile.layout.ribbon.tabs {
            sources.insert(tab.label);
            sources.extend(tab.groups.into_iter().map(|group| group.label));
        }
    }
    sources.remove("");
    sources
}

fn displayed_prompt_key(key: &str) -> bool {
    let mut words = key.split_whitespace().peekable();
    let placeholders_only = words.peek().is_some()
        && words.all(|word| {
            word.strip_prefix('{').and_then(|word| word.strip_suffix('}')).is_some_and(|name| !name.is_empty() && !name.contains(['{', '}']))
        });
    !key.is_empty() && !placeholders_only
}

fn prompt_source_keys(source: &str) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for marker in ["Prompt::new(", "Prompt::tf("] {
        for rest in source.split(marker).skip(1) {
            let argument = widget_text_argument(rest, 0).unwrap();
            let mut rest = argument;
            while let Some(start) = rest.find('"') {
                let mut decoder = serde_json::Deserializer::from_str(&rest[start..]).into_iter::<String>();
                let key = decoder.next().unwrap().unwrap();
                rest = &rest[start + decoder.byte_offset()..];
                if displayed_prompt_key(&key) {
                    keys.insert(key);
                }
            }
        }
    }
    keys
}

fn engine_prompt_keys() -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    let mut directories = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../engine/src")];
    while let Some(directory) = directories.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(directory).unwrap().map(|entry| entry.unwrap().path()).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                directories.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                keys.extend(prompt_source_keys(&std::fs::read_to_string(path).unwrap()));
            }
        }
    }
    let static_count = keys.len();
    for spec in cadcraft_engine::command_specs().iter().filter(|spec| spec.interactive.is_some()) {
        // Fresh drawings prevent commands that finish during begin from changing later prompts.
        let mut session = Session::new();
        session.open_drawing(cadcraft_engine::sample::default_sample(), "Sample", None);
        if session.start(spec.id).is_ok()
            && let Some(prompt) = session.current_prompt()
        {
            let key = prompt.template.map_or(prompt.message, |template| template.source.to_string());
            if displayed_prompt_key(&key) {
                keys.insert(key);
            }
        }
        session.cancel();
    }
    eprintln!("Engine prompt keys: {static_count} static, {} dynamic-only, {} total", keys.len() - static_count, keys.len());
    keys
}

#[test]
fn prompt_source_keys_balance_arguments_and_skip_placeholder_only_templates() {
    let source = r#"
        Prompt::new(if flag { "First, (point)" } else { "Second [point]" }, Accept::POINT);
        Prompt::new(["Third", "Fourth"].get(index).copied().unwrap_or(""), Accept::POINT);
        Prompt::tf("Block \"{name}\": enter an option", vec![("name", value)], Accept::TEXT);
        Prompt::tf("{title} {message}", args, Accept::POINT);
        Prompt::new("", Accept::TEXT);
    "#;
    let expected = BTreeSet::from(["First, (point)", "Second [point]", "Third", "Fourth", "Block \"{name}\": enter an option"].map(str::to_string));
    assert_eq!(prompt_source_keys(source), expected);
    assert!(displayed_prompt_key("(need {axes})"));
    assert!(displayed_prompt_key("{snap} of"));
    assert!(!displayed_prompt_key("{title} {message}"));
}

#[test]
fn every_engine_prompt_has_a_vietnamese_translation() {
    let keys = engine_prompt_keys();
    eprintln!("Vietnamese engine prompt coverage: {} keys", keys.len());
    assert!(keys.contains("Specify first point"));
    let catalog = catalog("vi").unwrap();
    let missing: Vec<_> = keys.into_iter().filter(|key| !catalog.contains_key(&("", key.as_str()))).collect();
    assert!(missing.is_empty(), "missing Vietnamese engine prompts: {missing:?}");
}

// These values contain no prose, or guard internal contracts rather than user input.
const INTERNAL_ENGINE_TEXT: &[(&str, &str)] = &[
    (", ", "list punctuation"),
    ("  {l}", "indented user layer name"),
    ("MSPACE", "command keyword"),
    (">>{}", "transparent command marker and id"),
    ("{:<24} {:<8} {:<12} {}", "tabular values only"),
    ("{k:<12} {}", "variable name and value only"),
    ("{PICKED}{}", "calculator expression and result only"),
    ("{0}", "transparent thiserror wrapper"),
    ("asynchronous print jobs require the frame pump", "internal native scheduler contract"),
    ("plan must serialize as an object", "typed plan serialization invariant"),
];

fn engine_message_literals(source: &str) -> Vec<(usize, String, bool)> {
    let tokens = production_ui_tokens(source);
    let mut result = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        if (token.text == "msg" || token.literal.as_deref() == Some("message")) && tokens.get(i + 1).is_some_and(|t| t.text == ":") {
            let start = i + 2;
            let mut end = start;
            while end < tokens.len() && !matches!(tokens[end].text, "," | "}" | ")") {
                if matches!(tokens[end].text, "(" | "[" | "{") {
                    end = closing_token(&tokens, end);
                }
                end += 1;
            }
            for (offset, text) in bound_text_literals(&tokens, start..end) {
                let index = tokens.iter().position(|t| t.offset == offset).unwrap();
                let formatted = index.checked_sub(3).is_some_and(|i| tokens[i].text == "format");
                result.push((offset, text, formatted));
            }
        }
        if !matches!(token.text, "Err" | "Other" | "other" | "bad" | "echo" | "notice" | "bail" | "error")
            || i.checked_sub(1).and_then(|i| tokens.get(i)).is_some_and(|t| t.text == "fn")
        {
            continue;
        }
        let open = i + if tokens.get(i + 1).is_some_and(|t| t.text == "!") { 2 } else { 1 };
        if !tokens.get(open).is_some_and(|t| t.text == "(") {
            continue;
        }
        let arguments = ui_arguments(&tokens, open);
        let position = usize::from(token.text == "bad" && arguments.len() > 1);
        let Some(argument) = arguments.get(position) else { continue };
        let Some(first) = tokens.get(argument.start) else { continue };
        if let Some(text) = &first.literal {
            result.push((first.offset, text.clone(), token.text == "error"));
        } else if first.text == "format" && tokens.get(argument.start + 1).is_some_and(|t| t.text == "!") {
            if let Some(format) = tokens.get(argument.start + 3)
                && let Some(text) = &format.literal
            {
                result.push((format.offset, text.clone(), true));
            }
        } else if matches!(first.text, "if" | "match") {
            for (offset, text) in text_literals(&tokens, argument.clone()) {
                let index = tokens.iter().position(|t| t.offset == offset).unwrap();
                let formatted = index.checked_sub(3).is_some_and(|i| tokens[i].text == "format");
                result.push((offset, text, formatted));
            }
        }
    }
    result
}

fn message_shape(text: &str) -> String {
    let mut result = String::new();
    let mut rest = text;
    while let Some((before, tail)) = rest.split_once('{') {
        result.push_str(before);
        if let Some(after) = tail.strip_prefix('{') {
            result.push('{');
            rest = after;
        } else if let Some((field, after)) = tail.split_once('}') {
            if field.contains(',') || (field.chars().any(char::is_whitespace) && field.chars().any(char::is_alphabetic)) {
                result.push('{');
                result.push_str(field);
                result.push('}');
            } else {
                result.push_str("{}");
            }
            rest = after;
        } else {
            result.push('{');
            rest = tail;
        }
    }
    result.push_str(rest);
    result.replace("}}", "}")
}

#[test]
fn every_engine_error_and_notice_has_a_vietnamese_translation() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../engine/src");
    let mut directories = vec![root.clone()];
    let catalog = catalog("vi").unwrap();
    let shapes: BTreeSet<_> =
        catalog.keys().filter(|(context, key)| ["", "echo"].contains(context) && key.contains('{')).map(|(_, key)| message_shape(key)).collect();
    let templates = echo_templates("vi");
    let mut missing = BTreeSet::new();
    let mut checked = 0;
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                directories.push(path);
                continue;
            }
            let file = path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
            if path.extension().is_none_or(|extension| extension != "rs") || test_source(&file) {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            for (offset, message, formatted) in engine_message_literals(&source) {
                if INTERNAL_ENGINE_TEXT.iter().any(|(internal, _)| *internal == message) {
                    continue;
                }
                for key in message.lines().filter(|line| !line.is_empty()) {
                    checked += 1;
                    let covered = catalog.contains_key(&("", key))
                        || catalog.contains_key(&("echo", key))
                        || (formatted && shapes.contains(&message_shape(key)))
                        || (!formatted && templates.iter().any(|template| template.capture(key).is_some()));
                    if !covered {
                        let line = source[..offset].bytes().filter(|b| *b == b'\n').count() + 1;
                        missing.insert(format!("{file}:{line}: {key}"));
                    }
                }
            }
        }
    }
    assert!(checked > 1_000, "engine scan unexpectedly checked only {checked} messages");
    assert!(missing.is_empty(), "missing Vietnamese engine errors/notices:\n{}", missing.into_iter().collect::<Vec<_>>().join("\n"));
}

#[test]
fn engine_message_scan_catches_new_errors_and_ignores_test_fixtures() {
    let source = r#"
        fn run() {
            Err("Brand new message".into());
            EngineError::Other("Other message".into());
            other("Helper message");
            bad("line", "Parameter message");
            session.echo(format!("Block {} missing", name));
            session.echo(if ok { "Done" } else { "Failed" });
        }
        #[cfg(test)] mod tests { fn fixture() { Err("Test only".into()); } }
    "#;
    let found: BTreeSet<_> = engine_message_literals(source).into_iter().map(|(_, text, _)| text).collect();
    assert_eq!(
        found,
        BTreeSet::from(
            ["Brand new message", "Other message", "Helper message", "Parameter message", "Block {} missing", "Done", "Failed"].map(str::to_string)
        )
    );
    assert!(!catalog("vi").unwrap().contains_key(&("", "Brand new message")));
    assert!(echo_templates("vi").iter().all(|template| template.capture("Brand new message").is_none()));
}

#[test]
fn echoed_templates_capture_whole_lines_with_shortest_nonempty_values() {
    let template = EchoTemplate::new("Block {name} failed: {reason}", "Lỗi {reason}: {name}").unwrap();
    let values = template.capture("Block A failed: B: C").unwrap();
    assert_eq!(values, [("name", "A"), ("reason", "B: C")]);
    assert_eq!(substitute(template.translated, &values), "Lỗi B: C: A");
    assert!(template.capture("prefix Block A failed: B").is_none());
    assert!(template.capture("Block  failed: B").is_none());
    assert!(template.capture("Block A failed:").is_none());
    assert!(template.capture("Block A failed: ").is_none());
    let repeated = EchoTemplate::new("Object name {name}/{name}.", "Tên {name}/{name}.").unwrap();
    assert!(repeated.capture("Object name A/B.").is_none());
    assert!(repeated.capture("Object name A/A.").is_some());
    let backtrack = EchoTemplate::new("Value {a}x{b}y", "Giá trị {a}/{b}").unwrap();
    assert_eq!(backtrack.capture("Value axxby").unwrap(), [("a", "a"), ("b", "xb")]);
    let unicode = EchoTemplate::new("Block name {name}.", "Tên khối {name}.").unwrap();
    assert_eq!(unicode.capture("Block name Văn {phòng}.").unwrap(), [("name", "Văn {phòng}")]);
    assert!(unicode.capture(&"x".repeat(16_385)).is_none());
    assert!(EchoTemplate::new("{title} {message}", "{title} {message}").is_none());
    assert!(EchoTemplate::new("Bad {}", "Lỗi {}").is_none());
    for source in ["{min} to {max}", "{axes} of", "{snap} of", "{path}: {e}", "{value1} = {value2}", "{width} × {height} mm"] {
        assert!(EchoTemplate::new(source, source).is_none(), "{source}");
    }
    assert!(EchoTemplate::new("Block {name}", "Khối {name}").is_none());
    assert!(EchoTemplate::new("Blocks {name}", "Các khối {name}").is_some());
    assert!(EchoTemplate::new("Áéíóúñ {name}", "Áéíóúñ {name}").is_some());
    let braces = EchoTemplate::new("Use {name: bool}, got {value}", "Dùng {name: bool}, đã nhận {value}").unwrap();
    assert_eq!(braces.capture("Use {name: bool}, got false").unwrap(), [("value", "false")]);
}

#[test]
fn echoed_catalog_lookup_preserves_unknown_text_and_english_bytes() {
    with_language("vi", || {
        assert_eq!(echo("Command: LINE"), "Lệnh: LINE");
        assert_eq!(echo("Block \"ROOM {name}\" not found."), "Không tìm thấy khối \"ROOM {name}\".");
        assert_eq!(echo("Custom output {name}: LINE"), "Custom output {name}: LINE");
        assert_eq!(echo("Moved 3 objects to Layer1"), "Moved 3 objects to Layer1");
        assert_eq!(echo("a: b"), "a: b");
        assert_eq!(echo("Current: RoutineName"), "Current: RoutineName");
        assert_eq!(echo("Value must be positive and nonzero."), "Giá trị phải dương và khác không.");
        assert_eq!(echo("Command: {name}"), "Lệnh: {name}");
        assert_eq!(echo("invalid parameters for `circle`: radius must be positive"), "tham số không hợp lệ cho `circle`: bán kính phải dương");
    });
    for language in ["en", "unknown"] {
        with_language(language, || {
            for line in ["Command: LINE", "Value must be positive and nonzero.", "Block \"ROOM\" not found.", "Custom output {name}: LINE", " \n", ""]
            {
                assert_eq!(echo(line).as_bytes(), line.as_bytes());
            }
        });
    }
}

#[test]
fn echo_memo_is_bounded_and_separate_for_each_language() {
    ECHO_MEMO.with(|memo| memo.borrow_mut().clear());
    let source = "Command: LINE";
    assert_eq!(translated_echo("vi", source), "Lệnh: LINE");
    assert_eq!(translated_echo("uk", source), "Команда: LINE");
    assert_eq!(translated_echo("vi", source), "Lệnh: LINE");
    assert_eq!(translated_echo("uk", source), "Команда: LINE");
    assert!(matches!(translated_echo("vi", "Value must be positive and nonzero."), Cow::Borrowed(_)));
    assert!(matches!(translated_echo("vi", "Command: {name}"), Cow::Borrowed(_)));
    for line in ["Moved 3 objects to Layer1", "a: b"] {
        assert_eq!(translated_echo("vi", line), line);
        assert_eq!(translated_echo("vi", line), line);
        assert!(matches!(translated_echo("vi", line), Cow::Owned(_)));
    }
    ECHO_MEMO.with(|memo| {
        let memo = memo.borrow();
        assert_eq!(memo.get("vi").unwrap().get(source).map(String::as_str), Some("Lệnh: LINE"));
        assert_eq!(memo.get("uk").unwrap().get(source).map(String::as_str), Some("Команда: LINE"));
        assert_eq!(memo.get("vi").unwrap().get("a: b").map(String::as_str), Some("a: b"));
        assert!(!memo.get("vi").unwrap().contains_key("Command: {name}"));
    });
    ECHO_MEMO.with(|memo| memo.borrow_mut().clear());
    for index in 0..514 {
        let line = format!("Unmatched routine output {index}");
        assert_eq!(translated_echo("vi", &line), line.as_str());
    }
    ECHO_MEMO.with(|memo| {
        let memo = memo.borrow();
        let lines = memo.get("vi").unwrap();
        assert_eq!(lines.len(), 2);
        assert!(!lines.contains_key("Unmatched routine output 0"));
    });
}

fn missing_commands_menus_tools_and_snap_labels(language: &str) -> Vec<String> {
    let catalog = catalog(language).unwrap();
    command_label_sources().into_iter().filter(|key| !catalog.contains_key(&("", key.as_str()))).collect()
}

fn widget_text_argument(arguments: &str, wanted: usize) -> Option<&str> {
    let (mut start, mut argument, mut depth) = (0, 0, 0usize);
    let (mut quoted, mut escape) = (false, false);
    for (i, c) in arguments.char_indices() {
        if quoted {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                quoted = false;
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            '(' | '[' | '{' => depth += 1,
            ')' if depth == 0 => return (argument == wanted).then(|| arguments[start..i].trim()),
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                if argument == wanted {
                    return Some(arguments[start..i].trim());
                }
                argument += 1;
                start = i + 1;
            }
            _ => {}
        }
    }
    None
}

#[derive(Clone, Debug)]
struct UiToken<'a> {
    text: &'a str,
    offset: usize,
    literal: Option<String>,
}

fn rust_string(text: &str) -> String {
    if text.starts_with('r') {
        let quote = text.find('"').unwrap();
        return text[quote + 1..text.len() - quote].to_string();
    }
    let mut decoded = String::new();
    let mut chars = text[1..text.len() - 1].chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            decoded.push(c);
            continue;
        }
        match chars.next().unwrap() {
            'n' => decoded.push('\n'),
            'r' => decoded.push('\r'),
            't' => decoded.push('\t'),
            '0' => decoded.push('\0'),
            'x' => {
                let hex: String = chars.by_ref().take(2).collect();
                decoded.push(char::from(u8::from_str_radix(&hex, 16).unwrap()));
            }
            'u' => {
                assert_eq!(chars.next(), Some('{'));
                let hex: String = chars.by_ref().take_while(|c| *c != '}').filter(|c| *c != '_').collect();
                decoded.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap());
            }
            '\n' => {
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
            }
            c => decoded.push(c),
        }
    }
    decoded
}

fn ui_tokens(source: &str) -> Vec<UiToken<'_>> {
    let mut tokens = Vec::new();
    let mut offset = 0;
    while offset < source.len() {
        let rest = &source[offset..];
        let first = rest.chars().next().unwrap();
        if first.is_whitespace() {
            offset += first.len_utf8();
            continue;
        }
        if rest.starts_with("//") {
            offset += rest.find('\n').unwrap_or(rest.len());
            continue;
        }
        if rest.starts_with("/*") {
            let mut depth = 1;
            offset += 2;
            while depth > 0 && offset < source.len() {
                if source[offset..].starts_with("/*") {
                    depth += 1;
                    offset += 2;
                } else if source[offset..].starts_with("*/") {
                    depth -= 1;
                    offset += 2;
                } else {
                    offset += source[offset..].chars().next().unwrap().len_utf8();
                }
            }
            continue;
        }
        let start = offset;
        let raw_marker = if rest.starts_with("br") || rest.starts_with("cr") {
            Some(2)
        } else if rest.starts_with('r') {
            Some(1)
        } else {
            None
        };
        let raw_prefix = raw_marker.map(|marker| rest[marker..].bytes().take_while(|b| *b == b'#').count() + marker);
        let mut literal = false;
        if let Some(prefix) = raw_prefix.filter(|prefix| rest.as_bytes().get(*prefix) == Some(&b'"')) {
            let suffix = format!("\"{}", "#".repeat(prefix - raw_marker.unwrap()));
            offset += prefix + 1;
            offset += source[offset..].find(&suffix).unwrap() + suffix.len();
            literal = raw_marker == Some(1);
        } else if first == '"' {
            offset += 1;
            while offset < source.len() {
                let c = source[offset..].chars().next().unwrap();
                offset += c.len_utf8();
                if c == '\\' {
                    offset += source[offset..].chars().next().unwrap().len_utf8();
                } else if c == '"' {
                    break;
                }
            }
            literal = true;
        } else if first == '\''
            && rest
                .get(1..)
                .and_then(|s| s.chars().next())
                .is_some_and(|c| rest.get(1 + c.len_utf8()..).is_some_and(|s| s.starts_with('\'')) || c == '\\')
        {
            offset += 1;
            while offset < source.len() {
                let c = source[offset..].chars().next().unwrap();
                offset += c.len_utf8();
                if c == '\\' {
                    offset += source[offset..].chars().next().unwrap().len_utf8();
                } else if c == '\'' {
                    break;
                }
            }
        } else if first.is_ascii_alphabetic() || first == '_' {
            offset += rest.bytes().take_while(|b| b.is_ascii_alphanumeric() || *b == b'_').count();
        } else {
            offset += first.len_utf8();
        }
        let text = &source[start..offset];
        tokens.push(UiToken { text, offset: start, literal: literal.then(|| rust_string(text)) });
    }
    tokens
}

fn closing_token(tokens: &[UiToken<'_>], open: usize) -> usize {
    let mut depth = 0usize;
    for (i, token) in tokens.iter().enumerate().skip(open) {
        match token.text {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    panic!("unclosed source token at {}", tokens[open].offset);
}

fn production_ui_tokens(source: &str) -> Vec<UiToken<'_>> {
    let tokens = ui_tokens(source);
    let mut result = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        if tokens[i].text == "#" && tokens.get(i + 1).is_some_and(|t| t.text == "[") {
            let end = closing_token(&tokens, i + 1);
            let attribute: String = tokens[i + 2..end].iter().map(|t| t.text).collect();
            // Test fixtures intentionally contain untranslated text and simulated widget calls.
            if attribute == "cfg(test)" || attribute == "test" {
                i = end + 1;
                while i < tokens.len() && !matches!(tokens[i].text, "{" | ";") {
                    i += 1;
                }
                if tokens.get(i).is_some_and(|t| t.text == "{") {
                    i = closing_token(&tokens, i);
                }
                i += 1;
                continue;
            }
        }
        result.push(tokens[i].clone());
        i += 1;
    }
    result
}

fn ui_source_files() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut directories = vec![root.clone()];
    let mut sources = Vec::new();
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                directories.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let file = path.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
                sources.push((file, std::fs::read_to_string(path).unwrap()));
            }
        }
    }
    sources.sort_by(|a, b| a.0.cmp(&b.0));
    sources
}

fn test_source(file: &str) -> bool {
    // Standalone test modules have no enclosing cfg(test) attribute in their own source file.
    file.split('/').any(|part| part == "tests" || part == "tests.rs" || part.ends_with("_tests.rs"))
}

fn ui_arguments(tokens: &[UiToken<'_>], open: usize) -> Vec<std::ops::Range<usize>> {
    let end = closing_token(tokens, open);
    let mut result = Vec::new();
    let mut start = open + 1;
    let mut i = start;
    while i < end {
        match tokens[i].text {
            "(" | "[" | "{" => i = closing_token(tokens, i),
            "," => {
                result.push(start..i);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < end {
        result.push(start..end);
    }
    result
}

// Each exemption is data or notation whose spelling must survive a language change.
const UNTRANSLATED_UI_TEXT: &[(&str, &str)] = &[
    ("UbotCAD", "product name"),
    ("CADCraft", "product name"),
    ("ArtCraft", "brand name"),
    ("ubotbuild.com", "public website address"),
    ("ByLayer", "CAD property token"),
    ("ByBlock", "CAD property token"),
    ("WCS", "coordinate system identifier"),
    ("UCS", "coordinate system identifier"),
    ("B", "bold glyph; tooltip is translated"),
    ("I", "italic glyph; tooltip is translated"),
    ("U", "underline glyph; tooltip is translated"),
    ("O", "overline glyph; tooltip is translated"),
    ("Q", "quick dimension icon glyph"),
    ("A▾", "text colour glyph; menu labels are translated"),
    ("A", "text icon glyph; tooltip is translated"),
];

fn intentional_ui_notation(text: &str) -> bool {
    if UNTRANSLATED_UI_TEXT.iter().any(|(key, reason)| {
        assert!(!reason.is_empty());
        *key == text
    }) || text == cadcraft_fonts::BUILTIN_FONT
    {
        return true;
    }
    // Placeholder values, punctuation, numeric readouts, axes and units carry no UI prose.
    let mut notation = String::new();
    let mut placeholder = false;
    for c in text.chars() {
        match c {
            '{' => placeholder = true,
            '}' => placeholder = false,
            _ if !placeholder => notation.push(c),
            _ => {}
        }
    }
    if !notation.chars().any(char::is_alphabetic) || ["X", "Y", "Z", "mm", "cm", "m", "in", "pt"].contains(&notation.trim()) {
        return true;
    }
    // A single font or drawing filename is data; prose mentioning an extension still needs translation.
    !text.chars().any(char::is_whitespace)
        && [".ttf", ".otf", ".ttc", ".shx", ".dxf", ".dwg", ".lin", ".ctb", ".stb", ".pc3"]
            .iter()
            .any(|extension| text.to_ascii_lowercase().ends_with(extension))
}

type TextHelpers = std::collections::BTreeMap<String, std::collections::BTreeMap<usize, bool>>;

fn text_slots(tokens: &[UiToken<'_>], i: usize, helpers: &TextHelpers) -> Vec<(usize, bool)> {
    let name = tokens[i].text;
    let after = tokens.get(i + 1).map(|t| t.text);
    if matches!(name, "tl" | "tf") && after == Some("!") && tokens.get(i + 2).is_some_and(|t| t.text == "(") {
        return vec![(0, true)];
    }
    if after != Some("(") || i.checked_sub(1).is_some_and(|j| tokens[j].text == "fn") {
        return Vec::new();
    }
    let mut slots = Vec::new();
    if name == "t" && !i.checked_sub(1).is_some_and(|j| tokens[j].text == ".") {
        slots.push((0, true));
    }
    if i.checked_sub(1).is_some_and(|j| tokens[j].text == ".") {
        let position = match name {
            "label"
            | "button"
            | "heading"
            | "on_hover_text"
            | "on_disabled_hover_text"
            | "small_button"
            | "collapsing"
            | "hint_text"
            | "menu_button"
            | "selected_text"
            | "set_status"
            | "echo"
            | "weak"
            | "strong"
            | "monospace"
            | "hyperlink"
            | "hyperlink_to" => Some(0),
            "selectable_label" | "checkbox" | "radio" => Some(1),
            "radio_value" | "selectable_value" => Some(2),
            "text" => Some(2),
            _ => None,
        };
        if let Some(position) = position {
            slots.push((position, false));
        }
    }
    if name == "new"
        && let Some(j) = i.checked_sub(3)
        && tokens[j + 1].text == ":"
        && tokens[j + 2].text == ":"
    {
        match tokens[j].text {
            "Button" | "RichText" | "Window" | "CollapsingHeader" | "Label" | "Link" | "MenuButton" => slots.push((0, false)),
            "Checkbox" | "RadioButton" | "SelectableLabel" => slots.push((1, false)),
            _ => {}
        }
    }
    if let Some(parameters) = helpers.get(name) {
        slots.extend(parameters.iter().map(|(position, translated)| (*position, *translated)));
    }
    slots.sort();
    slots.dedup();
    slots
}

fn text_call_open(tokens: &[UiToken<'_>], i: usize) -> usize {
    if tokens[i + 1].text == "!" { i + 2 } else { i + 1 }
}

fn bare_text_parameter<'a>(tokens: &[UiToken<'a>], mut range: std::ops::Range<usize>) -> Option<&'a str> {
    while range.start < range.end && tokens[range.start].text == "&" {
        range.start += 1;
    }
    while range.len() >= 4
        && tokens[range.end - 4].text == "."
        && matches!(tokens[range.end - 3].text, "as_str" | "clone" | "to_owned" | "to_string")
        && tokens[range.end - 2].text == "("
        && tokens[range.end - 1].text == ")"
    {
        range.end -= 4;
    }
    (range.len() == 1).then(|| tokens[range.start].text)
}

fn text_helpers(sources: &[Vec<UiToken<'_>>]) -> TextHelpers {
    let mut helpers = TextHelpers::new();
    loop {
        let mut changed = false;
        for tokens in sources {
            let mut i = 0;
            while i + 2 < tokens.len() {
                if tokens[i].text != "fn" || tokens[i + 2].text != "(" {
                    i += 1;
                    continue;
                }
                let name = tokens[i + 1].text;
                let parameters: Vec<_> = ui_arguments(tokens, i + 2)
                    .into_iter()
                    .map(|range| {
                        let parameter = &tokens[range];
                        let end = parameter.iter().position(|t| t.text == ":").unwrap_or(parameter.len());
                        parameter[..end].iter().rev().find(|t| t.text != "mut").map(|t| t.text)
                    })
                    .collect();
                let mut body = closing_token(tokens, i + 2) + 1;
                while body < tokens.len() && !matches!(tokens[body].text, "{" | ";") {
                    body += 1;
                }
                if !tokens.get(body).is_some_and(|t| t.text == "{") {
                    i = body + 1;
                    continue;
                }
                let end = closing_token(tokens, body);
                for j in body + 1..end {
                    for (position, translated) in text_slots(tokens, j, &helpers) {
                        let arguments = ui_arguments(tokens, text_call_open(tokens, j));
                        let Some(parameter) = arguments.get(position).and_then(|range| bare_text_parameter(tokens, range.clone())) else { continue };
                        let Some(position) = parameters.iter().position(|p| *p == Some(parameter)) else { continue };
                        let Some(position) = position.checked_sub(usize::from(parameters.first() == Some(&Some("self")))) else { continue };
                        let parameters = helpers.entry(name.to_string()).or_default();
                        match parameters.get_mut(&position) {
                            Some(old) if *old && !translated => {
                                *old = false;
                                changed = true;
                            }
                            None => {
                                parameters.insert(position, translated);
                                changed = true;
                            }
                            _ => {}
                        }
                    }
                }
                i = end + 1;
            }
        }
        if !changed {
            return helpers;
        }
    }
}

fn text_literals(tokens: &[UiToken<'_>], range: std::ops::Range<usize>) -> Vec<(usize, String)> {
    let mut result = Vec::new();
    let mut i = range.start;
    while i < range.end {
        let token = &tokens[i];
        // Conditions select text but their comparison values are engine data, not labels.
        if token.text == "if" {
            i += 1;
            while i < range.end && tokens[i].text != "{" {
                if matches!(tokens[i].text, "(" | "[") {
                    i = closing_token(tokens, i);
                }
                i += 1;
            }
            continue;
        }
        if token.text == "match" {
            let mut body = i + 1;
            while body < range.end && tokens[body].text != "{" {
                if matches!(tokens[body].text, "(" | "[") {
                    body = closing_token(tokens, body);
                }
                body += 1;
            }
            if body == range.end {
                break;
            }
            let end = closing_token(tokens, body);
            let mut arm = body + 1;
            while arm < end {
                if tokens[arm].text == "=" && tokens.get(arm + 1).is_some_and(|t| t.text == ">") {
                    let start = arm + 2;
                    arm = start;
                    while arm < end && tokens[arm].text != "," {
                        if matches!(tokens[arm].text, "(" | "[" | "{") {
                            arm = closing_token(tokens, arm);
                        }
                        arm += 1;
                    }
                    result.extend(text_literals(tokens, start..arm));
                } else if matches!(tokens[arm].text, "(" | "[" | "{") {
                    arm = closing_token(tokens, arm);
                }
                arm += 1;
            }
            i = end + 1;
            continue;
        }
        if text_slots(tokens, i, &TextHelpers::new()).contains(&(0, true)) {
            i = closing_token(tokens, text_call_open(tokens, i)) + 1;
            continue;
        }
        if token.text == "format" && tokens.get(i + 1).is_some_and(|t| t.text == "!") {
            let open = i + 2;
            if let Some(format) = ui_arguments(tokens, open).first() {
                result.extend(text_literals(tokens, format.clone()));
            }
            i = closing_token(tokens, open) + 1;
            continue;
        }
        // A nested lookup's arguments are identifiers/data; only text-producing wrappers expose literals.
        if tokens.get(i + 1).is_some_and(|t| t.text == "(")
            && token.text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            && !matches!(token.text, "Some" | "Ok" | "Err" | "unwrap_or" | "unwrap_or_else" | "map_or" | "map_or_else")
        {
            i = closing_token(tokens, i + 1) + 1;
            continue;
        }
        if let Some(text) = &token.literal
            && !intentional_ui_notation(text)
        {
            result.push((token.offset, text.clone()));
        }
        i += 1;
    }
    result
}

fn text_binding(tokens: &[UiToken<'_>], binding: usize, before: usize) -> Option<std::ops::Range<usize>> {
    let mut start = binding + 1;
    while start < before && !matches!(tokens[start].text, "=" | ";") {
        start += 1;
    }
    if start == before || tokens[start].text != "=" {
        return None;
    }
    start += 1;
    let mut end = start;
    while end < before && tokens[end].text != ";" {
        if matches!(tokens[end].text, "(" | "[" | "{") {
            end = closing_token(tokens, end);
        }
        end += 1;
    }
    (end < before).then_some(start..end)
}

fn bound_text_literals(tokens: &[UiToken<'_>], range: std::ops::Range<usize>) -> Vec<(usize, String)> {
    let mut range = range;
    let mut visited = BTreeSet::new();
    while let Some(name) = bare_text_parameter(tokens, range.clone()) {
        // Resolve local/static text bindings, leaving runtime drawing data to its existing path.
        let function = (0..range.start).rev().find(|i| tokens[*i].text == "fn").unwrap_or(0);
        let Some((binding, value)) = (0..range.start)
            .rev()
            .filter(|i| {
                matches!(tokens[*i].text, "let" | "const" | "static")
                    && (tokens[*i].text != "let" || *i >= function)
                    && tokens.get(*i + 1).is_some_and(|t| t.text == name || (t.text == "mut" && tokens.get(*i + 2).is_some_and(|t| t.text == name)))
            })
            .find_map(|binding| text_binding(tokens, binding, range.start).map(|value| (binding, value)))
        else {
            break;
        };
        if !visited.insert(binding) {
            break;
        }
        let first = &tokens[value.start];
        if first.literal.is_none()
            && !matches!(first.text, "if" | "match" | "format" | "crate" | "t" | "tl" | "tf" | "&")
            && bare_text_parameter(tokens, value.clone()).is_none()
        {
            break;
        }
        range = value;
    }
    text_literals(tokens, range)
}

#[derive(Default)]
struct UiLiteralAudit {
    unmarked: BTreeSet<String>,
    translated: BTreeSet<(String, usize, String)>,
}

fn audit_ui_sources(sources: &[(String, String)]) -> UiLiteralAudit {
    let production: Vec<_> = sources.iter().filter(|(file, _)| !test_source(file)).collect();
    let tokens: Vec<_> = production.iter().map(|(_, source)| production_ui_tokens(source)).collect();
    let helpers = text_helpers(&tokens);
    let mut audit = UiLiteralAudit::default();
    for ((file, source), tokens) in production.into_iter().zip(&tokens) {
        for i in 0..tokens.len() {
            if matches!(tokens[i].text, "tl" | "tf")
                && tokens.get(i + 1).is_some_and(|t| t.text == "!")
                && let Some(token) = tokens.get(i + 3)
                && let Some(text) = &token.literal
                && !["X", "Y", "B", "UbotCAD"].contains(&text.as_str())
            {
                let line = source[..token.offset].bytes().filter(|b| *b == b'\n').count() + 1;
                audit.translated.insert((file.clone(), line, text.clone()));
            }
            for (position, translated) in text_slots(tokens, i, &helpers) {
                let arguments = ui_arguments(tokens, text_call_open(tokens, i));
                let Some(range) = arguments.get(position) else { continue };
                for (offset, text) in bound_text_literals(tokens, range.clone()) {
                    let line = source[..offset].bytes().filter(|b| *b == b'\n').count() + 1;
                    if translated {
                        audit.translated.insert((file.clone(), line, text));
                    } else {
                        audit.unmarked.insert(format!("{file}:{line}: {text}"));
                    }
                }
            }
        }
    }
    audit
}

fn ui_literal_audit() -> &'static UiLiteralAudit {
    static AUDIT: std::sync::OnceLock<UiLiteralAudit> = std::sync::OnceLock::new();
    AUDIT.get_or_init(|| {
        let sources = ui_source_files();
        let mut audit = audit_ui_sources(&sources);
        for (file, source) in sources.iter().filter(|(file, _)| !test_source(file)) {
            let tokens = production_ui_tokens(source);
            for key in marked_ui_sources(file, source) {
                if audit.translated.iter().any(|(found, _, text)| found == file && *text == key) {
                    continue;
                }
                let offset = tokens.iter().find(|token| token.literal.as_ref() == Some(&key)).map_or(0, |token| token.offset);
                let line = source[..offset].bytes().filter(|b| *b == b'\n').count() + 1;
                audit.translated.insert((file.clone(), line, key));
            }
        }
        audit
    })
}

fn unmarked_widget_literals(source: &str) -> Vec<(usize, String)> {
    let sources = [("fixture.rs".to_string(), source.to_string())];
    audit_ui_sources(&sources)
        .unmarked
        .into_iter()
        .map(|message| {
            let mut parts = message.splitn(3, ':');
            parts.next();
            (parts.next().unwrap().parse().unwrap(), parts.next().unwrap().trim_start().to_string())
        })
        .collect()
}

#[test]
fn widget_literals_are_marked() {
    let missing = &ui_literal_audit().unmarked;
    assert!(missing.is_empty(), "unmarked widget literals:\n{}", missing.iter().cloned().collect::<Vec<_>>().join("\n"));
}

#[test]
fn widget_literal_scanner_checks_nested_arguments_and_exemptions() {
    let source = r#"
        ui.label("Label"); ui.button("Button"); Button::new("Enabled"); Window::new("Window");
        ui.heading("Heading"); ui.on_hover_text("Tooltip"); ui.small_button("Small");
        ui.collapsing("Details", |_| {}); RichText::new("Rich"); edit.hint_text("Hint");
        ui.selectable_label(matches!(value, (1, 2)), "Choice");
        ui.checkbox(&mut enabled, "Checkbox"); ui.radio(value == (1, 2), "Radio");
        ui.radio_value(&mut value, (1, 2), "Radio value");
        ui.label("X"); ui.label("Y"); ui.label("B"); ui.label("UbotCAD"); ui.button("OK");
        ui.label("ByLayer"); ui.label("ByBlock"); ui.label("WCS"); ui.label("Example.shx");
        ui.label(crate::tl!("Translated")); ui.label(crate::tf!("Count {count}", count = 2));
        // ui.label("Comment");
    "#;
    let labels: BTreeSet<_> = unmarked_widget_literals(source).into_iter().map(|(_, label)| label).collect();
    let expected: BTreeSet<_> = [
        "Label",
        "Button",
        "Enabled",
        "Window",
        "Heading",
        "Tooltip",
        "Small",
        "Details",
        "Rich",
        "Hint",
        "Choice",
        "Checkbox",
        "Radio",
        "Radio value",
        "OK",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    assert_eq!(labels, expected);
}

#[test]
fn ui_sources_are_discovered_recursively_including_unlisted_modules() {
    let sources = ui_source_files();
    for file in ["exchange.rs", "opening.rs", "fullscreen/escape.rs", "i18n/tests.rs", "wsprofile/startup.rs"] {
        assert!(sources.iter().any(|(name, _)| name == file), "source discovery omitted {file}");
    }
    assert!(sources.windows(2).all(|pair| pair[0].0 < pair[1].0));
}

#[test]
fn widget_literal_scanner_covers_lowercase_unicode_raw_strings_and_all_text_arguments() {
    let source = r##"
        ui.menu_button("load...", |_| {});
        egui::CollapsingHeader::new("details"); ui.label(r#"raw "label""#);
        ui.label("… more"); ui.label("tiếng Việt");
        ui.on_disabled_hover_text("unavailable"); ui.weak("weak text");
        ui.selectable_value(&mut value, "command_id", "visible choice");
        ui.radio_value(&mut value, "engine_token", "radio choice");
        ui.label("escaped \"quote\" and \u{2192}");
        ui.painter().text(at, anchor, "canvas label", font, color);
        app.set_status("status message"); app.session.echo("command message");
        ui.label(if mode == "engine condition" { "first label" } else { "second label" });
        ui.label(match value { "engine pattern" => "match label", _ => "fallback label" });
        ui.label(crate::tl!("translated")); ui.label(crate::i18n::t("also translated"));
        ui.label(format!("{} ({})", drawing_name, count));
        ui.label(format!("{value:.2} mm"));
        ui.label("X"); ui.label("UbotCAD"); ui.label("ubotbuild.com");
        ui.label("custom.shx"); ui.label("open custom.shx");
    "##;
    let labels: BTreeSet<_> = unmarked_widget_literals(source).into_iter().map(|(_, label)| label).collect();
    let expected = BTreeSet::from(
        [
            "load...",
            "details",
            "raw \"label\"",
            "… more",
            "tiếng Việt",
            "unavailable",
            "weak text",
            "visible choice",
            "radio choice",
            "escaped \"quote\" and →",
            "canvas label",
            "status message",
            "command message",
            "first label",
            "second label",
            "match label",
            "fallback label",
            "open custom.shx",
        ]
        .map(str::to_string),
    );
    assert_eq!(labels, expected);
}

#[test]
fn widget_literal_scanner_ignores_comments_string_contents_and_test_code() {
    let source = r##"
        /* ui.label("comment"); /* ui.button("nested comment"); */ */
        // ui.button("line comment");
        let example = r#"ui.label("documentation sample")"#;
        let bytes = br#"ui.label("byte data")"#;
        let character = '('; let lifetime: &'static str = "data";
        #[cfg(test)] mod tests { fn example() { ui.label("test label"); } }
        #[test] fn regression() { ui.button("test button"); }
        ui.label("production label");
    "##;
    assert_eq!(unmarked_widget_literals(source), vec![(9, "production label".to_string())]);
    let sources = [("nested/tests.rs".to_string(), "ui.label(\"fixture\");".to_string())];
    assert!(audit_ui_sources(&sources).unmarked.is_empty());
}

#[test]
fn text_bindings_and_translated_helper_parameters_are_checked_across_files() {
    let sources = [
        (
            "helpers.rs".to_string(),
            r#"
            fn prop_row(ui: &mut Ui, label: &str) { ui.label(crate::i18n::t(label)); }
            fn forwarded(ui: &mut Ui, title: &str) { prop_row(ui, title); }
            fn raw_heading(ui: &mut Ui, title: &str) { ui.heading(title); }
        "#
            .to_string(),
        ),
        (
            "new/nested_panel.rs".to_string(),
            r#"
            let heading = "lowercase heading";
            ui.label(heading);
            let translated = "a missing catalog key";
            ui.label(crate::i18n::t(translated));
            forwarded(ui, "a helper catalog key");
            raw_heading(ui, "raw helper heading");
            ui.button(crate::tl!("a marked key"));
            let owned = "owned heading";
            ui.label(owned.as_str());
            let translated_owned = "a shadowed catalog key";
            let translated_owned = crate::i18n::t(translated_owned);
            ui.label(translated_owned);
        "#
            .to_string(),
        ),
    ];
    let audit = audit_ui_sources(&sources);
    assert_eq!(
        audit.unmarked,
        BTreeSet::from([
            "new/nested_panel.rs:2: lowercase heading".to_string(),
            "new/nested_panel.rs:7: raw helper heading".to_string(),
            "new/nested_panel.rs:9: owned heading".to_string(),
        ])
    );
    let expected = BTreeSet::from([
        ("new/nested_panel.rs".to_string(), 4, "a missing catalog key".to_string()),
        ("new/nested_panel.rs".to_string(), 6, "a helper catalog key".to_string()),
        ("new/nested_panel.rs".to_string(), 8, "a marked key".to_string()),
        ("new/nested_panel.rs".to_string(), 11, "a shadowed catalog key".to_string()),
    ]);
    assert_eq!(audit.translated, expected);
}

#[test]
fn every_marked_ui_literal_has_a_translation() {
    for language in REQUIRED.iter().chain(REPORTED) {
        check(language, "UI translations", missing_marked_ui_literals(language).into_iter().collect());
    }
}

fn missing_marked_ui_literals(language: &str) -> BTreeSet<String> {
    let catalog = catalog(language).unwrap();
    ui_literal_audit()
        .translated
        .iter()
        .filter(|(_, _, key)| !catalog.contains_key(&("", key.as_str())) && !catalog.contains_key(&("echo", key.as_str())))
        .map(|(file, line, key)| format!("{file}:{line}: {key}"))
        .collect()
}

#[test]
fn translation_sources_include_all_ui_surfaces() {
    let sources = command_label_sources();
    for key in
        ["File", "New Drawing...", "Line", "Drawing as bytes", "Geometric Constraints", "Coincident", "Calibrate 3D Mouse", "Primitives", "Endpoint"]
    {
        assert!(sources.contains(key), "missing translation source: {key}");
    }
    assert!(!sources.contains(""));
    let sources = ui_source_files();
    let (_, source) = sources.iter().find(|(name, _)| name == "xrefs.rs").unwrap();
    let xrefs = marked_ui_sources("xrefs.rs", source);
    assert!(xrefs.contains("Attach DWG..."));
    assert!(xrefs.contains("References: {n}"));
    assert!(!xrefs.contains("New Drawing..."));
}

#[test]
fn translations_cover_ribbon_labels_with_context_fallback() {
    let sources = ribbon_label_sources();
    assert!(sources.contains("Home"));
    assert!(sources.contains("Output"));
    for language in REQUIRED.iter().chain(REPORTED) {
        let catalog = catalog(language).unwrap();
        let missing = sources
            .iter()
            .filter(|key| !catalog.contains_key(&("ribbon", key.as_str())) && !catalog.contains_key(&("", key.as_str())))
            .cloned()
            .collect();
        check(language, "ribbon translations", missing);
    }
}

#[test]
fn locale_and_saved_preference_resolution() {
    for tag in ["uk", "uk-UA", "uk_UA.UTF-8", "UK-ua", " uk-UA ", "uk_UA@euro"] {
        assert_eq!(locale_language(tag), Some("uk"), "{tag}");
    }
    for tag in ["de", "de-DE", "de_DE.UTF-8", "DE-at", "de-CH"] {
        assert_eq!(locale_language(tag), Some("de"), "{tag}");
    }
    for (tag, code) in [("ru", "ru"), ("ru-RU", "ru"), ("ru_RU.UTF-8", "ru"), ("fr", "fr"), ("fr-FR", "fr"), ("fr_CA.UTF-8", "fr")] {
        assert_eq!(locale_language(tag), Some(code), "{tag}");
    }
    for tag in ["ua", "ukulele", "pl-PL", "", "garbage", "💙"] {
        assert_eq!(locale_language(tag), None, "{tag}");
    }
    for tag in ["pt", "pt-BR", "pt_BR.UTF-8", "PT-br", "pt-PT"] {
        assert_eq!(locale_language(tag), Some("pt-BR"), "{tag}");
    }
    assert_eq!(locale_language("ptolemy"), None);
    assert_eq!(Preference::Auto.resolve(&["pl-PL".into(), "pt-BR".into()]), "pt-BR");
    assert_eq!(Preference::Auto.resolve(&["fr-FR".into(), "pt-BR".into()]), "fr");
    for tag in ["es", "es-ES", "es_MX.UTF-8", "ES-ar"] {
        assert_eq!(locale_language(tag), Some("es"), "{tag}");
    }
    assert_eq!(Preference::Auto.resolve(&["es-ES".into(), "pt-BR".into()]), "es");
    assert_eq!(Preference::parse("es"), Some(Preference::Spanish));
    assert_eq!(Preference::BrazilianPortuguese.resolve(&["uk-UA".into()]), "pt-BR");
    assert_eq!(Preference::parse("pt-BR"), Some(Preference::BrazilianPortuguese));
    let locales = vec!["pl-PL".into(), "uk-UA".into(), "en-US".into()];
    assert_eq!(Preference::Auto.resolve(&locales), "uk");
    assert_eq!(Preference::English.resolve(&locales), "en");
    assert_eq!(Preference::German.resolve(&locales), "de");
    assert_eq!(Preference::Auto.resolve(&["de-DE".into(), "en-US".into()]), "de");
    assert_eq!(Preference::Auto.resolve(&["en-US".into(), "uk-UA".into()]), "en");
    assert_eq!(Preference::Auto.resolve(&[]), "en");
    assert_eq!(Preference::restore(None), Preference::Auto);
    assert_eq!(Preference::restore(Some("obsolete")), Preference::Auto);
    for preference in Preference::ALL {
        assert_eq!(Preference::restore(Some(preference.code())), preference);
        let saved = serde_json::to_string(&preference).unwrap();
        assert_eq!(serde_json::from_str::<Preference>(&saved).unwrap(), preference);
    }
    assert!(serde_json::from_str::<Preference>("\"unsupported\"").is_err());
    assert_eq!(serde_json::from_value::<crate::UiState>(json!({})).unwrap().interface_language, Preference::Auto);
    // Saved with the host's other preferences; a bad saved value keeps the current choice.
    let mut a = CadApp::new(Session::new(), Services::default());
    a.ui.interface_language = Preference::Ukrainian;
    let mut b = CadApp::new(Session::new(), Services::default());
    b.load_prefs(&a.prefs_json());
    assert_eq!(b.ui.interface_language, Preference::Ukrainian);
    b.load_prefs(r#"{"interfaceLanguage":"klingon"}"#);
    assert_eq!(b.ui.interface_language, Preference::Ukrainian);
}

#[test]
fn plural_forms_and_templates_preserve_inserted_user_data() {
    with_language("uk", || {
        for (n, form) in [
            (0, 2),
            (1, 0),
            (2, 1),
            (4, 1),
            (5, 2),
            (11, 2),
            (12, 2),
            (14, 2),
            (21, 0),
            (22, 1),
            (25, 2),
            (101, 0),
            (111, 2),
            (114, 2),
            (u64::MAX, 2),
        ] {
            assert_eq!(plural_uk(n), form, "{n}");
        }
        assert_eq!(count(1, "{n} contributor", "{n} contributors"), "1 учасник");
        assert_eq!(count(22, "{n} contributor", "{n} contributors"), "22 учасники");
        assert_eq!(count(11, "{n} contributor", "{n} contributors"), "11 учасників");
        assert_eq!(format("Current: {color}", &[("color", "{n} Open".into())]), "Поточний: {n} Open");
        assert_eq!(format("{b}/{a}", &[("a", "А".into()), ("b", "Б".into())]), "Б/А");
    });
    assert_eq!(t("Layers"), "Layers");
    with_language("de", || {
        assert_eq!(t("Layers"), "Layer");
        assert_eq!(count(1, "{n} contributor", "{n} contributors"), "1 Mitwirkender");
        assert_eq!(count(2, "{n} contributor", "{n} contributors"), "2 Mitwirkende");
        assert_eq!(count(11, "{n} contributor", "{n} contributors"), "11 Mitwirkende");
    });
    with_language("uk", || {
        assert_eq!(t("Layers"), "Шари");
        with_language("en", || assert_eq!(t("Layers"), "Layers"));
        assert_eq!(t("Layers"), "Шари");
    });
    assert_eq!(count(2, "{n} contributor", "{n} contributors"), "2 contributors");
}

#[test]
fn brazilian_portuguese_plurals_and_accents() {
    for (n, form) in [(0, 0), (1, 0), (2, 1), (5, 1), (21, 1), (u64::MAX, 1)] {
        assert_eq!(plural_pt(n), form, "{n}");
    }
    with_language("pt-BR", || {
        assert_eq!(count(1, "{n} contributor", "{n} contributors"), "1 colaborador");
        assert_eq!(count(3, "{n} contributor", "{n} contributors"), "3 colaboradores");
        assert_eq!(t("Layers"), "Camadas");
        assert_eq!(t("Dimension"), "Cota");
        assert_eq!(format("Height: {height}", &[("height", "{n}".into())]), "Altura: {n}");
    });
    assert_eq!(count(2, "{n} contributor", "{n} contributors"), "2 contributors");
}

#[test]
fn live_switch_to_brazilian_portuguese_translates_the_shell() {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.ui.in_window_menu = true;
    let ctx = egui::Context::default();
    app.logic(&ctx);
    app.start("line");
    app.run("ui.language", json!({"lang":"pt-BR"})).unwrap();
    let text = render(&mut app, &ctx);
    assert!(text.contains("Propriedades"), "{text}");
    assert!(text.contains("Arquivo"), "{text}");
    assert_eq!(app.language(), "pt-BR");
    assert_eq!(t("Properties"), "Properties", "language must not leak out of the render scope");
    app.run("ui.language", json!({"lang":"en"})).unwrap();
    assert!(render(&mut app, &ctx).contains("Properties"));
}

fn painted_text(shapes: &[egui::epaint::ClippedShape]) -> String {
    fn collect(shape: &egui::Shape, text: &mut String) {
        match shape {
            egui::Shape::Text(s) => {
                text.push_str(s.galley.text());
                text.push('\n');
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect(shape, text);
                }
            }
            _ => {}
        }
    }
    let mut text = String::new();
    for shape in shapes {
        collect(&shape.shape, &mut text);
    }
    text
}

fn render(app: &mut CadApp, ctx: &egui::Context) -> String {
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0))), ..Default::default() };
    // Windows fade in; a second pass makes their text visible without a time dependency.
    let mut text = String::new();
    for _ in 0..2 {
        let mut output = ctx.run_ui(input.clone(), |ui| app.ui(ui));
        text = painted_text(&output.shapes);
        output.textures_delta.clear();
    }
    text
}

#[test]
fn no_english_catalog_text_in_p0_screens_vi() {
    if !crate::theme::can_show("vi") {
        eprintln!("Vietnamese P0 rendering skipped: no covering UI font; fallback is tested separately");
        return;
    }
    let catalog = catalog("vi").unwrap();
    let mut leaks = BTreeSet::new();
    for (screen, dialog, heading) in [
        ("Start page", None, "Start"),
        ("LINE", None, "Specify first point"),
        ("Customize", Some(crate::customize::DIALOG), "Customize Workspace..."),
        ("Fonts", Some("fonts"), "Fonts"),
        ("Plot", Some("plot"), "Plot"),
        ("Command Palette", Some(crate::palette::DIALOG), "Command Palette..."),
        ("Quick Plot", Some(crate::ubplot::DIALOG), "Quick Plot"),
    ] {
        let mut app = CadApp::new(Session::new(), Services::default());
        app.skip_first_run_picker = true;
        app.attach_profile_store(Box::new(crate::wsprofile::store::MemoryStore::default()));
        app.run("new", json!({})).unwrap();
        app.run("ui.workspace.select", json!({"id": "AUTOCAD_FAMILIAR"})).unwrap();
        app.run("ui.language", json!({"lang": "vi"})).unwrap();
        app.ui.in_window_menu = true;
        app.ui.start_tab = screen == "Start page";
        app.session.state_mut().unwrap().title = "Bản vẽ kiểm tra".into();
        if screen == "LINE" {
            app.start("line");
        } else if dialog == Some(crate::ubplot::DIALOG) {
            // First set wins per process: UBPLOT scans hash the drawing through the writer.
            cadcraft_engine::cmd::file::set_io(cadcraft_engine::cmd::file::IoHooks {
                read: |_, _| Err("no reader in tests".into()),
                write: |d, _| Ok(cadcraft_io::write_dxf(d).into_bytes()),
                plot: Some(|d, space, options| cadcraft_io::plotter::plot(d, space, options).map_err(|error| error.to_string())),
            });
            app.run("ubplot.scan", json!({"mode": "window", "windows": [[[0, 0], [420, 297]]], "scale": "fit"})).unwrap();
            app.ui.dialog = dialog.map(str::to_string);
        } else if let Some(dialog) = dialog {
            let command = match dialog {
                crate::customize::DIALOG => "ui.customize.open",
                crate::palette::DIALOG => "ui.palette",
                other => other,
            };
            crate::menus::activate(&mut app, command);
        }
        let drawing = app.session.doc().unwrap();
        // Drawing table names remain user data even when they happen to match a catalog key.
        let names: BTreeSet<_> = drawing
            .layers
            .iter()
            .map(|entry| entry.name.clone())
            .chain(drawing.linetypes.iter().map(|entry| entry.name.clone()))
            .chain(drawing.text_styles.iter().map(|entry| entry.name.clone()))
            .chain(drawing.dim_styles.iter().map(|entry| entry.name.clone()))
            .chain(drawing.mleader_styles.iter().map(|entry| entry.name.clone()))
            .chain(drawing.table_styles.iter().map(|entry| entry.name.clone()))
            .chain(drawing.layouts.iter().map(|entry| entry.name.clone()))
            .chain(drawing.blocks.keys().cloned())
            .chain(std::iter::once(app.session.state().unwrap().title.clone()))
            .collect();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1366.0, 768.0)).build_ui_state(
            |ui, app: &mut CadApp| {
                app.logic(&ui.ctx().clone());
                app.ui(ui);
            },
            app,
        );
        h.run_steps(5);
        assert_eq!(h.state().language(), "vi", "{screen}");
        assert_eq!(h.state().ui.dialog.as_deref(), dialog, "{screen}");
        if screen == "LINE" {
            assert_eq!(h.state().session.running.as_ref().map(|command| command.id.as_str()), Some("line"));
        }
        let text = painted_text(&h.output().shapes);
        assert!(text.contains(tr("vi", heading)), "{screen}: missing translated heading {heading}: {text}");
        for text in text.lines() {
            let text = text.trim();
            // Upper-case command IDs and bracketed input keywords retain the engine's spelling.
            let identifier = text.chars().any(char::is_alphabetic) && text.chars().all(|c| !c.is_alphabetic() || c.is_uppercase());
            if names.contains(text) || identifier || (text.starts_with('[') && text.ends_with(']')) {
                continue;
            }
            for ((_, key), translation) in catalog {
                if text == *key && key != translation {
                    leaks.insert(format!("{screen}: {text}"));
                }
            }
        }
    }
    assert!(leaks.is_empty(), "English catalog text in Vietnamese P0 screens:\n{}", leaks.into_iter().collect::<Vec<_>>().join("\n"));
}

#[test]
fn live_switch_translates_the_shell_without_mutating_drawing_or_automation() {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.ui.in_window_menu = true;
    app.session.state_mut().unwrap().title = "Open".into();
    let ctx = egui::Context::default();
    app.logic(&ctx);
    let before = render(&mut app, &ctx);
    assert!(before.contains("Properties"));
    let commands = crate::control::all_commands(&app);
    let (menu_request, _) = crate::ControlRequest::new("ui.menu.list", json!({}));
    let crate::control::Outcome::Done(menu_before) = crate::control::handle(&mut app, &ctx, &menu_request) else { panic!("expected menu reply") };
    app.start("line");
    let prompt = crate::control::cmdline_state(&app);
    app.run("ui.language", json!({"lang":"uk"})).unwrap();
    let after = render(&mut app, &ctx);
    assert!(after.contains("Властивості"), "{after}");
    assert!(after.contains("Файл"), "{after}");
    assert!(after.contains("Укажіть першу точку"), "{after}");
    assert!(after.contains("Open"), "the drawing title must remain unchanged");
    assert_eq!(crate::control::cmdline_state(&app), prompt);
    assert_eq!(crate::control::all_commands(&app), commands);
    let crate::control::Outcome::Done(menu_after) = crate::control::handle(&mut app, &ctx, &menu_request) else { panic!("expected menu reply") };
    assert_eq!(menu_before, menu_after);
    assert_eq!(app.session.state().unwrap().title, "Open");
    assert_eq!(t("Properties"), "Properties", "language must not leak out of the render scope");
    assert!(app.run("ui.language", json!({"lang":"bad"})).is_err());
    assert_eq!(app.language(), "uk");
    app.run("ui.resetpalettes", json!({})).unwrap();
    assert_eq!(app.language(), "uk");
    app.run("ui.language", json!({"lang":"en"})).unwrap();
    assert!(render(&mut app, &ctx).contains("Properties"));
    // The menu tree used by the control channel keeps its original labels and IDs.
    assert_eq!(crate::menus::tree(&app).first().unwrap().0, "File");
}

#[test]
fn dialogs_render_in_ukrainian_and_keep_their_source_ids() {
    for (dialog, heading) in [
        ("dsettings", "Налаштування креслення"),
        ("about", "Про UbotCAD"),
        ("commands", "Довідник команд"),
        ("blocks", "Блоки"),
        ("layers", "Диспетчер властивостей шарів"),
        ("qselect", "Швидкий вибір"),
        ("parameters", "Диспетчер параметрів"),
        ("language", "Мова інтерфейсу"),
        ("hatch", "Штрихування та градієнт"),
    ] {
        let mut app = CadApp::new(Session::new(), Services::default());
        app.ui.interface_language = Preference::Ukrainian;
        app.ui.dialog = Some(dialog.into());
        let ctx = egui::Context::default();
        app.logic(&ctx);
        let text = render(&mut app, &ctx);
        assert!(text.contains(heading), "{dialog}: {text}");
        assert_eq!(app.ui.dialog.as_deref(), Some(dialog));
    }
}

#[test]
fn default_fonts_cover_every_german_special_letter_in_the_catalog() {
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    let letters: BTreeSet<_> = include_str!("de.tsv").chars().filter(|c| ('\u{00c0}'..='\u{00ff}').contains(c)).collect();
    ctx.fonts_mut(|view| {
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let mut font = view.fonts.font(&family);
            for c in &letters {
                assert!(font.characters().contains_key(c), "missing {c} in {family:?}");
            }
        }
    });
}

#[test]
fn default_fonts_cover_every_brazilian_portuguese_letter_in_the_catalog() {
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    let letters: BTreeSet<_> = include_str!("pt_br.tsv").chars().filter(|c| ('\u{00c0}'..='\u{00ff}').contains(c)).collect();
    ctx.fonts_mut(|view| {
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let mut font = view.fonts.font(&family);
            for c in &letters {
                assert!(font.characters().contains_key(c), "missing {c} in {family:?}");
            }
        }
    });
}

#[test]
fn default_fonts_cover_every_ukrainian_letter_in_the_catalog() {
    let ctx = egui::Context::default();
    // Use only egui's bundled fonts, exactly as the web build does (no system-font loader).
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    let letters: BTreeSet<_> = include_str!("uk.tsv").chars().filter(|c| ('\u{0400}'..='\u{04ff}').contains(c)).collect();
    ctx.fonts_mut(|view| {
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            // In egui 0.36, has_glyph can report a false negative for glyphs in the same
            // face as the replacement character. Check the family's actual character maps.
            let mut font = view.fonts.font(&family);
            for c in &letters {
                assert!(font.characters().contains_key(c), "missing {c} in {family:?}");
            }
        }
    });
}

#[test]
fn czech_locales_preferences_and_plural_forms() {
    for tag in ["cs", "cs-CZ", "cs_CZ.UTF-8", "CS-cz", " cs-CZ ", "cs_CZ@euro"] {
        assert_eq!(locale_language(tag), Some("cs"), "{tag}");
    }
    for tag in ["cz", "csharp", "csomething"] {
        assert_eq!(locale_language(tag), None, "{tag}");
    }
    assert_eq!(Preference::parse("cs"), Some(Preference::Czech));
    assert_eq!(Preference::Czech.name(), "Čeština");
    assert_eq!(Preference::Czech.resolve(&["en-US".into()]), "cs");
    assert_eq!(Preference::Auto.resolve(&["pl-PL".into(), "cs-CZ".into(), "en-US".into()]), "cs");
    let mut app = CadApp::new(Session::new(), Services::default());
    app.ui.interface_language = Preference::Czech;
    let mut restored = CadApp::new(Session::new(), Services::default());
    restored.load_prefs(&app.prefs_json());
    assert_eq!(restored.ui.interface_language, Preference::Czech);
    with_language("cs", || {
        for (n, form) in [(0, 2), (1, 0), (2, 1), (4, 1), (5, 2), (11, 2), (21, 2), (22, 2), (25, 2), (u64::MAX, 2)] {
            assert_eq!(plural_cs(n), form, "{n}");
            let contributor = ["přispěvatel", "přispěvatelé", "přispěvatelů"][form];
            let commit = ["commit", "commity", "commitů"][form];
            assert_eq!(count(n, "{n} contributor", "{n} contributors"), format!("{n} {contributor}"));
            assert_eq!(count(n, "{n} commit", "{n} commits"), format!("{n} {commit}"));
        }
        assert_eq!(format("Current: {color}", &[("color", "{n} Open".into())]), "Aktuální: {n} Open");
    });
}

#[test]
fn czech_menu_headings_keep_object_labels_and_other_catalogs() {
    with_language("cs", || {
        assert_eq!(menu("View"), "Zobrazení");
        assert_eq!(t("View"), "Pohled");
        assert_eq!(menu("Dimension"), "Kótování");
        assert_eq!(t("Dimension"), "Kóta");
        assert_eq!(menu("Draw"), "Kreslení");
        assert_eq!(menu("Modify"), "Modifikace");
        assert_eq!(menu("Edit"), "Úpravy");
        assert_eq!(menu("File"), "Soubor");
    });
    for language in ["en", "uk", "de", "es", "pt-BR", "unknown"] {
        for source in crate::menus::MENUS {
            assert_eq!(tr_menu(language, source), tr(language, source));
        }
    }
}

#[test]
fn live_czech_switch_preserves_drawing_command_and_saved_choice() {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.ui.in_window_menu = true;
    app.session.state_mut().unwrap().title = "Open".into();
    let ctx = egui::Context::default();
    app.logic(&ctx);
    assert!(render(&mut app, &ctx).contains("Properties"));
    app.start("line");
    let prompt = crate::control::cmdline_state(&app);
    let commands = crate::control::all_commands(&app);
    app.run("ui.language", json!({"lang":"cs"})).unwrap();
    let text = render(&mut app, &ctx);
    for label in ["Vlastnosti", "Soubor", "Zobrazení", "Kreslení", "Kótování", "Určete první bod"] {
        assert!(text.contains(label), "missing {label}: {text}");
    }
    assert_eq!(app.language(), "cs");
    assert_eq!(app.session.state().unwrap().title, "Open");
    assert_eq!(crate::control::cmdline_state(&app), prompt);
    assert_eq!(crate::control::all_commands(&app), commands);
    assert_eq!(t("Properties"), "Properties");
    app.run("ui.resetpalettes", json!({})).unwrap();
    assert_eq!(app.language(), "cs");
    app.run("ui.language", json!({"lang":"en"})).unwrap();
    assert!(render(&mut app, &ctx).contains("Properties"));
}

#[test]
fn czech_dialogs_render_with_stable_ids() {
    for (dialog, heading) in [
        ("dsettings", "Nastavení kreslení"),
        ("about", "O aplikaci UbotCAD"),
        ("commands", "Přehled příkazů"),
        ("blocks", "Bloky"),
        ("layers", "Správce vlastností hladin"),
        ("qselect", "Rychlý výběr"),
        ("parameters", "Správce parametrů"),
        ("language", "Čeština"),
        ("options:preferences", "Uživatelské předvolby"),
    ] {
        let mut app = CadApp::new(Session::new(), Services::default());
        app.ui.interface_language = Preference::Czech;
        app.ui.dialog = Some(dialog.into());
        let ctx = egui::Context::default();
        app.logic(&ctx);
        let text = render(&mut app, &ctx);
        assert!(text.contains(heading), "{dialog}: {text}");
        assert_eq!(app.ui.dialog.as_deref(), Some(dialog));
    }
}

#[test]
fn default_fonts_cover_czech_letters_and_typographic_quotes() {
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    let letters: BTreeSet<_> = "ÁČĎÉĚÍŇÓŘŠŤÚŮÝŽáčďéěíňóřšťúůýž„“".chars().collect();
    ctx.fonts_mut(|view| {
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let mut font = view.fonts.font(&family);
            for c in &letters {
                assert!(font.characters().contains_key(c), "missing {c} in {family:?}");
            }
        }
    });
}

#[test]
fn russian_and_french_plurals() {
    with_language("ru", || {
        for (n, word) in [(1, "участник"), (2, "участника"), (5, "участников"), (21, "участник"), (22, "участника"), (11, "участников")]
        {
            assert_eq!(count(n, "{n} contributor", "{n} contributors"), format!("{n} {word}"));
        }
    });
    with_language("fr", || {
        for (n, word) in [(0, "contributeur"), (1, "contributeur"), (2, "contributeurs"), (10, "contributeurs")] {
            assert_eq!(count(n, "{n} contributor", "{n} contributors"), format!("{n} {word}"));
        }
    });
}

#[test]
fn italian_locales_plurals_and_accents() {
    for tag in ["it", "it-IT", "it_IT.UTF-8", "IT-ch", " it-CH ", "it_IT@euro"] {
        assert_eq!(locale_language(tag), Some("it"), "{tag}");
    }
    assert_eq!(locale_language("italic"), None);
    assert_eq!(Preference::parse("it"), Some(Preference::Italian));
    assert_eq!(Preference::Auto.resolve(&["it-IT".into(), "en-US".into()]), "it");
    assert_eq!(Preference::Italian.resolve(&["uk-UA".into()]), "it");
    with_language("it", || {
        for (n, word) in [(0, "collaboratori"), (1, "collaboratore"), (2, "collaboratori"), (21, "collaboratori")] {
            assert_eq!(count(n, "{n} contributor", "{n} contributors"), format!("{n} {word}"));
        }
        assert_eq!(t("Draw"), "Disegna");
        assert_eq!(tr_menu("it", "Modify"), "Edita");
        assert_eq!(tr("it", "Modify"), "Modifica");
        assert_eq!(tr_menu("it", "View"), "Visualizza");
        assert_eq!(tr("it", "View"), "Vista");
    });
    // Italian accented letters (à è é ì ò ù and capitals) come from the bundled egui fonts.
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    let letters: BTreeSet<_> = include_str!("it.tsv").chars().filter(|c| ('\u{00c0}'..='\u{00ff}').contains(c)).collect();
    assert!(letters.contains(&'à') && letters.contains(&'è'));
    ctx.fonts_mut(|view| {
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let mut font = view.fonts.font(&family);
            for c in &letters {
                assert!(font.characters().contains_key(c), "missing {c} in {family:?}");
            }
        }
    });
}

/// Chinese shows only with an installed CJK font (none is bundled); without one the interface
/// stays English instead of drawing boxes, and choosing it says why.
#[test]
fn chinese_needs_an_installed_cjk_font() {
    assert_eq!(locale_language("zh-CN"), Some("zh"));
    let mut app = CadApp::new(Session::new(), Services::default());
    let r = app.run("ui.language", json!({"lang": "zh"})).unwrap();
    assert_eq!(r["interfaceLanguage"], "zh");
    let font = crate::theme::cjk_font();
    assert_eq!(app.language(), if font.is_some() { "zh" } else { "en" });
    assert_eq!(r["shown"], app.language());
    assert!(crate::theme::can_show("uk") && crate::theme::can_show("en"));
    if font.is_none() {
        assert!(app.status.as_ref().is_some_and(|(s, _)| s.contains("No Chinese font")));
    }
}

#[test]
fn hatch_dialog_extents_label_says_extents_in_ukrainian() {
    assert_eq!(tr("uk", "Default to boundary extents"), "Типово за габаритами контуру");
}

#[test]
fn korean_representative_translations_remain_available() {
    let ko = catalog("ko").unwrap();
    // Fixed regression examples; new source strings remain reported, not required for Korean.
    assert!(REPORTED.contains(&"ko"));
    for (context, source, expected) in [
        ("", "File", "파일"),
        ("menu", "View", "보기"),
        ("menu", "Draw", "그리기"),
        ("menu", "Dimension", "치수"),
        ("menu", "Modify", "수정"),
        ("", "Options...", "옵션..."),
        ("", "User Preferences", "사용자 기본 설정"),
        ("", "Interface Language", "인터페이스 언어"),
        ("", "Properties", "특성"),
        ("", "Degree", "도 기호"),
        ("", "Style: Standard", "스타일: Standard"),
        ("", "WCS ⌄", "WCS 선택"),
        ("@plural", "{n} contributor", "기여자 {n}명"),
        ("@plural", "{n} commit", "커밋 {n}개"),
    ] {
        assert_eq!(ko.get(&(context, source)).copied(), Some(expected), "{context}: {source}");
    }
    assert_eq!(tr("ko", "future untranslated label"), "future untranslated label");
}

#[test]
fn korean_locale_saved_preference_and_single_count_form() {
    for tag in ["ko", "ko-KR", "ko_KR.UTF-8", "KO-kr", " ko-KR ", "ko_KR@euro"] {
        assert_eq!(locale_language(tag), Some("ko"), "{tag}");
    }
    for tag in ["kr", "korean", "kotlin"] {
        assert_eq!(locale_language(tag), None, "{tag}");
    }
    assert_eq!(Preference::parse("ko"), Some(Preference::Korean));
    assert_eq!(Preference::Korean.resolve(&["en-US".into()]), "ko");
    assert_eq!(Preference::Auto.resolve(&["pl-PL".into(), "ko-KR".into(), "en-US".into()]), "ko");
    let mut app = CadApp::new(Session::new(), Services::default());
    app.system_languages = vec!["ko-KR".into()];
    app.run("ui.language", json!({"lang": "ko"})).unwrap();
    let mut restored = CadApp::new(Session::new(), Services::default());
    restored.system_languages = vec!["en-US".into()];
    restored.load_prefs(&app.prefs_json());
    assert_eq!(restored.ui.interface_language, Preference::Korean);
    assert_eq!(restored.language(), app.language());
    with_language("ko", || {
        for n in [0, 1, 2, 5, 11, 21, 101, u64::MAX] {
            assert_eq!(plural_single(n), 0);
            assert_eq!(count(n, "{n} contributor", "{n} contributors"), format!("기여자 {n}명"));
        }
        assert_eq!(format("Current: {color}", &[("color", "{n} Open".into())]), "현재: {n} Open");
    });
}

#[test]
fn korean_live_switch_keeps_automation_and_drawing_and_falls_back_without_font() {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.ui.in_window_menu = true;
    app.session.state_mut().unwrap().title = "Open".into();
    let ctx = egui::Context::default();
    app.logic(&ctx);
    assert!(render(&mut app, &ctx).contains("Properties"));
    app.start("line");
    let prompt = crate::control::cmdline_state(&app);
    let commands = crate::control::all_commands(&app);
    let result = app.run("ui.language", json!({"lang": "ko"})).unwrap();
    let supported = crate::theme::can_show("ko");
    assert_eq!(result["shown"], if supported { "ko" } else { "en" });
    assert_eq!(app.ui.interface_language, Preference::Korean);
    app.logic(&ctx);
    assert_eq!(app.fonts_language, Some(app.language()));
    let text = render(&mut app, &ctx);
    if supported {
        for label in ["특성", "파일", "첫 번째 점 지정"] {
            assert!(text.contains(label), "missing {label}: {text}");
        }
    } else {
        assert!(text.contains("Properties"));
        assert!(app.status.as_ref().is_some_and(|(s, _)| s.contains("No font covers the Korean interface")));
        assert_eq!(Preference::Korean.name(), "Korean");
    }
    assert_eq!(app.session.state().unwrap().title, "Open");
    assert_eq!(crate::control::cmdline_state(&app), prompt);
    assert_eq!(crate::control::all_commands(&app), commands);
    assert_eq!(t("Properties"), "Properties");
    assert!(app.run("ui.language", json!({"lang": "bad"})).is_err());
    app.run("ui.resetpalettes", json!({})).unwrap();
    assert_eq!(app.ui.interface_language, Preference::Korean);
    // The general ui.set route uses the same saved preference and font installation.
    let (request, _) = crate::ControlRequest::new("ui.set", json!({"interfaceLanguage": "zh"}));
    let _ = crate::control::handle(&mut app, &ctx, &request);
    assert_eq!(app.ui.interface_language, Preference::Chinese);
    app.logic(&ctx);
    assert_eq!(app.fonts_language, Some(if crate::theme::can_show("zh") { "zh" } else { "en" }));
    render(&mut app, &ctx);
    let (request, _) = crate::ControlRequest::new("ui.set", json!({"interfaceLanguage": "ko"}));
    let _ = crate::control::handle(&mut app, &ctx, &request);
    assert_eq!(app.ui.interface_language, Preference::Korean);
    app.logic(&ctx);
    assert_eq!(app.fonts_language, Some(app.language()));
    let text = render(&mut app, &ctx);
    assert!(text.contains(if supported { "특성" } else { "Properties" }));
    let (request, _) = crate::ControlRequest::new("ui.set", json!({"interfaceLanguage": "en"}));
    let _ = crate::control::handle(&mut app, &ctx, &request);
    app.logic(&ctx);
    assert_eq!(app.fonts_language, Some("en"));
    assert_eq!(app.ui.interface_language, Preference::English);
    assert!(render(&mut app, &ctx).contains("Properties"));
    assert_eq!(crate::menus::tree(&app).first().unwrap().0, "File");
}

#[test]
fn korean_dialogs_keep_source_ids() {
    if !crate::theme::can_show("ko") {
        return; // The fallback path is covered above; do not claim glyph rendering here.
    }
    for (dialog, source) in [
        ("dsettings", "Drafting Settings"),
        ("about", "About UbotCAD"),
        ("commands", "Command Reference"),
        ("blocks", "Blocks"),
        ("layers", "Layer Properties Manager"),
        ("qselect", "Quick Select"),
        ("parameters", "Parameters Manager"),
        ("language", "Interface Language"),
        ("options:preferences", "User Preferences"),
    ] {
        let mut app = CadApp::new(Session::new(), Services::default());
        app.ui.interface_language = Preference::Korean;
        app.ui.dialog = Some(dialog.into());
        let ctx = egui::Context::default();
        app.logic(&ctx);
        let text = render(&mut app, &ctx);
        assert!(text.contains(tr("ko", source)), "{dialog}: {text}");
        if dialog == "language" {
            assert!(text.contains("한국어"), "{dialog}: {text}");
        }
        assert_eq!(app.ui.dialog.as_deref(), Some(dialog));
    }
}

#[test]
fn korean_ui_fonts_cover_catalog_and_language_picker_in_both_families() {
    if !crate::theme::can_show("ko") {
        return;
    }
    let letters: BTreeSet<_> = include_str!("ko.tsv")
        .lines()
        .filter_map(|line| line.split('\t').nth(2))
        .flat_map(str::chars)
        .chain("한국어".chars())
        .filter(|c| !c.is_ascii())
        .collect();
    // The picker shows 한국어 in an English interface only when the CJK fonts loaded in every
    // language cover Hangul; the Korean interface itself always does, with a fallback font.
    let ctx = egui::Context::default();
    let english = crate::theme::cjk_covers_hangul();
    assert_eq!(with_language("en", || Preference::Korean.name()), if english { "한국어" } else { "Korean" });
    assert_eq!(with_language("ko", || Preference::Korean.name()), "한국어");
    crate::theme::install_fonts(&ctx, if english { "en" } else { "ko" });
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    let missing = ctx.fonts_mut(|view| {
        let mut missing = Vec::new();
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let mut font = view.fonts.font(&family);
            for c in &letters {
                if !font.characters().contains_key(c) {
                    missing.push(format!("{family:?}: {c} (U+{:04X})", u32::from(*c)));
                }
            }
        }
        missing
    });
    assert!(missing.is_empty(), "missing Korean UI glyphs: {}", missing.join(", "));
}

#[test]
fn swedish_locales_preferences_and_plural_forms() {
    for tag in ["sv", "sv-SE", "sv_SE.UTF-8", "SV-se", " sv-SE ", "sv-FI"] {
        assert_eq!(locale_language(tag), Some("sv"), "{tag}");
    }
    // `se` is Northern Sami and `swe` the three-letter code; neither picks Swedish.
    for tag in ["se", "swe", "svenska"] {
        assert_eq!(locale_language(tag), None, "{tag}");
    }
    assert_eq!(Preference::parse("sv"), Some(Preference::Swedish));
    assert_eq!(Preference::Swedish.name(), "Svenska");
    assert_eq!(Preference::Swedish.resolve(&["en-US".into()]), "sv");
    assert_eq!(Preference::Auto.resolve(&["pl-PL".into(), "sv-SE".into(), "en-US".into()]), "sv");
    let mut app = CadApp::new(Session::new(), Services::default());
    app.ui.interface_language = Preference::Swedish;
    let mut restored = CadApp::new(Session::new(), Services::default());
    restored.load_prefs(&app.prefs_json());
    assert_eq!(restored.ui.interface_language, Preference::Swedish);
    with_language("sv", || {
        for n in [0, 1, 2, 5, 21, u64::MAX] {
            // `bidragsgivare` is the same in the singular and the plural; `commit` takes an -s.
            assert_eq!(count(n, "{n} contributor", "{n} contributors"), format!("{n} bidragsgivare"));
            let commit = if n == 1 { "commit" } else { "commits" };
            assert_eq!(count(n, "{n} commit", "{n} commits"), format!("{n} {commit}"));
        }
        assert_eq!(format("Current: {color}", &[("color", "{n} Open".into())]), "Aktuell: {n} Open");
    });
}

#[test]
fn swedish_menu_headings_keep_object_labels() {
    with_language("sv", || {
        assert_eq!(menu("Dimension"), "Måttsättning");
        assert_eq!(t("Dimension"), "Mått");
        assert_eq!(menu("View"), "Visa");
        assert_eq!(menu("Draw"), "Rita");
        assert_eq!(menu("Modify"), "Ändra");
        assert_eq!(menu("Edit"), "Redigera");
        assert_eq!(menu("File"), "Arkiv");
    });
}

#[test]
fn live_swedish_switch_preserves_drawing_command_and_saved_choice() {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.ui.in_window_menu = true;
    app.session.state_mut().unwrap().title = "Open".into();
    let ctx = egui::Context::default();
    app.logic(&ctx);
    assert!(render(&mut app, &ctx).contains("Properties"));
    app.start("line");
    let prompt = crate::control::cmdline_state(&app);
    let commands = crate::control::all_commands(&app);
    app.run("ui.language", json!({"lang":"sv"})).unwrap();
    let text = render(&mut app, &ctx);
    for label in ["Egenskaper", "Arkiv", "Visa", "Rita", "Måttsättning", "Ange första punkten"] {
        assert!(text.contains(label), "missing {label}: {text}");
    }
    assert_eq!(app.language(), "sv");
    assert_eq!(app.session.state().unwrap().title, "Open");
    assert_eq!(crate::control::cmdline_state(&app), prompt);
    assert_eq!(crate::control::all_commands(&app), commands);
    app.run("ui.language", json!({"lang":"en"})).unwrap();
    assert!(render(&mut app, &ctx).contains("Properties"));
}

#[test]
fn swedish_dialogs_render_with_stable_ids() {
    for (dialog, heading) in [
        ("dsettings", "Ritinställningar"),
        ("about", "Om UbotCAD"),
        ("blocks", "Block"),
        ("layers", "Hantera lageregenskaper"),
        ("qselect", "Snabbmarkering"),
        ("parameters", "Hantera parametrar"),
        ("language", "Svenska"),
        ("options:preferences", "Användarinställningar"),
    ] {
        let mut app = CadApp::new(Session::new(), Services::default());
        app.ui.interface_language = Preference::Swedish;
        app.ui.dialog = Some(dialog.into());
        let ctx = egui::Context::default();
        app.logic(&ctx);
        let text = render(&mut app, &ctx);
        assert!(text.contains(heading), "{dialog}: {text}");
        assert_eq!(app.ui.dialog.as_deref(), Some(dialog));
    }
}

#[test]
fn default_fonts_cover_swedish_letters_and_quotes() {
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    let letters: BTreeSet<_> = "ÅÄÖÉåäöé”–".chars().collect();
    ctx.fonts_mut(|view| {
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let mut font = view.fonts.font(&family);
            for c in &letters {
                assert!(font.characters().contains_key(c), "missing {c} in {family:?}");
            }
        }
    });
}

#[test]
fn formatted_prompt_translates_template_and_keeps_user_text() {
    use cadcraft_engine::prompt::{Accept, Prompt, PromptArg};

    with_language("uk", || {
        let p = Prompt::tf("Enter value for {tag}", vec![("tag", PromptArg::Text("ROOM".into()))], Accept::TEXT);
        assert_ne!(tr("uk", "Enter value for {tag}"), "Enter value for {tag}");
        assert_eq!(prompt_message(&p), tr("uk", "Enter value for {tag}").replace("{tag}", "ROOM"));

        let p = Prompt::tf(
            "{title} {message}",
            vec![("title", PromptArg::Text("** STRETCH **".into())), ("message", PromptArg::Key("Specify stretch point"))],
            Accept::POINT,
        );
        assert_ne!(tr("uk", "Specify stretch point"), "Specify stretch point");
        assert_eq!(prompt_message(&p), format!("** STRETCH ** {}", tr("uk", "Specify stretch point")));

        let p = Prompt::tf(
            "{title} {message}",
            vec![("title", PromptArg::Text("{message} Вітальня".into())), ("message", PromptArg::Key("Specify stretch point"))],
            Accept::POINT,
        );
        assert_eq!(prompt_message(&p), format!("{{message}} Вітальня {}", tr("uk", "Specify stretch point")));
        assert_eq!(prompt_message(&Prompt::new("Select objects", Accept::SELECT)), tr("uk", "Select objects"));
    });
}

#[test]
fn vietnamese_locales_preferences_and_single_count_form() {
    for tag in ["vi", "vi-VN", "vi_VN.UTF-8", "VI-vn", " vi-VN "] {
        assert_eq!(locale_language(tag), Some("vi"), "{tag}");
    }
    for tag in ["vie", "vn"] {
        assert_eq!(locale_language(tag), None, "{tag}");
    }
    assert_eq!(Preference::parse("vi"), Some(Preference::Vietnamese));
    assert_eq!(Preference::Vietnamese.name(), "Tiếng Việt");
    assert_eq!(Preference::Vietnamese.resolve(&["en-US".into()]), "vi");
    assert_eq!(Preference::Auto.resolve(&["pl-PL".into(), "vi-VN".into()]), "vi");
    assert_eq!(Preference::restore(Some("vi")), Preference::Vietnamese);
    assert_eq!(serde_json::to_string(&Preference::Vietnamese).unwrap(), "\"vi\"");
    assert_eq!(serde_json::from_str::<Preference>("\"vi\"").unwrap(), Preference::Vietnamese);
    let mut app = CadApp::new(Session::new(), Services::default());
    app.ui.interface_language = Preference::Vietnamese;
    let mut restored = CadApp::new(Session::new(), Services::default());
    restored.load_prefs(&app.prefs_json());
    assert_eq!(restored.ui.interface_language, Preference::Vietnamese);
    with_language("vi", || {
        for n in [0, 1, 2, 5, 21, u64::MAX] {
            assert_eq!(plural_single(n), 0);
            assert_eq!(count(n, "{n} contributor", "{n} contributors"), format!("{n} người đóng góp"));
        }
        assert_eq!(format("Current: {color}", &[("color", "{n} Open".into())]), "Hiện hành: {n} Open");
    });
}

#[test]
fn ribbon_tab_labels_follow_the_uf_table() {
    for (en, vi) in [
        ("Home", "Vẽ"),
        ("Insert", "Chèn"),
        ("Annotate", "Ghi chú"),
        ("Layers", "Lớp bản vẽ"),
        ("View", "Hiển thị"),
        ("Layout", "Bố cục in"),
        ("Properties", "Thuộc tính"),
        ("Xref", "Tham chiếu ngoài"),
        ("Plot", "In bản vẽ"),
    ] {
        assert_eq!(tr_ribbon("vi", en), vi, "{en}");
        assert_eq!(with_language("vi", || ribbon(en)), vi, "{en}");
    }
    assert_eq!(tr_ribbon("uk", "Home"), tr("uk", "Home"));
    assert_eq!(tr_ribbon("vi", "Open"), tr("vi", "Open"));
    assert_eq!(tr_ribbon("unknown", "Home"), "Home");
}

#[test]
fn live_vietnamese_switch_preserves_drawing_command_and_saved_choice() {
    if !crate::theme::can_show("vi") {
        eprintln!("Vietnamese rendering skipped: no covering UI font; fallback is tested separately");
        return;
    }
    let mut app = CadApp::new(Session::new(), Services::default());
    app.ui.in_window_menu = true;
    app.session.state_mut().unwrap().title = "Open".into();
    let ctx = egui::Context::default();
    app.logic(&ctx);
    assert!(render(&mut app, &ctx).contains("Properties"));
    app.start("line");
    let prompt = crate::control::cmdline_state(&app);
    let commands = crate::control::all_commands(&app);
    let menus = format!("{:?}", crate::menus::tree(&app));
    let drawing = app.session.state().unwrap().clone();
    app.run("ui.language", json!({"lang":"vi"})).unwrap();
    app.logic(&ctx);
    let text = render(&mut app, &ctx);
    for label in ["Thuộc tính", "Tệp", "Chỉ định điểm đầu"] {
        assert!(text.contains(label), "missing {label}: {text}");
    }
    assert_eq!(app.language(), "vi");
    assert_eq!(app.fonts_language, Some("vi"));
    let mut restored = CadApp::new(Session::new(), Services::default());
    restored.load_prefs(&app.prefs_json());
    assert_eq!(restored.ui.interface_language, Preference::Vietnamese);
    app.ui.dialog = Some("language".into());
    for language in ["en", "vi", "en"] {
        app.run("ui.language", json!({"lang":language})).unwrap();
        app.logic(&ctx);
        let text = render(&mut app, &ctx);
        assert!(text.contains(if language == "vi" { "Ngôn ngữ giao diện" } else { "Interface Language" }), "{text}");
        assert_eq!(app.ui.dialog.as_deref(), Some("language"));
        assert_eq!(crate::control::cmdline_state(&app), prompt);
        assert_eq!(crate::control::all_commands(&app), commands);
        assert_eq!(format!("{:?}", crate::menus::tree(&app)), menus);
        let state = app.session.state().unwrap();
        assert_eq!(state.title, "Open");
        assert!(std::sync::Arc::ptr_eq(&state.doc, &drawing.doc));
        assert_eq!(state.selection, drawing.selection);
        assert_eq!(state.views, drawing.views);
        assert_eq!(state.is_dirty(), drawing.is_dirty());
    }
    app.ui.dialog = None;
    assert!(render(&mut app, &ctx).contains("Properties"));
}

#[test]
fn user_named_ribbon_tab_is_not_translated() {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.ws.effective.layout.ribbon = crate::wsprofile::presets::preset("AUTOCAD_FAMILIAR").unwrap().layout.ribbon;
    app.ws.effective.layout.ribbon.visible = true;
    let home = app.ws.effective.layout.ribbon.tabs.iter().find(|tab| tab.id == "home").unwrap().clone();
    let mut renamed = home.clone();
    renamed.label = "Bản vẽ kiến trúc".into();
    let mut custom = home.clone();
    custom.id = "custom_1".into();
    custom.label = "Home".into();
    app.ws.effective.layout.ribbon.tabs.push(crate::wsprofile::schema::RibbonTab {
        id: "custom_2".into(),
        label: "Bản vẽ kiến trúc".into(),
        ..Default::default()
    });
    let ctx = egui::Context::default();
    for language in ["vi", "en"] {
        with_language(language, || {
            assert_eq!(crate::ribbon::render::tab_label(&home), if language == "vi" { "Vẽ" } else { "Home" });
            assert_eq!(crate::ribbon::render::tab_label(&renamed), "Bản vẽ kiến trúc");
            renamed.label = "Insert".into();
            assert_eq!(crate::ribbon::render::tab_label(&renamed), "Insert");
            renamed.label = "Bản vẽ kiến trúc".into();
            assert_eq!(crate::ribbon::render::tab_label(&custom), "Home");
            let mut group = home.groups.iter().find(|group| group.id == "draw").unwrap().clone();
            assert_eq!(crate::ribbon::render::group_label(&home, &group), if language == "vi" { "Vẽ" } else { "Draw" });
            group.label = "Insert".into();
            assert_eq!(crate::ribbon::render::group_label(&home, &group), "Insert");
            group.id = "custom_1".into();
            group.label = "Draw".into();
            assert_eq!(crate::ribbon::render::group_label(&home, &group), "Draw");
            let input =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0))), ..Default::default() };
            let mut output = ctx.run_ui(input, |ui| crate::ribbon::render::show(&mut app, ui));
            assert!(painted_text(&output.shapes).contains("Bản vẽ kiến trúc"));
            output.textures_delta.clear();
        });
    }
}

#[test]
fn vietnamese_falls_back_to_english_without_a_covering_font() {
    let mut app = CadApp::new(Session::new(), Services::default());
    let ctx = egui::Context::default();
    let result = app.run("ui.language", json!({"lang":"vi"})).unwrap();
    app.logic(&ctx);
    let supported = crate::theme::can_show("vi");
    assert_eq!(app.language(), if supported { "vi" } else { "en" });
    assert_eq!(result["shown"], app.language());
    assert_eq!(app.ui.interface_language, Preference::Vietnamese);
    let mut restored = CadApp::new(Session::new(), Services::default());
    restored.load_prefs(&app.prefs_json());
    assert_eq!(restored.ui.interface_language, Preference::Vietnamese);
    let text = render(&mut app, &ctx);
    if supported {
        assert!(text.contains("Thuộc tính"), "{text}");
    } else {
        assert!(text.contains("Properties"), "{text}");
        assert!(app.status.as_ref().is_some_and(|(s, _)| s.contains("No font covers the Vietnamese interface")));
    }
}

#[test]
fn vietnamese_ui_fonts_cover_catalog_when_shown() {
    if !crate::theme::can_show("vi") {
        eprintln!("Vietnamese glyph coverage skipped: no covering UI font");
        return;
    }
    let letters: BTreeSet<_> = include_str!("vi.tsv")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split('\t').nth(2))
        .flat_map(str::chars)
        .chain("Tiếng Việt".chars())
        .filter(|c| !c.is_ascii())
        .collect();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx, "vi");
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    let missing = ctx.fonts_mut(|view| {
        let mut missing = Vec::new();
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            let mut font = view.fonts.font(&family);
            for c in &letters {
                if !font.characters().contains_key(c) {
                    missing.push(format!("{family:?}: {c} (U+{:04X})", u32::from(*c)));
                }
            }
        }
        missing
    });
    assert!(missing.is_empty(), "missing Vietnamese UI glyphs: {}", missing.join(", "));
}

#[test]
fn glossary_is_approved_before_bulk_rows() {
    let rows = include_str!("vi.tsv").lines().filter(|line| !line.is_empty() && !line.starts_with('#')).count();
    if rows > 60 {
        let status = include_str!("vi_glossary.tsv").lines().find(|line| line.starts_with("# status:"));
        assert!(status.is_some_and(|line| line.starts_with("# status: approved ")), "bulk Vietnamese translation needs an approved glossary");
    }
}

#[test]
fn vietnamese_catalog_follows_the_glossary() {
    let (catalog, errors) = parse(include_str!("vi.tsv"));
    assert!(errors.is_empty(), "{errors:?}");
    let glossary = glossary();
    for (context, key, value) in &glossary {
        if let Some(translated) = catalog.get(&(*context, *key)) {
            assert_eq!(translated, value, "{context}: {key}");
        }
    }
    for ((context, key), value) in &catalog {
        if *context == "ribbon" {
            assert!(glossary.contains(&(*context, *key, *value)), "unknown ribbon translation: {key}");
        }
    }
    for (context, key, value) in glossary.iter().filter(|(context, key, _)| {
        *context == "ribbon" && ["Home", "Insert", "Annotate", "Layers", "View", "Layout", "Properties", "Xref", "Plot"].contains(key)
    }) {
        assert_eq!(catalog.get(&(*context, *key)), Some(value), "missing U-F ribbon row: {key}");
    }
}

#[test]
fn dynamic_layer_prompts_translate_values_without_changing_engine_text() {
    for (option, value, english, vietnamese) in [
        ("Color", "1", "Enter name list of layer(s) for color 1", "Nhập danh sách tên các lớp cần gán màu 1"),
        (
            "Ltype",
            "Continuous",
            "Enter name list of layer(s) for linetype \"Continuous\"",
            "Nhập danh sách tên các lớp cần gán kiểu đường \"Continuous\"",
        ),
        ("LWeight", "0.25", "Enter name list of layer(s) for lineweight 0.25mm", "Nhập danh sách tên các lớp cần gán độ dày nét 0.25mm"),
        ("TRansparency", "35", "Enter name list of layer(s) for transparency 35%", "Nhập danh sách tên các lớp cần gán độ trong suốt 35%"),
    ] {
        let mut session = Session::new();
        session.start("-layer").unwrap();
        session.cmdline(option).unwrap();
        session.cmdline(value).unwrap();
        let prompt = session.current_prompt().unwrap();
        assert!(prompt.template.is_some(), "{option}");
        assert_eq!(prompt.message, english);
        assert_eq!(prompt.default.as_deref(), Some("0"));
        with_language("vi", || assert_eq!(prompt_message(&prompt), vietnamese));
        with_language("en", || assert_eq!(prompt_message(&prompt), english));
        assert_eq!(prompt.message, english);
    }
}

#[test]
fn group_confirmation_keeps_placeholder_text_in_the_group_name() {
    let name = "ROOM {name} Văn phòng";
    for (selectable, english, vietnamese) in [
        (
            true,
            "Group ROOM {name} Văn phòng is selectable. Make it unselectable?",
            "Có thể chọn nhóm ROOM {name} Văn phòng. Chuyển nhóm sang không cho chọn?",
        ),
        (
            false,
            "Group ROOM {name} Văn phòng is not selectable. Make it selectable?",
            "Không thể chọn nhóm ROOM {name} Văn phòng. Chuyển nhóm sang cho phép chọn?",
        ),
    ] {
        let mut session = Session::new();
        session.doc_mut().unwrap().groups.push(cadcraft_engine::doc::Group {
            name: name.into(),
            description: String::new(),
            selectable,
            members: Vec::new(),
        });
        session.start("-group").unwrap();
        session.cmdline("Selectable").unwrap();
        session.input(cadcraft_engine::Input::Text(name.into())).unwrap();
        let prompt = session.current_prompt().unwrap();
        assert_eq!(prompt.message, english);
        with_language("vi", || assert_eq!(prompt_message(&prompt), vietnamese));
        assert_eq!(prompt.match_keyword("y").as_deref(), Some("Yes"));
    }
}

#[test]
fn ubplot_source_prompt_translates_without_changing_keyword_metadata() {
    let mut session = Session::new();
    session.start("ubplot").unwrap();
    let prompt = session.current_prompt().unwrap();
    assert!(prompt.raw);
    with_language("vi", || assert_eq!(prompt_message(&prompt), "In khung bản vẽ theo [Block/Window/Layout/Last] <Block>:"));
    with_language("en", || assert_eq!(prompt_message(&prompt), "Plot frames by [Block/Window/Layout/Last] <Block>:"));
    assert_eq!(prompt.keywords, ["Block", "Window", "Layout", "Last"]);
    assert_eq!(prompt.default.as_deref(), Some("Block"));
    assert_eq!(prompt.display(), "Plot frames by [Block/Window/Layout/Last] <Block>:");
    for (text, keyword) in [("b", "Block"), ("blo", "Block"), ("W", "Window"), ("L", "Layout"), ("last", "Last")] {
        assert_eq!(prompt.match_keyword(text).as_deref(), Some(keyword));
    }
}

#[test]
fn unknown_raw_prompt_keeps_routine_text_and_formatting() {
    let mut prompt = cadcraft_engine::Prompt::new("\nRoutine says: Specify first point {name} [Yes/No] <No>: \n", cadcraft_engine::Accept::TEXT)
        .kw(&["Yes", "No"])
        .default("No");
    prompt.raw = true;
    let expected = "Routine says: Specify first point {name} [Yes/No] <No>: ";
    for language in ["vi", "en", "qps"] {
        with_language(language, || assert_eq!(prompt_message(&prompt), expected));
    }
}
