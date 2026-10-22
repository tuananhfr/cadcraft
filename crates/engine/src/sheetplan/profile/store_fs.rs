use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::store::{decode, validate_id};
use super::{MAX_PROFILE_BYTES, PrintProfileStore, ProfileLevel};
use crate::hash::sha256_hex;

const MAX_DIRECTORY_ENTRIES: usize = 4096;
const MAX_DAMAGED_FILES: usize = 256;

pub struct DirPrintProfileStore {
    root: PathBuf,
}

impl DirPrintProfileStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn settings_default() -> Option<Self> {
        crate::cmd::plotters::settings_root().map(|root| Self::new(root.join("print-profiles")))
    }

    fn level_dir(&self, level: ProfileLevel) -> Result<PathBuf, String> {
        let name = match level {
            ProfileLevel::Platform => return Err("platform print settings are built into the application".into()),
            ProfileLevel::Company => "company",
            ProfileLevel::Project => "project",
            ProfileLevel::SheetSet => "sheet_set",
            ProfileLevel::Drawing => "drawing",
            ProfileLevel::User => "user",
        };
        reject_symlink(&self.root)?;
        let dir = self.root.join(name);
        reject_symlink(&dir)?;
        Ok(dir)
    }

    fn file(&self, level: ProfileLevel, id: &str, suffix: &str) -> Result<PathBuf, String> {
        validate_id(level, id)?;
        let name = disk_id(id);
        Ok(self.level_dir(level)?.join(format!("{name}{suffix}")))
    }
}

fn disk_id(id: &str) -> String {
    match id.strip_prefix("name:") {
        Some(hash) => format!("name--{hash}"),
        None => id.into(),
    }
}

fn reject_symlink(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err("print profile store paths must not be symbolic links".into()),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, String> {
    reject_symlink(path)?;
    match fs::metadata(path) {
        Ok(metadata) if !metadata.is_file() => return Err("print profile path is not a regular file".into()),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    }
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let limit = u64::try_from(MAX_PROFILE_BYTES).map_err(|error| error.to_string())?;
    if file.metadata().map_err(|error| error.to_string())?.len() > limit {
        return Err("print profile exceeds 256 KiB".into());
    }
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() > MAX_PROFILE_BYTES {
        return Err("print profile exceeds 256 KiB".into());
    }
    Ok(Some(bytes))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    reject_symlink(path)?;
    let name = path.file_name().and_then(|name| name.to_str()).ok_or_else(|| "invalid print profile file name".to_string())?;
    let temporary = path.with_file_name(format!("{name}.part"));
    // Exclusive creation refuses an in-flight writer or a stale temp file after a crash.
    let mut file = OpenOptions::new().write(true).create_new(true).open(&temporary).map_err(|error| error.to_string())?;
    let result = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    let result = result.and_then(|()| fs::rename(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|error| error.to_string())
}

impl PrintProfileStore for DirPrintProfileStore {
    fn read(&self, level: ProfileLevel, id: &str) -> Result<Option<Vec<u8>>, String> {
        read_bounded(&self.file(level, id, ".json")?)
    }

    fn write(&mut self, level: ProfileLevel, id: &str, json: &str) -> Result<(), String> {
        decode(json.as_bytes(), level, id)?;
        let path = self.file(level, id, ".json")?;
        let last_good = self.file(level, id, ".last-good.json")?;
        let old = self.read(level, id)?;
        fs::create_dir_all(self.level_dir(level)?).map_err(|error| error.to_string())?;
        if let Some(raw) = old {
            if decode(&raw, level, id).is_ok() {
                write_atomic(&last_good, &raw)?;
            } else {
                self.keep_damaged(level, id, &raw)?;
                if self.read_last_good(level, id).is_none() {
                    write_atomic(&last_good, json.as_bytes())?;
                }
            }
        } else if self.read_last_good(level, id).is_none() {
            write_atomic(&last_good, json.as_bytes())?;
        }
        write_atomic(&path, json.as_bytes())
    }

    fn read_last_good(&self, level: ProfileLevel, id: &str) -> Option<String> {
        let raw = read_bounded(&self.file(level, id, ".last-good.json").ok()?).ok()??;
        decode(&raw, level, id).ok()?;
        String::from_utf8(raw).ok()
    }

    fn keep_damaged(&mut self, level: ProfileLevel, id: &str, raw: &[u8]) -> Result<(), String> {
        validate_id(level, id)?;
        if raw.len() > MAX_PROFILE_BYTES {
            return Err("damaged print profile exceeds 256 KiB".into());
        }
        let dir = self.level_dir(level)?.join("damaged");
        reject_symlink(&dir)?;
        fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
        let name = disk_id(id);
        let hash = sha256_hex(raw);
        let path = dir.join(format!("{name}-{hash}.json"));
        if let Some(existing) = read_bounded(&path)? {
            return if existing == raw { Ok(()) } else { Err("damaged print profile archive collision".into()) };
        }
        if fs::read_dir(&dir).map_err(|error| error.to_string())?.take(MAX_DAMAGED_FILES).count() >= MAX_DAMAGED_FILES {
            return Err("damaged print profile count limit reached".into());
        }
        write_atomic(&path, raw)
    }

    fn list(&self, level: ProfileLevel) -> Vec<String> {
        let Ok(dir) = self.level_dir(level) else { return Vec::new() };
        let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
        let mut ids = Vec::new();
        for entry in entries.take(MAX_DIRECTORY_ENTRIES).flatten() {
            if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                continue;
            }
            let Ok(name) = entry.file_name().into_string() else { continue };
            let Some(id) = name.strip_suffix(".json") else { continue };
            let id = if level == ProfileLevel::Drawing {
                match id.strip_prefix("name--") {
                    Some(hash) => format!("name:{hash}"),
                    None => id.into(),
                }
            } else {
                id.into()
            };
            if validate_id(level, &id).is_ok() {
                ids.push(id);
            }
        }
        ids.sort();
        ids
    }
}
