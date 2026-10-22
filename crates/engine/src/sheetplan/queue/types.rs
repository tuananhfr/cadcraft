use std::collections::BTreeSet;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

use crate::hash::sha256_hex;
use crate::sheetplan::codes::issue;
use crate::sheetplan::ids::{job_id, sheet_key};
use crate::sheetplan::{PreflightIssue, Severity, Sheet, SheetPlan, SheetStatus};

pub const MANIFEST_SCHEMA: &str = "1.0";
pub const MAX_JOB_SOURCES: usize = 5_000;
pub const MAX_JOB_SHEETS: usize = 20_000;
pub const MAX_RETRY_LIMIT: u8 = 5;
pub const DEFAULT_RETRY_LIMIT: u8 = 2;
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    #[default]
    Merged,
    PerSheet,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputSpec {
    pub mode: OutputMode,
    pub dir: Option<String>,
    pub stem: String,
    #[serde(deserialize_with = "deserialize_pages_per_file")]
    pub pages_per_file: u32,
}

impl Default for OutputSpec {
    fn default() -> Self {
        Self { mode: OutputMode::Merged, dir: None, stem: "plot".into(), pages_per_file: if cfg!(target_arch = "wasm32") { 64 } else { 256 } }
    }
}

fn valid_pages(pages: u32) -> bool {
    usize::try_from(pages).is_ok_and(|pages| (1..=cadcraft_doc::MAX_PLOT_PAGES).contains(&pages))
}

fn deserialize_pages_per_file<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    let pages = u32::deserialize(deserializer)?;
    if valid_pages(pages) { Ok(pages) } else { Err(serde::de::Error::custom("pages_per_file exceeds PDF page limits")) }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct JobSource {
    pub path: Option<String>,
    pub source_hash: String,
    pub plan: SheetPlan,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PrintJob {
    pub job_id: String,
    pub sources: Vec<JobSource>,
    pub output: OutputSpec,
    pub retry_limit: u8,
    #[serde(default)]
    pub printer: Option<String>,
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default)]
    pub profile_version: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JobStatus {
    Queued,
    Running,
    Cancelled,
    Done,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PrintArtifact {
    pub sheet_key: String,
    pub source_index: u32,
    pub sheet_id: String,
    pub order: u32,
    pub status: SheetStatus,
    pub attempts: u8,
    pub sha256: Option<String>,
    pub output_file: Option<String>,
    pub page_index: Option<u32>,
    #[serde(default)]
    pub issues: Vec<PreflightIssue>,
    pub finished_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputFile {
    pub name: String,
    pub pages: u32,
    pub sha256: String,
    pub delivered_to: String,
    pub delivered_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SourceEntry {
    pub path: Option<String>,
    pub source_hash: String,
    pub sheets: u32,
    #[serde(default)]
    pub issues: Vec<PreflightIssue>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkippedFile {
    pub path: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PublishManifest {
    pub schema_version: String,
    pub job_id: String,
    pub source_hash: String,
    pub sources: Vec<SourceEntry>,
    pub profile_id: Option<String>,
    pub profile_version: Option<String>,
    pub status: JobStatus,
    pub created_ms: u64,
    pub updated_ms: u64,
    pub sheets: Vec<PrintArtifact>,
    pub files: Vec<OutputFile>,
    #[serde(default)]
    pub issues: Vec<PreflightIssue>,
    #[serde(default)]
    pub skipped_files: Vec<SkippedFile>,
}

impl PrintJob {
    pub fn new(sources: Vec<JobSource>, output: OutputSpec, retry_limit: u8) -> Result<PrintJob, Vec<PreflightIssue>> {
        let invalid = |code, message| vec![issue(Severity::Block, code, message, None)];
        if sources.is_empty() || sources.len() > MAX_JOB_SOURCES {
            return Err(invalid("JOB_SOURCE_LIMIT", "Job source count is outside the supported limits."));
        }
        if !valid_pages(output.pages_per_file) {
            return Err(invalid("PAGES_PER_FILE_INVALID", "pages_per_file is outside the PDF page limits."));
        }
        if retry_limit > MAX_RETRY_LIMIT {
            return Err(invalid("RETRY_LIMIT_INVALID", "Retry limit exceeds the supported limit."));
        }
        let total = sources.iter().try_fold(0_usize, |total, source| total.checked_add(source.plan.sheets.len()));
        if total.is_none_or(|total| total > MAX_JOB_SHEETS) {
            return Err(invalid("JOB_SHEET_LIMIT", "Job sheet count exceeds the supported limit."));
        }
        let mut errors = Vec::new();
        let mut keys = BTreeSet::new();
        for (index, source) in (0_u32..).zip(&sources) {
            for sheet in &source.plan.sheets {
                if sheet.status == SheetStatus::Blocked || sheet.issues.iter().any(|issue| issue.severity == Severity::Block) {
                    errors.push(issue(Severity::Block, "PLAN_HAS_BLOCKED", "Blocked sheets cannot be queued.", Some(&sheet.sheet_id)));
                }
                if !keys.insert(job_sheet_key(index, sheet)) {
                    errors.push(issue(Severity::Block, "DUPLICATE_SHEET_KEY", "Sheet keys must be unique within a job.", Some(&sheet.sheet_id)));
                }
            }
            if source.plan.issues.iter().any(|issue| issue.severity == Severity::Block) {
                errors.push(issue(Severity::Block, "PLAN_HAS_BLOCKED", "The source plan has blocking issues.", None));
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        let settings = json!({"output": output, "retry_limit": retry_limit, "printer": null, "profile_version": null});
        Ok(Self {
            job_id: job_id(&combined_source_hash(&sources), &settings),
            sources,
            output,
            retry_limit,
            printer: None,
            profile_id: None,
            profile_version: None,
        })
    }
}

fn combined_source_hash(sources: &[JobSource]) -> String {
    if let [source] = sources {
        return source.source_hash.clone();
    }
    let mut hashes = String::new();
    for source in sources.iter().take(MAX_JOB_SOURCES) {
        hashes.push_str(&source.source_hash);
    }
    sha256_hex(hashes.as_bytes())
}

impl PublishManifest {
    pub fn new(job: &PrintJob, now_ms: u64) -> PublishManifest {
        let mut sources = Vec::new();
        let mut sheets = Vec::new();
        let mut issues = Vec::new();
        // Public fields can bypass PrintJob::new; keep manifest construction bounded too.
        for (source_index, source) in (0_u32..).zip(job.sources.iter().take(MAX_JOB_SOURCES)) {
            let remaining = MAX_JOB_SHEETS.saturating_sub(sheets.len());
            let count = source.plan.sheets.len().min(remaining);
            sources.push(SourceEntry {
                path: source.path.clone(),
                source_hash: source.source_hash.clone(),
                sheets: u32::try_from(count).unwrap_or(u32::MAX),
                issues: source.plan.issues.clone(),
            });
            for sheet in source.plan.sheets.iter().take(count) {
                sheets.push(PrintArtifact {
                    sheet_key: job_sheet_key(source_index, sheet),
                    source_index,
                    sheet_id: sheet.sheet_id.clone(),
                    order: sheet.order,
                    status: if sheet.status == SheetStatus::NeedsPreview { SheetStatus::Ready } else { sheet.status },
                    attempts: 0,
                    sha256: None,
                    output_file: None,
                    page_index: None,
                    issues: sheet.issues.clone(),
                    finished_ms: None,
                });
            }
            if count != source.plan.sheets.len() {
                issues.push(issue(Severity::Block, "JOB_SHEET_LIMIT", "Job sheet count exceeds the supported limit.", None));
            }
        }
        if job.sources.len() > MAX_JOB_SOURCES {
            issues.push(issue(Severity::Block, "JOB_SOURCE_LIMIT", "Job source count exceeds the supported limit.", None));
        }
        sheets.sort_by_key(|sheet| (sheet.source_index, sheet.order));
        Self {
            schema_version: MANIFEST_SCHEMA.into(),
            job_id: job.job_id.clone(),
            source_hash: combined_source_hash(&job.sources),
            sources,
            profile_id: job.profile_id.clone(),
            profile_version: job.profile_version.clone(),
            status: if issues.is_empty() { JobStatus::Queued } else { JobStatus::Failed },
            created_ms: now_ms,
            updated_ms: now_ms,
            sheets,
            files: Vec::new(),
            issues,
            skipped_files: Vec::new(),
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|error| json!({"error": error.to_string()}).to_string())
    }

    pub fn from_json(text: &str) -> Result<PublishManifest, String> {
        if text.len() > MAX_MANIFEST_BYTES {
            return Err("manifest exceeds 64 MiB".into());
        }
        let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
        for (key, cap) in [("sheets", MAX_JOB_SHEETS), ("sources", MAX_JOB_SOURCES), ("files", MAX_JOB_SHEETS), ("skipped_files", MAX_JOB_SOURCES)] {
            if value.get(key).and_then(Value::as_array).is_some_and(|items| items.len() > cap) {
                return Err(format!("manifest {key} exceeds {cap} entries"));
            }
        }
        crate::sheetplan::types::validate_input_size(&value)?;
        let manifest = Self::deserialize(&value).map_err(|error| error.to_string())?;
        if manifest.schema_version != MANIFEST_SCHEMA {
            return Err(format!("unsupported manifest schema_version {}", manifest.schema_version));
        }
        let mut keys = BTreeSet::new();
        for sheet in &manifest.sheets {
            let index = usize::try_from(sheet.source_index).map_err(|error| error.to_string())?;
            if sheet.order == 0 || manifest.sources.get(index).is_none() {
                return Err("manifest contains an invalid sheet order or source index".into());
            }
            let key = scoped_key(sheet.source_index, format!("{}@{}", sheet.sheet_id, sheet.order));
            if sheet.sheet_key != key || !keys.insert(&sheet.sheet_key) {
                return Err("manifest contains an invalid or duplicate sheet key".into());
            }
        }
        Ok(manifest)
    }
}

pub fn job_sheet_key(source_index: u32, sheet: &Sheet) -> String {
    scoped_key(source_index, sheet_key(sheet))
}

fn scoped_key(source_index: u32, key: String) -> String {
    if source_index == 0 { key } else { format!("{source_index}:{key}") }
}
