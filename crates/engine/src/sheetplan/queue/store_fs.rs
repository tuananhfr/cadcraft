use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use super::store::{JobStore, validate_bytes, validate_commit, validate_delete, validate_name};
use super::types::{MAX_JOB_SHEETS, MAX_JOB_SOURCES, MAX_MANIFEST_BYTES, PrintJob};

pub struct DirJobStore {
    root: PathBuf,
}

impl DirJobStore {
    pub fn new(root: PathBuf) -> DirJobStore {
        Self { root }
    }

    pub fn settings_default() -> Option<DirJobStore> {
        crate::cmd::plotters::settings_root().map(|root| Self::new(root.join("jobs")))
    }

    pub fn temp_name(job_id: &str, file: &str) -> String {
        format!("{file}.{job_id}.part")
    }

    fn job_dir(&self, job_id: &str) -> Result<PathBuf, String> {
        validate_name(job_id)?;
        let dir = self.root.join(job_id);
        reject_symlink(&dir)?;
        Ok(dir)
    }

    fn sheets_dir(&self, job_id: &str) -> Result<PathBuf, String> {
        let dir = self.job_dir(job_id)?.join("sheets");
        reject_symlink(&dir)?;
        Ok(dir)
    }
}

fn reject_symlink(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err("job store paths must not be symbolic links".into()),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn read_bounded(path: &Path, cap: usize) -> Result<Option<Vec<u8>>, String> {
    reject_symlink(path)?;
    match fs::metadata(path) {
        Ok(metadata) if !metadata.is_file() => return Err("stored path is not a regular file".into()),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    }
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let limit = u64::try_from(cap).map_err(|error| error.to_string())?;
    if file.metadata().map_err(|error| error.to_string())?.len() > limit {
        return Err("stored file exceeds supported byte limit".into());
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() > cap {
        return Err("stored file exceeds supported byte limit".into());
    }
    Ok(Some(bytes))
}

fn write_atomic(path: &Path, job_id: &str, bytes: &[u8]) -> Result<(), String> {
    let file_name = path.file_name().and_then(|name| name.to_str()).ok_or_else(|| "invalid name".to_string())?;
    let temporary = path.with_file_name(DirJobStore::temp_name(job_id, file_name));
    // An exclusive temp file protects an in-flight publisher, including after a stale crash file.
    let mut file = OpenOptions::new().write(true).create_new(true).open(&temporary).map_err(|error| error.to_string())?;
    let result = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    let result = result.and_then(|()| fs::rename(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|error| error.to_string())
}

fn normalize(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().len() > 4096 {
        return Err("path exceeds supported length".into());
    }
    let absolute = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().map_err(|error| error.to_string())?.join(path) };
    let mut result = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => {
                result.push(other.as_os_str());
                if let Ok(canonical) = fs::canonicalize(&result) {
                    result = canonical;
                }
            }
        }
    }
    Ok(result)
}

fn check_source(destination: &Path, source: &str) -> Result<(), String> {
    let destination = normalize(destination)?;
    let source = normalize(Path::new(source))?;
    #[cfg(target_os = "windows")]
    let equal = destination.to_string_lossy().to_lowercase() == source.to_string_lossy().to_lowercase();
    #[cfg(not(target_os = "windows"))]
    let equal = destination == source;
    if equal { Err("OUTPUT_IS_SOURCE: output would overwrite the source drawing".into()) } else { Ok(()) }
}

impl JobStore for DirJobStore {
    fn list_jobs(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut jobs = Vec::new();
        for entry in entries.take(MAX_JOB_SHEETS).flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if validate_name(&name).is_ok()
                && entry.file_type().is_ok_and(|kind| kind.is_dir())
                && fs::symlink_metadata(entry.path().join("manifest.json")).is_ok_and(|metadata| metadata.is_file())
            {
                jobs.push(name);
            }
        }
        jobs.sort();
        jobs
    }

    fn read_manifest(&self, job_id: &str) -> Result<Option<String>, String> {
        read_bounded(&self.job_dir(job_id)?.join("manifest.json"), MAX_MANIFEST_BYTES)?
            .map(|bytes| String::from_utf8(bytes).map_err(|error| error.to_string()))
            .transpose()
    }

    fn commit(&mut self, job_id: &str, artifacts: &[(String, Vec<u8>)], manifest_json: &str) -> Result<(), String> {
        validate_commit(job_id, artifacts, manifest_json)?;
        let sheets = self.sheets_dir(job_id)?;
        fs::create_dir_all(&sheets).map_err(|error| error.to_string())?;
        for (name, bytes) in artifacts {
            write_atomic(&sheets.join(name), job_id, bytes)?;
        }
        write_atomic(&self.job_dir(job_id)?.join("manifest.json"), job_id, manifest_json.as_bytes())
    }

    fn read_artifact(&self, job_id: &str, name: &str) -> Result<Option<Vec<u8>>, String> {
        validate_name(name)?;
        read_bounded(&self.sheets_dir(job_id)?.join(name), cadcraft_doc::MAX_PDF_BYTES)
    }

    fn delete_artifacts(&mut self, job_id: &str, names: &[String]) -> Result<(), String> {
        validate_delete(job_id, names)?;
        let dir = self.sheets_dir(job_id)?;
        for name in names {
            match fs::remove_file(dir.join(name)) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok(())
    }

    fn deliver(&mut self, job: &PrintJob, file_name: &str, bytes: &[u8]) -> Result<String, String> {
        validate_name(&job.job_id)?;
        validate_name(file_name)?;
        validate_bytes(bytes)?;
        if job.sources.len() > MAX_JOB_SOURCES {
            return Err("job source count exceeds supported limits".into());
        }
        let dir = job.output.dir.as_deref().filter(|dir| !dir.is_empty()).ok_or_else(|| "output directory is required".to_string())?;
        let path = Path::new(dir).join(file_name);
        let temporary = Path::new(dir).join(Self::temp_name(&job.job_id, file_name));
        for source in &job.sources {
            if let Some(source) = &source.path {
                check_source(&path, source)?;
                check_source(&temporary, source)?;
            }
        }
        fs::create_dir_all(dir).map_err(|error| error.to_string())?;
        write_atomic(&path, &job.job_id, bytes)?;
        Ok(path.to_string_lossy().into_owned())
    }

    fn remove_job(&mut self, job_id: &str) -> Result<(), String> {
        match fs::remove_dir_all(self.job_dir(job_id)?) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}
