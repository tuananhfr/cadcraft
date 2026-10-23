//! Display-time localization, following the sibling Craft apps' TSV convention.
//! English strings remain lookup keys; command IDs, drawing data and automation stay unchanged.
//! Add a catalog, a `LANGUAGES` row and a preference variant to introduce another language.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

pub mod fold;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Preference {
    #[default]
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "en")]
    English,
    #[serde(rename = "uk")]
    Ukrainian,
    #[serde(rename = "pt-BR")]
    BrazilianPortuguese,
    #[serde(rename = "de")]
    German,
    #[serde(rename = "es")]
    Spanish,
    #[serde(rename = "cs")]
    Czech,
    #[serde(rename = "ru")]
    Russian,
    #[serde(rename = "fr")]
    French,
    #[serde(rename = "zh")]
    Chinese,
    #[serde(rename = "it")]
    Italian,
    #[serde(rename = "ko")]
    Korean,
    #[serde(rename = "sv")]
    Swedish,
    #[serde(rename = "ar")]
    Arabic,
    #[serde(rename = "vi")]
    Vietnamese,
}

impl Preference {
    pub const ALL: [Self; 15] = [
        Self::Auto,
        Self::English,
        Self::Ukrainian,
        Self::BrazilianPortuguese,
        Self::German,
        Self::Spanish,
        Self::Czech,
        Self::Russian,
        Self::French,
        Self::Chinese,
        Self::Italian,
        Self::Korean,
        Self::Swedish,
        Self::Arabic,
        Self::Vietnamese,
    ];

    /// The codes a preference can be given as, for messages ("auto, en or uk").
    pub fn codes() -> String {
        let codes: Vec<&str> = Self::ALL.iter().map(|p| p.code()).collect();
        match codes.split_last() {
            Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
            _ => codes.join(""),
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::English => "en",
            Self::Ukrainian => "uk",
            Self::BrazilianPortuguese => "pt-BR",
            Self::German => "de",
            Self::Spanish => "es",
            Self::Czech => "cs",
            Self::Russian => "ru",
            Self::French => "fr",
            Self::Chinese => "zh",
            Self::Italian => "it",
            Self::Korean => "ko",
            Self::Swedish => "sv",
            Self::Arabic => "ar",
            Self::Vietnamese => "vi",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => t("Auto (system language)"),
            Self::English => "English",
            Self::Ukrainian => "Українська",
            Self::BrazilianPortuguese => "Português (Brasil)",
            Self::German => "Deutsch",
            Self::Spanish => "Español",
            Self::Czech => "Čeština",
            Self::Russian => "Русский",
            Self::French => "Français",
            Self::Chinese => "简体中文",
            Self::Italian => "Italiano",
            Self::Korean => {
                if crate::theme::shows_hangul(CURRENT.with(Cell::get)) {
                    "한국어"
                } else {
                    "Korean"
                }
            }
            Self::Swedish => "Svenska",
            Self::Arabic => "العربية",
            Self::Vietnamese => "Tiếng Việt",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.code() == code)
    }

    /// An invalid or obsolete saved choice follows the system language.
    pub fn restore(saved: Option<&str>) -> Self {
        saved.and_then(Self::parse).unwrap_or_default()
    }

    pub fn resolve(self, locales: &[String]) -> &'static str {
        match self {
            Self::English => "en",
            Self::Ukrainian => "uk",
            Self::BrazilianPortuguese => "pt-BR",
            Self::German => "de",
            Self::Spanish => "es",
            Self::Czech => "cs",
            Self::Russian => "ru",
            Self::French => "fr",
            Self::Chinese => "zh",
            Self::Italian => "it",
            Self::Korean => "ko",
            Self::Swedish => "sv",
            Self::Arabic => "ar",
            Self::Vietnamese => "vi",
            Self::Auto => locales.iter().find_map(|s| locale_language(s)).unwrap_or("en"),
        }
    }
}

pub struct Language {
    pub code: &'static str,
    pub name: &'static str,
    pub source: &'static str,
    /// Index of the `@plural` form for an integer count.
    pub plural: fn(u64) -> usize,
    /// Number of `|`-separated forms every `@plural` row must carry.
    pub plural_forms: usize,
    catalog: OnceLock<Catalog<'static>>,
    templates: OnceLock<Vec<EchoTemplate<'static>>>,
}

pub static LANGUAGES: [Language; 14] = [
    Language { code: "en", name: "English", source: "", plural: plural_en, plural_forms: 2, catalog: OnceLock::new(), templates: OnceLock::new() },
    Language {
        code: "uk",
        name: "Українська",
        source: include_str!("uk.tsv"),
        plural: plural_uk,
        plural_forms: 3,
        catalog: OnceLock::new(),
        templates: OnceLock::new(),
    },
    Language {
        code: "pt-BR",
        name: "Português (Brasil)",
        source: include_str!("pt_br.tsv"),
        plural: plural_pt,
        plural_forms: 2,
        catalog: OnceLock::new(),
        templates: OnceLock::new(),
    },
    // German plurals follow the English rule: one (1), other.
    Language {
        code: "de",
        name: "Deutsch",
        source: include_str!("de.tsv"),
        plural: plural_en,
        plural_forms: 2,
        catalog: OnceLock::new(),
        templates: OnceLock::new(),
    },
    // Spanish integer plurals follow the English rule too.
    Language {
        code: "es",
        name: "Español",
        source: include_str!("es.tsv"),
        plural: plural_en,
        plural_forms: 2,
        catalog: OnceLock::new(),
        templates: OnceLock::new(),
    },
    Language {
        code: "cs",
        name: "Čeština",
        source: include_str!("cs.tsv"),
        plural: plural_cs,
        plural_forms: 3,
        catalog: OnceLock::new(),
        templates: OnceLock::new(),
    },
    // Russian integer plurals follow the Ukrainian rule: one (1, 21), few (2–4), many.
    Language {
        code: "ru",
        name: "Русский",
        source: include_str!("ru.tsv"),
        plural: plural_uk,
        plural_forms: 3,
        catalog: OnceLock::new(),
        templates: OnceLock::new(),
    },
    // French integer plurals (CLDR `fr`): one (0, 1), other, the Portuguese rule.
    Language {
        code: "fr",
        name: "Français",
        source: include_str!("fr.tsv"),
        plural: plural_pt,
        plural_forms: 2,
        catalog: OnceLock::new(),
        templates: OnceLock::new(),
    },
    // Chinese has no grammatical plural: both `@plural` forms read the same. Its glyphs come from
    // an installed CJK font (`theme::cjk_font`); without one the interface stays English.
    Language {
        code: "zh",
        name: "简体中文",
        source: include_str!("zh.tsv"),
        plural: plural_en,
        plural_forms: 2,
        catalog: OnceLock::new(),
        templates: OnceLock::new(),
    },
    // Italian integer plurals (CLDR `it`) follow the English rule: one (1), other.
    Language {
        code: "it",
        name: "Italiano",
        source: include_str!("it.tsv"),
        plural: plural_en,
        plural_forms: 2,
        catalog: OnceLock::new(),
        templates: OnceLock::new(),
    },
    // Korean has one integer plural form and needs a Hangul-capable interface font.
    Language {
        code: "ko",
        name: "한국어",
        source: include_str!("ko.tsv"),
        plural: plural_single,
        plural_forms: 1,
        catalog: OnceLock::new(),
        templates: OnceLock::new(),
    },
    // Swedish plurals follow the English rule: one (1), other.
    Language {
        code: "sv",
        name: "Svenska",
        source: include_str!("sv.tsv"),
        plural: plural_en,
        plural_forms: 2,
        catalog: OnceLock::new(),
        templates: OnceLock::new(),
    },
    // العربية (Arabic) is right-to-left; an installed Arabic font (`theme::arabic_font`)
    // covers its glyphs, and the shell keeps egui's left-to-right run order. Six plural
    // forms: zero, one, two, few, many, other.
    Language {
        code: "ar",
        name: "العربية",
        source: include_str!("ar.tsv"),
        plural: plural_arabic,
        plural_forms: 6,
        catalog: OnceLock::new(),
        templates: OnceLock::new(),
    },
    Language {
        code: "vi",
        name: "Tiếng Việt",
        source: include_str!("vi.tsv"),
        plural: plural_single,
        plural_forms: 1,
        catalog: OnceLock::new(),
        templates: OnceLock::new(),
    },
];

/// BCP 47 and POSIX locale tags, including `uk-UA`, `uk_UA.UTF-8` and `pt_BR.UTF-8`.
/// A catalog is chosen by the tag's primary language, so any `pt-*` tag selects `pt-BR`.
pub fn locale_language(tag: &str) -> Option<&'static str> {
    let primary = tag.trim().split(['-', '_', '.', '@']).next()?;
    LANGUAGES.iter().find(|l| l.code.split('-').next().is_some_and(|p| p.eq_ignore_ascii_case(primary))).map(|l| l.code)
}

thread_local! {
    // Scoped to a single app's render. Headless calls and parallel tests default to English.
    static CURRENT: Cell<&'static str> = const { Cell::new("en") };
    static ECHO_MEMO: RefCell<HashMap<String, HashMap<String, String>>> = RefCell::new(HashMap::new());
}

pub fn with_language<R>(language: &'static str, draw: impl FnOnce() -> R) -> R {
    struct Restore(&'static str);
    impl Drop for Restore {
        fn drop(&mut self) {
            CURRENT.set(self.0);
        }
    }
    let _restore = Restore(CURRENT.replace(language));
    draw()
}

type Catalog<'a> = HashMap<(&'a str, &'a str), &'a str>;

#[derive(Debug)]
enum EchoPart<'a> {
    Literal(&'a str),
    Capture(&'a str),
}

#[derive(Debug)]
struct EchoTemplate<'a> {
    source: &'a str,
    translated: &'a str,
    parts: Vec<EchoPart<'a>>,
    literal_bytes: usize,
}

impl<'a> EchoTemplate<'a> {
    fn new(source: &'a str, translated: &'a str) -> Option<Self> {
        let mut parts = Vec::new();
        let mut rest = source;
        let mut literal_bytes = 0;
        let mut captures = 0;
        while let Some((literal, tail)) = rest.split_once('{') {
            let (name, after) = tail.split_once('}')?;
            parts.push(EchoPart::Literal(literal));
            literal_bytes += literal.len();
            if !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                parts.push(EchoPart::Capture(name));
                captures += 1;
            } else {
                let braces = rest.get(literal.len()..literal.len() + name.len() + 2)?;
                parts.push(EchoPart::Literal(braces));
                literal_bytes += braces.len();
            }
            rest = after;
        }
        parts.push(EchoPart::Literal(rest));
        literal_bytes += rest.len();
        // Short literal fragments can match arbitrary output from user routines.
        let letters: usize = parts
            .iter()
            .map(|part| match part {
                EchoPart::Literal(text) => text.chars().filter(|ch| ch.is_alphabetic()).count(),
                EchoPart::Capture(_) => 0,
            })
            .sum();
        (captures > 0 && captures <= 16 && parts.len() <= 64 && source.len() <= 4096 && letters >= 6).then_some(Self {
            source,
            translated,
            parts,
            literal_bytes,
        })
    }

    fn capture<'s>(&self, line: &'s str) -> Option<Vec<(&'a str, &'s str)>> {
        fn visit<'a, 's>(parts: &[EchoPart<'a>], rest: &'s str, values: &mut Vec<(&'a str, &'s str)>, budget: &mut usize) -> bool {
            if *budget == 0 {
                return false;
            }
            *budget -= 1;
            match parts.split_first() {
                None => rest.is_empty(),
                Some((EchoPart::Literal(literal), tail)) => rest.strip_prefix(*literal).is_some_and(|rest| visit(tail, rest, values, budget)),
                Some((EchoPart::Capture(name), tail)) => {
                    if let Some((_, previous)) = values.iter().find(|(key, _)| key == name).copied() {
                        return rest.strip_prefix(previous).is_some_and(|rest| visit(tail, rest, values, budget));
                    }
                    for end in rest.char_indices().map(|(i, _)| i).skip(1).chain(std::iter::once(rest.len())) {
                        if *budget == 0 {
                            break;
                        }
                        if let Some((value, after)) = rest.get(..end).zip(rest.get(end..)) {
                            if value.is_empty() {
                                continue;
                            }
                            values.push((*name, value));
                            if visit(tail, after, values, budget) {
                                return true;
                            }
                            values.pop();
                        }
                    }
                    false
                }
            }
        }

        if line.len() > 16_384 {
            return None;
        }
        let mut values = Vec::new();
        let mut budget = 16_384;
        visit(&self.parts, line, &mut values, &mut budget).then_some(values)
    }
}

fn echo_templates(language: &str) -> &'static [EchoTemplate<'static>] {
    let Some(language) = LANGUAGES.iter().find(|entry| entry.code == language) else { return &[] };
    language.templates.get_or_init(|| {
        let mut templates: Vec<_> = catalog(language.code)
            .into_iter()
            .flat_map(|catalog| catalog.iter())
            .filter(|((context, _), _)| *context == "echo")
            .filter_map(|((_, source), translated)| EchoTemplate::new(source, translated))
            .collect();
        templates.sort_by(|a, b| b.literal_bytes.cmp(&a.literal_bytes).then_with(|| a.source.cmp(b.source)));
        templates
    })
}

/// Engine logs retain their English bytes; only displayed whole lines use template lookup.
pub fn echo(source: &str) -> Cow<'_, str> {
    CURRENT.with(|language| translated_echo(language.get(), source))
}

fn translated_echo<'a>(language: &str, source: &'a str) -> Cow<'a, str> {
    let Some(catalog) = catalog(language) else { return Cow::Borrowed(source) };
    if let Some(translated) = catalog.get(&("", source)).or_else(|| catalog.get(&("echo", source))) {
        return Cow::Borrowed(translated);
    }
    if let Some(translated) =
        ECHO_MEMO.with(|memo| memo.try_borrow().ok().and_then(|memo| memo.get(language).and_then(|lines| lines.get(source)).cloned()))
    {
        return Cow::Owned(translated);
    }
    let translated = translated_echo_inner(language, source, 0);
    if source.len() <= 4096 && translated.len() <= 16_384 {
        ECHO_MEMO.with(|memo| {
            if let Ok(mut memo) = memo.try_borrow_mut() {
                let lines = memo.entry(language.to_string()).or_default();
                if lines.len() >= 512 {
                    lines.clear();
                }
                lines.insert(source.to_string(), translated.to_string());
            }
        });
    }
    translated
}

fn translated_echo_inner<'a>(language: &str, source: &'a str, depth: usize) -> Cow<'a, str> {
    let Some(catalog) = catalog(language) else { return Cow::Borrowed(source) };
    if let Some(translated) = catalog.get(&("", source)).or_else(|| catalog.get(&("echo", source))) {
        return Cow::Borrowed(translated);
    }
    for template in echo_templates(language) {
        if let Some(values) = template.capture(source) {
            // Only engine error envelopes contain another message; names and ids remain opaque.
            let nested = match template.source {
                "invalid parameters for `{cmd}`: {msg}" | "; error: {msg}" => Some("msg"),
                "command `{value1}` is not available right now: {value2}"
                | "internal error in `{value1}` (the drawing was kept as it was): {value2}" => Some("value2"),
                "Internal error in {id}: {m}" | "CAL: {m}" | "QuickCalc: {m}" => Some("m"),
                "Object not converted: {why}." | "Object {value1} not converted: {why}." => Some("why"),
                "; error: {value1}" => Some("value1"),
                _ => None,
            };
            let localized: Vec<_> = values
                .iter()
                .map(|(name, value)| {
                    let value =
                        if depth < 4 && nested == Some(*name) { translated_echo_inner(language, value, depth + 1) } else { Cow::Borrowed(*value) };
                    (*name, value)
                })
                .collect();
            let values: Vec<_> = localized.iter().map(|(name, value)| (*name, value.as_ref())).collect();
            return Cow::Owned(substitute(template.translated, &values));
        }
    }
    Cow::Borrowed(source)
}

fn substitute(text: &str, values: &[(&str, &str)]) -> String {
    let mut result = String::new();
    let mut rest = text;
    while let Some((before, tail)) = rest.split_once('{') {
        result.push_str(before);
        let Some((key, after)) = tail.split_once('}') else {
            result.push('{');
            rest = tail;
            break;
        };
        if let Some((_, value)) = values.iter().find(|(name, _)| *name == key) {
            result.push_str(value);
        } else {
            result.push('{');
            result.push_str(key);
            result.push('}');
        }
        rest = after;
    }
    result.push_str(rest);
    result
}

/// Malformed rows are ignored at runtime and diagnosed by the catalog tests.
fn parse(source: &str) -> (Catalog<'_>, Vec<String>) {
    let mut entries = HashMap::new();
    let mut errors = Vec::new();
    for (i, line) in source.lines().enumerate() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let columns: Vec<_> = line.split('\t').collect();
        match columns.as_slice() {
            [context, key, value] if !key.is_empty() && !value.is_empty() => {
                if entries.insert((*context, *key), *value).is_some() {
                    errors.push(format!("line {}: duplicate key {key}", i + 1));
                }
            }
            _ => errors.push(format!("line {}: expected context, source, translation", i + 1)),
        }
    }
    (entries, errors)
}

fn catalog(language: &str) -> Option<&'static Catalog<'static>> {
    LANGUAGES.iter().find(|l| l.code == language && !l.source.is_empty()).map(|l| l.catalog.get_or_init(|| parse(l.source).0))
}

/// The borrowed return permits unchanged input above the cache cap without leaking or using Cow.
pub fn pseudo(source: &str) -> &str {
    const MAX_ENTRIES: usize = 20_000;
    const MAX_SOURCE_BYTES: usize = 4096;
    static CACHE: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();

    // Dynamic or hostile text must not turn the debug locale into an unbounded allocation.
    if source.len() > MAX_SOURCE_BYTES || source.trim().is_empty() {
        return source;
    }
    let Ok(mut cache) = CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock() else { return source };
    if let Some(text) = cache.get(source).copied() {
        return text;
    }
    if cache.len() >= MAX_ENTRIES {
        return source;
    }
    let trimmed_start = source.trim_start();
    let body = trimmed_start.trim_end();
    let leading = source.get(..source.len() - trimmed_start.len()).unwrap_or_default();
    let trailing = trimmed_start.get(body.len()..).unwrap_or_default();
    let mut text = String::new();
    text.push_str(leading);
    text.push('[');
    let mut placeholder = false;
    for ch in body.chars() {
        if ch == '{' {
            placeholder = true;
        }
        let accented = if !placeholder && ch.is_ascii_alphabetic() {
            let alphabet = if ch.is_ascii_uppercase() {
                "ÁḂÇĎÉḞĞĤÍĴĶĽḾÑÓṔɊŘŠŢÚṼŴẊÝŽ"
            } else {
                "áḃçďéḟğĥíĵķľḿñóṕɋřšţúṽŵẋýž"
            };
            alphabet.chars().nth(usize::from(ch.to_ascii_lowercase() as u8 - b'a')).unwrap_or(ch)
        } else {
            ch
        };
        text.push(accented);
        if ch == '}' {
            placeholder = false;
        }
    }
    let padding = (body.chars().count() * 2).div_ceil(5);
    text.extend(std::iter::repeat_n('·', padding));
    text.push(']');
    text.push_str(trailing);
    let text: &'static str = Box::leak(text.into_boxed_str());
    cache.insert(source.to_string(), text);
    text
}

pub fn tr<'a>(language: &str, source: &'a str) -> &'a str {
    if language == "qps" {
        return pseudo(source);
    }
    catalog(language).and_then(|c| c.get(&("", source))).copied().unwrap_or(source)
}

pub fn t(source: &str) -> &str {
    CURRENT.with(|l| tr(l.get(), source))
}

/// Menu headings can differ from command or object labels (e.g. Czech Kótování / Kóta).
/// Catalogs without a `menu` entry retain their ordinary translation.
pub fn tr_menu<'a>(language: &str, source: &'a str) -> &'a str {
    catalog(language).and_then(|c| c.get(&("menu", source))).copied().unwrap_or_else(|| tr(language, source))
}

pub fn menu(source: &str) -> &str {
    CURRENT.with(|l| tr_menu(l.get(), source))
}

/// Ribbon headings can differ from ordinary labels; catalogs without a context row fall back.
pub fn tr_ribbon<'a>(language: &str, source: &'a str) -> &'a str {
    catalog(language).and_then(|c| c.get(&("ribbon", source))).copied().unwrap_or_else(|| tr(language, source))
}

pub fn ribbon(source: &str) -> &str {
    CURRENT.with(|l| tr_ribbon(l.get(), source))
}

/// Named placeholders allow translations to reorder values. Inserted values are never looked up
/// or substituted again, so a user's name containing `{n}` is preserved verbatim.
pub fn format(source: &str, values: &[(&str, String)]) -> String {
    let text = t(source);
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((before, tail)) = rest.split_once('{') {
        result.push_str(before);
        let Some((key, after)) = tail.split_once('}') else {
            result.push('{');
            rest = tail;
            break;
        };
        if let Some((_, value)) = values.iter().find(|(name, _)| *name == key) {
            result.push_str(value);
        } else {
            result.push('{');
            result.push_str(key);
            result.push('}');
        }
        rest = after;
    }
    result.push_str(rest);
    result
}

pub fn prompt_message(prompt: &cadcraft_engine::Prompt) -> String {
    use cadcraft_engine::prompt::PromptArg;

    if prompt.raw {
        // Routine text has no template semantics; only a whole displayed key may be translated.
        let displayed = prompt.display();
        return CURRENT.with(|language| {
            catalog(language.get()).and_then(|c| c.get(&("", displayed.as_str()))).copied().unwrap_or(displayed.as_str()).to_string()
        });
    }

    match &prompt.template {
        Some(template) => {
            let values: Vec<_> = template
                .args
                .iter()
                .map(|(name, arg)| {
                    let value = match arg {
                        PromptArg::Text(text) => text.clone(),
                        PromptArg::Key(key) => t(key).to_string(),
                    };
                    (*name, value)
                })
                .collect();
            format(template.source, &values)
        }
        None => t(&prompt.message).to_string(),
    }
}

/// Ukrainian integer plurals: one (1, 21), few (2–4, 22–24), many (0, 5–20, 25…).
pub fn plural_uk(n: u64) -> usize {
    match (n % 10, n % 100) {
        (1, 11) | (2..=4, 12..=14) => 2,
        (1, _) => 0,
        (2..=4, _) => 1,
        _ => 2,
    }
}

/// Portuguese integer plurals (CLDR `pt`): one (0, 1), other (2 and up).
pub fn plural_pt(n: u64) -> usize {
    usize::from(n > 1)
}

/// Korean and Vietnamese use the same count form for every integer.
fn plural_single(_n: u64) -> usize {
    0
}

/// Arabic integer plurals (CLDR `ar`): zero (0), one (1), two (2), few (3–10, 103–110),
/// many (11–99) — the same rule pdfcraft uses, so the sibling apps agree.
pub fn plural_arabic(n: u64) -> usize {
    match (n, n % 100) {
        (0, _) => 0,
        (1, _) => 1,
        (2, _) => 2,
        (_, 3..=10) => 3,
        (_, 11..=99) => 4,
        _ => 5,
    }
}

fn plural_en(n: u64) -> usize {
    usize::from(n != 1)
}

/// Czech integer plurals: one (1), few (2–4), other (0 and 5 or more).
pub fn plural_cs(n: u64) -> usize {
    match n {
        1 => 0,
        2..=4 => 1,
        _ => 2,
    }
}

pub fn count(n: u64, one: &str, other: &str) -> String {
    let language = CURRENT.get();
    let source = catalog(language).and_then(|c| c.get(&("@plural", one))).copied();
    let plural = LANGUAGES.iter().find(|l| l.code == language).map_or(plural_en as fn(u64) -> usize, |l| l.plural);
    let form = source.and_then(|s| s.split('|').nth(plural(n))).unwrap_or(if n == 1 { one } else { other });
    format(form, &[("n", n.to_string())])
}

#[macro_export]
macro_rules! tl {
    ($source:literal) => {
        $crate::i18n::t($source)
    };
}

#[macro_export]
macro_rules! tf {
    ($source:literal $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::i18n::format($source, &[$((stringify!($name), ($value).to_string())),*])
    };
}

#[cfg(test)]
mod tests;
