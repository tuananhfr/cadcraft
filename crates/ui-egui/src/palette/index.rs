//! Shared command metadata and bounded, accent-insensitive command search.

use std::collections::BTreeMap;

use cadcraft_engine::cmd::registry::{CommandStatus, command_registry};

use crate::i18n::{fold::fold, tr};
use crate::icons::Icon;

pub struct Entry {
    pub id: &'static str,
    pub namespaced_id: String,
    pub label_en: &'static str,
    pub aliases: Vec<String>,
    pub shortcut: Option<String>,
    pub icon: Option<Icon>,
    pub status: CommandStatus,
}

pub struct Hit {
    pub entry: usize,
    pub score: u32,
    pub matched: MatchKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchKind {
    ExactId,
    ExactAlias,
    IdPrefix,
    AliasPrefix,
    LabelWordPrefix,
    LabelSubstring,
    Fuzzy,
}

pub struct SearchIndex {
    entries: Vec<Entry>,
    labels: Vec<Vec<String>>,
}

impl SearchIndex {
    pub fn build(language: &'static str, shortcuts: &BTreeMap<String, String>) -> Self {
        let mut entries: Vec<_> = command_registry()
            .into_iter()
            .map(|command| Entry {
                id: command.stable_id,
                namespaced_id: command.namespaced_id,
                label_en: command.label_key,
                aliases: command.aliases,
                shortcut: shortcuts.get(command.stable_id).filter(|keys| !keys.is_empty()).cloned(),
                icon: crate::toolbars::resolve(command.stable_id).icon,
                status: command.status,
            })
            .collect();
        for &(id, label, _, _) in crate::menus::UI_COMMANDS {
            if !label.is_empty() && !crate::menus::command_hidden(id) && !entries.iter().any(|entry| entry.id == id) {
                entries.push(Entry {
                    id,
                    namespaced_id: id.into(),
                    label_en: label,
                    aliases: Vec::new(),
                    shortcut: shortcuts.get(id).filter(|keys| !keys.is_empty()).cloned(),
                    icon: crate::toolbars::resolve(id).icon,
                    status: CommandStatus::Available,
                });
            }
        }
        // User aliases must resolve identically in search and on the command line.
        for (alias, id) in cadcraft_engine::cmd::file::pgp::aliases() {
            for entry in &mut entries {
                entry.aliases.retain(|held| !held.eq_ignore_ascii_case(&alias));
                if entry.id == id {
                    entry.aliases.push(alias.clone());
                }
            }
        }
        let labels = entries
            .iter()
            .map(|entry| {
                let mut labels: Vec<_> = [entry.label_en, tr("vi", entry.label_en), tr(language, entry.label_en)].into_iter().map(fold).collect();
                labels.sort();
                labels.dedup();
                labels
            })
            .collect();
        Self { entries, labels }
    }

    pub fn search(&self, query: &str, recent: &[String], favorites: &[String], limit: usize) -> Vec<Hit> {
        let query: String = query.chars().take(64).collect();
        let query = fold(query.trim());
        let mut hits: Vec<_> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(entry, command)| {
                let labels = self.labels.get(entry)?;
                let (matched, score) = matching(command, labels, &query)?;
                Some(Hit { entry, score, matched })
            })
            .collect();
        hits.sort_by_key(|hit| {
            let command = self.entry(hit);
            let favorite = command.is_some_and(|entry| favorites.iter().take(10_000).any(|id| same_id(entry, id)));
            let recent = command.and_then(|entry| recent.iter().take(8).position(|id| same_id(entry, id))).unwrap_or(8);
            let id = command.map_or("", |entry| entry.id);
            (hit.matched, !favorite, recent, id.len(), std::cmp::Reverse(hit.score), id)
        });
        hits.truncate(limit.min(50));
        hits
    }

    pub fn entry(&self, hit: &Hit) -> Option<&Entry> {
        self.entries.get(hit.entry)
    }

    pub(crate) fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter()
    }
}

fn same_id(entry: &Entry, id: &str) -> bool {
    entry.id.eq_ignore_ascii_case(id) || entry.namespaced_id.eq_ignore_ascii_case(id)
}

fn matching(entry: &Entry, labels: &[String], query: &str) -> Option<(MatchKind, u32)> {
    if query.is_empty() {
        return Some((MatchKind::LabelSubstring, 0));
    }
    if same_id(entry, query) {
        return Some((MatchKind::ExactId, 0));
    }
    if entry.aliases.iter().any(|alias| alias.eq_ignore_ascii_case(query)) {
        return Some((MatchKind::ExactAlias, 0));
    }
    if entry.id.starts_with(query) || entry.namespaced_id.starts_with(query) {
        return Some((MatchKind::IdPrefix, 0));
    }
    if entry.aliases.iter().any(|alias| alias.to_ascii_lowercase().starts_with(query)) {
        return Some((MatchKind::AliasPrefix, 0));
    }
    if labels.iter().any(|label| word_prefix(label, query)) {
        return Some((MatchKind::LabelWordPrefix, 0));
    }
    if entry.id.contains(query) || labels.iter().any(|label| label.contains(query)) {
        return Some((MatchKind::LabelSubstring, 0));
    }
    labels.iter().filter_map(|label| fuzzy_score(label, query)).max().map(|score| (MatchKind::Fuzzy, score))
}

fn word_prefix(label: &str, query: &str) -> bool {
    label.starts_with(query)
        || label.char_indices().any(|(at, ch)| !ch.is_alphanumeric() && label.get(at + ch.len_utf8()..).is_some_and(|tail| tail.starts_with(query)))
}

fn fuzzy_score(label: &str, query: &str) -> Option<u32> {
    let mut wanted = query.chars().peekable();
    let mut score = 0u32;
    let mut consecutive = false;
    let mut word_start = true;
    for ch in label.chars() {
        if wanted.peek() == Some(&ch) {
            wanted.next();
            score += 1 + u32::from(consecutive) * 4 + u32::from(word_start) * 8;
            consecutive = true;
            if wanted.peek().is_none() {
                return Some(score);
            }
        } else {
            consecutive = false;
        }
        word_start = !ch.is_alphanumeric();
    }
    None
}
