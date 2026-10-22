use std::collections::BTreeSet;
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::Value;

pub use super::registry::REGISTRY_SCHEMA;
use super::registry::{FrameRegistry, FrameTemplate, MAX_REGISTRY_BYTES, MAX_TEMPLATES, TemplateStatus};

pub fn load_registry(text: Option<&str>) -> (FrameRegistry, Option<String>) {
    match text {
        None => (FrameRegistry::builtin(), None),
        Some(text) => parse_registry(text, true).unwrap_or_else(|error| {
            (FrameRegistry::builtin(), Some(format!("Frame template registry could not be loaded; using builtins: {error}")))
        }),
    }
}

pub fn save_registry(reg: &FrameRegistry) -> String {
    let current = FrameRegistry { schema_version: REGISTRY_SCHEMA.into(), templates: reg.templates.clone() };
    serde_json::to_string(&current).unwrap_or_else(|_| r#"{"schema_version":"2","templates":[]}"#.into())
}

pub fn resolve<'a>(reg: &'a FrameRegistry, versioned_id: &str) -> Option<&'a FrameTemplate> {
    let (id, version) = match versioned_id.rsplit_once('@') {
        Some((id, version)) => (id, version.parse::<u32>().ok()?),
        None => (versioned_id, 1),
    };
    reg.templates.iter().take(MAX_TEMPLATES).find(|template| template.id == id && template.version == version)
}

pub(crate) fn parse_registry(text: &str, recover: bool) -> Result<(FrameRegistry, Option<String>), String> {
    if text.len() > MAX_REGISTRY_BYTES {
        return Err("registry import exceeds 2 MiB".into());
    }
    // Value parsing bounds nesting before serde buffers internally tagged geometry rules.
    let value: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let schema = value.get("schema_version").and_then(Value::as_str).ok_or_else(|| "schema_version must be a string".to_string())?;
    let legacy = matches!(schema, "1" | "1.0");
    if !legacy && schema != REGISTRY_SCHEMA {
        return Err(format!("unsupported registry schema_version {schema}"));
    }
    let items = value.get("templates").and_then(Value::as_array).ok_or_else(|| "templates must be an array".to_string())?;
    if !recover && items.len() > MAX_TEMPLATES {
        return Err(format!("registry import exceeds {MAX_TEMPLATES} templates"));
    }
    let mut notices = Vec::new();
    if legacy {
        notices.push("Frame template registry migrated to schema 2".to_string());
    }
    if items.len() > MAX_TEMPLATES {
        notices.push(format!("Frame template registry exceeds {MAX_TEMPLATES} templates; remaining entries were dropped"));
    }
    let mut templates = Vec::new();
    let mut seen = BTreeSet::new();
    let mut dropped = 0_usize;
    let mut repaired = 0_usize;
    for item in items.iter().take(MAX_TEMPLATES) {
        let parsed = FrameTemplate::deserialize(item).map_err(|error| error.to_string()).and_then(|template| {
            template.validate()?;
            Ok(template)
        });
        let mut template = match parsed {
            Ok(template) => template,
            Err(error) if !recover => return Err(error),
            Err(_) => {
                dropped += 1;
                continue;
            }
        };
        if !seen.insert((template.id.clone(), template.version)) {
            if !recover {
                return Err(format!("duplicate template {}", template.versioned_id()));
            }
            dropped += 1;
            continue;
        }
        if template.superseded_by.is_some() && template.status != TemplateStatus::Superseded {
            template.status = TemplateStatus::Superseded;
            repaired += 1;
        } else if template.superseded_by.is_none() && template.status == TemplateStatus::Superseded {
            // A missing successor must not silently reactivate an obsolete template.
            template.status = TemplateStatus::Draft;
            repaired += 1;
        }
        templates.push(template);
    }
    if dropped > 0 {
        notices.push(format!("Frame template registry dropped {dropped} invalid or duplicate entries"));
    }
    if repaired > 0 {
        notices.push(format!("Frame template registry repaired {repaired} inconsistent superseded entries"));
    }
    Ok((FrameRegistry { schema_version: REGISTRY_SCHEMA.into(), templates }, (!notices.is_empty()).then(|| notices.join("; "))))
}

pub trait RegistryStore: Send + Sync {
    fn read(&self) -> Result<Option<String>, String>;
    fn read_last_good(&self) -> Result<Option<String>, String>;
    fn write(&mut self, text: &str) -> Result<(), String>;
}

pub type RegistryStoreFactory = fn() -> Result<Box<dyn RegistryStore>, String>;
static STORE_FACTORY: OnceLock<RegistryStoreFactory> = OnceLock::new();

pub fn set_registry_store_factory(factory: RegistryStoreFactory) {
    let _ = STORE_FACTORY.set(factory);
}

pub(crate) fn open_host_registry() -> (FrameRegistry, Option<Box<dyn RegistryStore>>, Option<String>) {
    let Some(factory) = STORE_FACTORY.get() else { return (FrameRegistry::builtin(), None, None) };
    match factory() {
        Ok(store) => {
            let (registry, notice) = load_stored_registry(store.as_ref());
            (registry, Some(store), notice)
        }
        Err(error) => (FrameRegistry::builtin(), None, Some(format!("Frame template persistence is unavailable: {error}"))),
    }
}

pub fn load_stored_registry(store: &dyn RegistryStore) -> (FrameRegistry, Option<String>) {
    let error = match store.read() {
        Ok(Some(text)) => match parse_registry(&text, true) {
            Ok(loaded) => return loaded,
            Err(error) => Some(error),
        },
        Ok(None) => None,
        Err(error) => Some(error),
    };
    match store.read_last_good() {
        Ok(Some(text)) => {
            if let Ok((registry, notice)) = parse_registry(&text, true) {
                let mut message = "Frame template registry was damaged or missing; restored the last good copy".to_string();
                if let Some(notice) = notice {
                    message.push_str("; ");
                    message.push_str(&notice);
                }
                return (registry, Some(message));
            }
        }
        Ok(None) if error.is_none() => return (FrameRegistry::builtin(), None),
        _ => {}
    }
    let reason = error.unwrap_or_else(|| "no usable last-good copy".into());
    (FrameRegistry::builtin(), Some(format!("Frame template registry could not be restored; using builtins: {reason}")))
}

impl crate::Session {
    pub fn set_frame_registry_store(&mut self, store: Box<dyn RegistryStore>) {
        let (registry, notice) = load_stored_registry(store.as_ref());
        self.ubplot.registry = registry;
        self.ubplot.registry_store = Some(store);
        self.ubplot.registry_notice = None;
        if let Some(notice) = notice {
            self.echo(notice);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub struct DirRegistryStore {
    root: std::path::PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl DirRegistryStore {
    pub fn new(root: std::path::PathBuf) -> Self {
        Self { root }
    }

    pub fn settings_default() -> Result<Self, String> {
        crate::cmd::plotters::settings_root().map(Self::new)
    }

    fn read_file(&self, name: &str) -> Result<Option<String>, String> {
        use std::io::Read;

        let file = match std::fs::File::open(self.root.join(name)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        let mut text = String::new();
        file.take(MAX_REGISTRY_BYTES as u64 + 1).read_to_string(&mut text).map_err(|error| error.to_string())?;
        if text.len() > MAX_REGISTRY_BYTES {
            return Err("frame template registry file exceeds 2 MiB".into());
        }
        Ok(Some(text))
    }

    fn atomic_write(&self, name: &str, text: &str) -> Result<(), String> {
        use std::io::Write;
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
        std::fs::create_dir_all(&self.root).map_err(|error| error.to_string())?;
        let nonce = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let temp = self.root.join(format!("{name}.{}.{}.tmp", std::process::id(), nonce));
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|error| error.to_string())?;
        let result = file.write_all(text.as_bytes()).and_then(|()| file.sync_all());
        drop(file);
        let result = result.and_then(|()| std::fs::rename(&temp, self.root.join(name))).map_err(|error| error.to_string());
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl RegistryStore for DirRegistryStore {
    fn read(&self) -> Result<Option<String>, String> {
        self.read_file("frame-templates.json")
    }

    fn read_last_good(&self) -> Result<Option<String>, String> {
        self.read_file("frame-templates.json.last-good")
    }

    fn write(&mut self, text: &str) -> Result<(), String> {
        FrameRegistry::from_json(text)?;
        if let Ok(Some(previous)) = self.read()
            && FrameRegistry::from_json(&previous).is_ok()
        {
            self.atomic_write("frame-templates.json.last-good", &previous)?;
        }
        self.atomic_write("frame-templates.json", text)
    }
}
