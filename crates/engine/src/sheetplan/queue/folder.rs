use std::fs::{self, Metadata};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use cadcraft_doc::{Drawing, EntityKind, dynblock};

use super::{JobSource, MAX_JOB_SOURCES, SkippedFile, SourceLoader};
use crate::cmd::file::io;
use crate::hash::sha256_hex;
use crate::sheetplan::codes::issue;
use crate::sheetplan::{FrameRegistry, MAX_SCAN_FRAMES, ScanMode, ScanRequest, Severity, build_plan};

const MAX_DEPTH: usize = 32;
const MAX_SCAN_ENTRIES: usize = 50_000;
const MAX_MODEL_ENTITIES: usize = 1_000_000;
// Match the native decoder's Limits::DEFAULT without making engine depend on the DWG bridge.
const MAX_DWG_BYTES: u64 = 512 << 20;
const MAX_DXF_BYTES: u64 = 1 << 30;

pub struct FolderScan {
    pub files: Vec<String>,
    pub skipped: Vec<SkippedFile>,
}

impl FolderScan {
    fn skip(&mut self, path: &Path, reason: impl Into<String>) {
        if self.skipped.len() < MAX_JOB_SOURCES {
            self.skipped.push(SkippedFile { path: path.to_string_lossy().into_owned(), reason: reason.into() });
        } else if let Some(last) = self.skipped.last_mut() {
            last.reason = "scan_limit: further skipped entries omitted".into();
        }
    }
}

fn linked(metadata: &Metadata) -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(target_os = "windows"))]
    {
        metadata.file_type().is_symlink()
    }
}

pub(crate) fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.as_os_str().len() > 4096 {
        return Err("path exceeds 4096 bytes".into());
    }
    let path = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().map_err(|error| error.to_string())?.join(path) };
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    Ok(result)
}

fn path_key(path: &Path) -> (String, String) {
    let text = path.to_string_lossy().replace('\\', "/");
    (text.to_lowercase(), text)
}

fn excluded(path: &Path, exclude: &[PathBuf]) -> bool {
    exclude.iter().any(|excluded| {
        #[cfg(target_os = "windows")]
        {
            Path::new(&path_key(path).0).starts_with(Path::new(&path_key(excluded).0))
        }
        #[cfg(not(target_os = "windows"))]
        {
            path.starts_with(excluded)
        }
    })
}

fn open_source(path: &Path) -> Result<(fs::File, u64), String> {
    if path.as_os_str().len() > 4096 {
        return Err("read_error: path exceeds 4096 bytes".into());
    }
    for ancestor in path.ancestors() {
        let metadata = fs::symlink_metadata(ancestor).map_err(|error| format!("read_error: {error}"))?;
        if linked(&metadata) {
            return Err("symlink: symbolic links and junctions are not followed".into());
        }
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| format!("read_error: {error}"))?;
    if !metadata.is_file() {
        return Err("read_error: not a regular file".into());
    }
    let mut file = fs::File::open(path).map_err(|error| format!("read_error: {error}"))?;
    let size = file.metadata().map_err(|error| format!("read_error: {error}"))?.len();
    if size == 0 {
        return Err("empty: file has zero bytes".into());
    }
    let mut prefix = [0; 512];
    let count = file.read(&mut prefix).map_err(|error| format!("read_error: {error}"))?;
    let prefix = prefix.get(..count).ok_or("read_error: invalid header length")?;
    let dwg = prefix.starts_with(b"AC10");
    let mut lines =
        prefix.split(|byte| *byte == b'\n').take(2).map(|line| std::str::from_utf8(line).unwrap_or("").trim_start_matches('\u{feff}').trim());
    let dxf = (lines.next() == Some("0") && lines.next() == Some("SECTION")) || cadcraft_dxf::is_binary(prefix);
    if !dwg && !dxf {
        return Err("invalid_magic: expected DWG or DXF content".into());
    }
    let limit = if dwg { MAX_DWG_BYTES } else { MAX_DXF_BYTES };
    if size > limit {
        return Err("size_limit: drawing exceeds native decoder limits".into());
    }
    Ok((file, limit))
}

pub fn scan_folder(root: &Path, max_depth: usize, max_files: usize, exclude: &[PathBuf]) -> FolderScan {
    let mut scan = FolderScan { files: Vec::new(), skipped: Vec::new() };
    let root = match absolute(root) {
        Ok(root) => root,
        Err(error) => {
            scan.skip(root, error);
            return scan;
        }
    };
    let exclude: Vec<_> = exclude.iter().take(MAX_JOB_SOURCES).filter_map(|path| absolute(path).ok()).collect();
    let mut pending = vec![(root.clone(), 0_usize)];
    let mut remaining = MAX_SCAN_ENTRIES;
    while let Some((path, depth)) = pending.pop() {
        if excluded(&path, &exclude) {
            continue;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                scan.skip(&path, format!("read_error: {error}"));
                continue;
            }
        };
        if linked(&metadata) {
            scan.skip(&path, "symlink: symbolic links and junctions are not followed");
        } else if metadata.is_dir() {
            if depth > max_depth.min(MAX_DEPTH) {
                scan.skip(&path, "max_depth: directory depth limit exceeded");
                continue;
            }
            let entries = match fs::read_dir(&path) {
                Ok(entries) => entries,
                Err(error) => {
                    scan.skip(&path, format!("read_error: {error}"));
                    continue;
                }
            };
            let mut children = Vec::new();
            for entry in entries {
                if remaining == 0 {
                    scan.skip(&path, "scan_limit: directory entry limit exceeded");
                    break;
                }
                remaining = remaining.saturating_sub(1);
                match entry {
                    Ok(entry) => children.push(entry.path()),
                    Err(error) => scan.skip(&path, format!("read_error: {error}")),
                }
            }
            children.sort_by_cached_key(|path| path_key(path));
            pending.extend(children.into_iter().rev().map(|path| (path, depth.saturating_add(1))));
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("dwg") || extension.eq_ignore_ascii_case("dxf"))
        {
            match open_source(&path) {
                Ok(_) => match path.to_str() {
                    Some(path) => scan.files.push(path.into()),
                    None => scan.skip(&path, "read_error: path is not UTF-8"),
                },
                Err(error) => scan.skip(&path, error),
            }
        }
    }
    scan.files.sort_by_cached_key(|path| path_key(Path::new(path).strip_prefix(&root).unwrap_or(Path::new(path))));
    let limit = max_files.min(MAX_JOB_SOURCES);
    let overflow = scan.files.split_off(scan.files.len().min(limit));
    for path in overflow {
        scan.skip(Path::new(&path), "max_files: file count limit exceeded");
    }
    scan.skipped.sort_by_cached_key(|entry| path_key(Path::new(&entry.path)));
    scan
}

fn read_source(path: &str) -> Result<Vec<u8>, String> {
    use std::io::{Seek, SeekFrom};

    let (mut file, limit) = open_source(Path::new(path))?;
    file.seek(SeekFrom::Start(0)).map_err(|error| format!("read_error: {error}"))?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1)).read_to_end(&mut bytes).map_err(|error| format!("read_error: {error}"))?;
    if u64::try_from(bytes.len()).map_err(|error| error.to_string())? > limit {
        return Err("size_limit: drawing exceeds native decoder limits".into());
    }
    Ok(bytes)
}

fn decode(bytes: &[u8], path: &str) -> Result<Drawing, String> {
    let hooks = io().ok_or("file formats are not available in this build")?;
    std::panic::catch_unwind(|| (hooks.read)(bytes, path)).map_err(|_| "read_error: drawing decoder failed".to_string())?
}

pub fn folder_source(path: &str, registry: &FrameRegistry, layouts: bool) -> Result<JobSource, String> {
    let bytes = read_source(path)?;
    let hash = sha256_hex(&bytes);
    let drawing = decode(&bytes, path)?;
    drop(bytes);
    let mut handles = Vec::new();
    if drawing.model.len() > MAX_MODEL_ENTITIES {
        return Err("scan_limit: model entity limit exceeded".into());
    }
    for entity in drawing.model.iter() {
        if let EntityKind::Insert(insert) = &entity.kind {
            let name = dynblock::effective_name(&drawing, entity.handle).unwrap_or_else(|| insert.block.clone());
            if registry.find(&name).is_some_and(|template| template.id != "generic") {
                if handles.len() >= MAX_SCAN_FRAMES {
                    return Err("scan_limit: frame count limit exceeded".into());
                }
                handles.push(entity.handle.hex());
            }
        }
    }
    let mut plan = build_plan(&drawing, &ScanRequest { handles, ..Default::default() }, registry, &hash)?;
    if layouts {
        let layout_plan =
            build_plan(&drawing, &ScanRequest { mode: ScanMode::Layout, layouts: vec!["*".into()], ..Default::default() }, registry, &hash)?;
        plan.issues.extend(layout_plan.issues);
        for mut sheet in layout_plan.sheets {
            if plan.sheets.len() >= MAX_SCAN_FRAMES {
                return Err("scan_limit: frame and layout count limit exceeded".into());
            }
            sheet.order = u32::try_from(plan.sheets.len().saturating_add(1)).map_err(|error| error.to_string())?;
            sheet.sheet_id = format!("LAYOUT-{:03}", sheet.order);
            for issue in &mut sheet.issues {
                issue.sheet_id = Some(sheet.sheet_id.clone());
            }
            plan.sheets.push(sheet);
        }
    }
    if plan.sheets.is_empty() {
        plan.issues.push(issue(Severity::Warn, "NO_FRAMES_FOUND", "No registered plot frames were found in this drawing.", None));
    }
    Ok(JobSource { path: Some(path.into()), source_hash: hash, plan })
}

#[derive(Default)]
pub struct FolderLoader {
    current: Option<Arc<Drawing>>,
}

impl SourceLoader for FolderLoader {
    fn load(&mut self, _: u32, source: &JobSource) -> Result<Arc<Drawing>, String> {
        self.current = None;
        let path = source.path.as_deref().ok_or("SOURCE_UNAVAILABLE: the folder source has no path")?;
        let bytes = read_source(path).map_err(|error| {
            let code = if ["empty:", "invalid_magic:", "size_limit:"].iter().any(|prefix| error.starts_with(prefix)) {
                "SOURCE_CHANGED"
            } else {
                "SOURCE_UNAVAILABLE"
            };
            format!("{code}: {error}")
        })?;
        if sha256_hex(&bytes) != source.source_hash {
            return Err("SOURCE_CHANGED: the folder drawing differs from the queued source".into());
        }
        let drawing = Arc::new(decode(&bytes, path).map_err(|error| format!("SOURCE_UNAVAILABLE: {error}"))?);
        Ok(self.current.insert(drawing).clone())
    }
}
