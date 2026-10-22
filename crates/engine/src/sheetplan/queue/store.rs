use std::collections::BTreeMap;

use super::types::{MAX_JOB_SHEETS, MAX_MANIFEST_BYTES, PrintJob};

pub trait JobStore {
    fn list_jobs(&self) -> Vec<String>;
    fn read_manifest(&self, job_id: &str) -> Result<Option<String>, String>;
    /// Artifacts precede the manifest so a committed manifest never names missing data.
    fn commit(&mut self, job_id: &str, artifacts: &[(String, Vec<u8>)], manifest_json: &str) -> Result<(), String>;
    fn read_artifact(&self, job_id: &str, name: &str) -> Result<Option<Vec<u8>>, String>;
    fn delete_artifacts(&mut self, job_id: &str, names: &[String]) -> Result<(), String>;
    fn deliver(&mut self, job: &PrintJob, file_name: &str, bytes: &[u8]) -> Result<String, String>;
    fn remove_job(&mut self, job_id: &str) -> Result<(), String>;
}

#[derive(Clone, Debug, Default)]
struct StoredJob {
    manifest: Option<String>,
    artifacts: BTreeMap<String, Vec<u8>>,
}

#[derive(Clone, Debug, Default)]
pub struct MemoryJobStore {
    jobs: BTreeMap<String, StoredJob>,
    deliveries: BTreeMap<String, Vec<u8>>,
    pub fail_writes: bool,
    pub fail_after: Option<usize>,
}

pub fn artifact_name(order: u32, source_index: u32) -> String {
    format!("s{source_index:05}-{order:04}.pdf")
}

/// Validate a delivered basename without changing the user's Unicode spelling.
pub fn validate_output_file_name(name: &str) -> Result<(), String> {
    let reason = if name.is_empty() {
        Some("name is empty")
    } else if name.len() > 255 {
        Some("name exceeds 255 UTF-8 bytes")
    } else if name.split(['/', '\\']).any(|component| component == "..") {
        Some("parent directory components are forbidden")
    } else if name.contains(['/', '\\']) {
        Some("path separators are forbidden")
    } else if name.chars().any(char::is_control) {
        Some("control characters are forbidden")
    } else if name.ends_with([' ', '.']) {
        Some("name ends in a space or dot")
    } else if name.contains(['<', '>', ':', '"', '|', '?', '*']) {
        Some("Windows-forbidden characters are forbidden")
    } else {
        let stem = name.split('.').next().unwrap_or("").trim_end_matches(' ').to_ascii_uppercase();
        let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ["COM", "LPT"]
                .iter()
                .any(|prefix| stem.strip_prefix(prefix).is_some_and(|suffix| matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")));
        if reserved { Some("Windows reserved stem") } else { None }
    };
    if let Some(reason) = reason { Err(format!("invalid output file name: {reason}")) } else { Ok(()) }
}

pub(crate) fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 255
        || matches!(name, "." | "..")
        || !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return Err("invalid name".into());
    }
    Ok(())
}

pub(crate) fn validate_commit(job_id: &str, artifacts: &[(String, Vec<u8>)], manifest: &str) -> Result<(), String> {
    validate_name(job_id)?;
    if artifacts.len() > MAX_JOB_SHEETS || manifest.len() > MAX_MANIFEST_BYTES {
        return Err("job commit exceeds supported limits".into());
    }
    for (name, bytes) in artifacts {
        validate_name(name)?;
        validate_bytes(bytes)?;
    }
    Ok(())
}

pub(crate) fn validate_bytes(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > cadcraft_doc::MAX_PDF_BYTES { Err("artifact exceeds PDF byte limit".into()) } else { Ok(()) }
}

pub(crate) fn validate_delete(job_id: &str, names: &[String]) -> Result<(), String> {
    validate_name(job_id)?;
    if names.len() > MAX_JOB_SHEETS {
        return Err("artifact count exceeds supported limits".into());
    }
    for name in names {
        validate_name(name)?;
    }
    Ok(())
}

impl MemoryJobStore {
    fn check_write(&self, completed: usize) -> Result<(), String> {
        if self.fail_writes || self.fail_after.is_some_and(|limit| completed >= limit) { Err("job store write failed".into()) } else { Ok(()) }
    }
}

impl JobStore for MemoryJobStore {
    fn list_jobs(&self) -> Vec<String> {
        self.jobs.iter().filter(|(_, job)| job.manifest.is_some()).map(|(id, _)| id.clone()).collect()
    }

    fn read_manifest(&self, job_id: &str) -> Result<Option<String>, String> {
        validate_name(job_id)?;
        Ok(self.jobs.get(job_id).and_then(|job| job.manifest.clone()))
    }

    fn commit(&mut self, job_id: &str, artifacts: &[(String, Vec<u8>)], manifest_json: &str) -> Result<(), String> {
        validate_commit(job_id, artifacts, manifest_json)?;
        let mut staged = BTreeMap::new();
        for (index, (name, bytes)) in artifacts.iter().enumerate() {
            self.check_write(index)?;
            staged.insert(name.clone(), bytes.clone());
        }
        self.check_write(artifacts.len())?;
        let job = self.jobs.entry(job_id.into()).or_default();
        job.artifacts.extend(staged);
        job.manifest = Some(manifest_json.into());
        Ok(())
    }

    fn read_artifact(&self, job_id: &str, name: &str) -> Result<Option<Vec<u8>>, String> {
        validate_name(job_id)?;
        validate_name(name)?;
        Ok(self.jobs.get(job_id).and_then(|job| job.artifacts.get(name).cloned()))
    }

    fn delete_artifacts(&mut self, job_id: &str, names: &[String]) -> Result<(), String> {
        validate_delete(job_id, names)?;
        self.check_write(0)?;
        if let Some(job) = self.jobs.get_mut(job_id) {
            for name in names {
                job.artifacts.remove(name);
            }
        }
        Ok(())
    }

    fn deliver(&mut self, job: &PrintJob, file_name: &str, bytes: &[u8]) -> Result<String, String> {
        validate_name(&job.job_id)?;
        validate_output_file_name(file_name)?;
        validate_bytes(bytes)?;
        self.check_write(0)?;
        self.deliveries.insert(file_name.into(), bytes.into());
        Ok(format!("download:{file_name}"))
    }

    fn remove_job(&mut self, job_id: &str) -> Result<(), String> {
        validate_name(job_id)?;
        self.check_write(0)?;
        self.jobs.remove(job_id);
        Ok(())
    }
}
