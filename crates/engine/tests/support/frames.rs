use std::sync::Arc;

use cadcraft_doc::{Attrib, Block, Common, Drawing, Entity, EntityKind, HAlign, Handle, Insert, Line, LwPolyline, RawTag, Space, Text, VAlign};
use cadcraft_engine::Session;
use cadcraft_engine::sheetplan::queue::{JobStore, RenderFailure};
use cadcraft_geom::{PolyVertex, Vec2, Vec3};

pub fn polyline(w: f64, h: f64) -> EntityKind {
    EntityKind::LwPolyline(LwPolyline {
        vertices: [Vec2::ZERO, Vec2::new(w, 0.0), Vec2::new(w, h), Vec2::new(0.0, h)].map(PolyVertex::new).into(),
        closed: true,
        const_width: 0.0,
        elevation: 0.0,
        plinegen: false,
    })
}

pub fn text(value: &str) -> Text {
    Text {
        insert: Vec3::new(10.0, 10.0, 0.0),
        align_pt: None,
        height: 2.0,
        value: value.into(),
        rotation: 0.0,
        width_factor: 1.0,
        oblique: 0.0,
        style: "Standard".into(),
        halign: HAlign::Left,
        valign: VAlign::Baseline,
    }
}

pub fn frame_block(d: &mut Drawing, name: &str, w: f64, h: f64) {
    let mut block = Block::new(name);
    let h1 = d.new_handle();
    let h2 = d.new_handle();
    block.entities.push(Entity::new(h1, polyline(w, h)));
    block.entities.push(Entity::new(h2, EntityKind::Text(text("FRAME"))));
    d.blocks.insert(name.into(), Arc::new(block));
}

pub fn title_only_block(d: &mut Drawing, name: &str) {
    let mut block = Block::new(name);
    let corners = [Vec2::ZERO, Vec2::new(180.0, 0.0), Vec2::new(180.0, 60.0), Vec2::new(0.0, 60.0)];
    for (a, b) in corners.into_iter().zip(corners.into_iter().cycle().skip(1)).take(4) {
        block.entities.push(Entity::new(d.new_handle(), EntityKind::Line(Line { a: a.to3(0.0), b: b.to3(0.0) })));
    }
    d.blocks.insert(name.into(), Arc::new(block));
}

pub fn place(s: &mut Session, name: &str, at: [f64; 2], scale: f64, rot_deg: f64, mirror_x: bool, attribs: &[(&str, &str)]) -> Handle {
    let attributes = attribs
        .iter()
        .map(|(tag, value)| Attrib {
            tag: (*tag).into(),
            text: text(value),
            invisible: false,
            constant: false,
            prompt: String::new(),
            props: Default::default(),
            multiline: None,
            modes: Default::default(),
        })
        .collect();
    s.doc_mut()
        .unwrap()
        .add(
            &Space::Model,
            Common::default(),
            EntityKind::Insert(Insert {
                block: name.into(),
                insert: Vec3::new(at[0], at[1], 0.0),
                scale: Vec3::new(if mirror_x { -scale } else { scale }, scale, 1.0),
                rotation: rot_deg.to_radians(),
                attribs: attributes,
                cols: 1,
                rows: 1,
                col_spacing: 0.0,
                row_spacing: 0.0,
            }),
        )
        .unwrap()
}

pub fn nest(d: &mut Drawing, outer: &str, inner: &str, at: [f64; 2], rot: f64) {
    let h = d.new_handle();
    let block = d.blocks.entry(outer.into()).or_insert_with(|| Arc::new(Block::new(outer)));
    Arc::make_mut(block).entities.push(Entity::new(
        h,
        EntityKind::Insert(Insert {
            block: inner.into(),
            insert: Vec3::new(at[0], at[1], 0.0),
            scale: Vec3::new(1.0, 1.0, 1.0),
            rotation: rot.to_radians(),
            attribs: Vec::new(),
            cols: 1,
            rows: 1,
            col_spacing: 0.0,
            row_spacing: 0.0,
        }),
    ));
}

pub fn anon_dynamic(d: &mut Drawing, effective: &str, anon: &str) {
    let mut block = (**d.block(effective).unwrap()).clone();
    block.name = anon.into();
    block.anonymous = true;
    let mut hidden = Entity::new(d.new_handle(), polyline(2000.0, 1500.0));
    hidden.common.visible = false;
    block.entities.push(hidden);
    d.blocks.insert(anon.into(), Arc::new(block));
    let target = d.new_handle();
    let foreign = Arc::make_mut(&mut d.foreign);
    foreign.targets.insert(target, ("BLOCK_RECORD".into(), effective.into()));
    foreign.records.insert(
        ("BLOCK_RECORD".into(), anon.to_ascii_uppercase()),
        cadcraft_doc::foreign::ForeignExtra {
            xdata: vec![RawTag { code: 1001, value: "AcDbBlockRepBTag".into() }, RawTag { code: 1005, value: target.hex() }],
            ..Default::default()
        },
    );
}

// Only the queue tests build whole batches; the other binaries sharing this module never call it.
#[allow(dead_code)]
pub fn frames_drawing(n: usize) -> Drawing {
    let mut session = Session::new();
    session.doc_mut().unwrap().header.set_i64("INSUNITS", 4);
    frame_block(session.doc_mut().unwrap(), "FRAME-A3", 420.0, 297.0);
    for index in 0..n {
        let sheet_id = format!("KT-{:02}", index + 1);
        place(&mut session, "FRAME-A3", [index as f64 * 500.0, 0.0], 1.0, 0.0, false, &[("SHEET_NO", &sheet_id)]);
    }
    session.doc().unwrap().clone()
}

#[allow(dead_code)]
pub fn install_budget_plot() {
    cadcraft_engine::cmd::file::set_budget_plot(|d, space, options, cancel, max_prims| {
        cadcraft_io::pdf::plot_with_budget(d, space, options, Some(cancel), Some(max_prims)).map_err(|error| error.to_string())
    });
    cadcraft_engine::cmd::file::set_merge_pdf(|parts, labels, title| {
        cadcraft_io::pdf_merge::merge_own(parts, labels, title).map_err(|error| error.to_string())
    });
}

#[allow(dead_code)]
pub struct FixedSheetRenderer(pub usize);

impl cadcraft_engine::sheetplan::queue::SheetRenderer for FixedSheetRenderer {
    fn render(
        &self,
        _: &Drawing,
        _: &cadcraft_engine::sheetplan::Sheet,
        _: &cadcraft_engine::sheetplan::queue::SheetBudget,
    ) -> Result<Vec<u8>, cadcraft_engine::sheetplan::queue::RenderFailure> {
        Ok(vec![0; self.0])
    }

    fn merge(&self, parts: &[&[u8]], labels: &[String], title: &str) -> Result<Vec<u8>, cadcraft_engine::sheetplan::queue::RenderFailure> {
        cadcraft_io::pdf_merge::merge_own(parts, labels, title).map_err(|error| RenderFailure::Error(error.to_string()))
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)]
pub struct WaitingSheetRenderer;

#[cfg(not(target_arch = "wasm32"))]
impl cadcraft_engine::sheetplan::queue::SheetRenderer for WaitingSheetRenderer {
    fn render(
        &self,
        _: &Drawing,
        _: &cadcraft_engine::sheetplan::Sheet,
        budget: &cadcraft_engine::sheetplan::queue::SheetBudget,
    ) -> Result<Vec<u8>, cadcraft_engine::sheetplan::queue::RenderFailure> {
        let started = std::time::Instant::now();
        while !budget.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            if started.elapsed() >= std::time::Duration::from_secs(1) {
                return Err(cadcraft_engine::sheetplan::queue::RenderFailure::Error("test watchdog did not cancel".into()));
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        Err(cadcraft_engine::sheetplan::queue::RenderFailure::Cancelled)
    }

    fn merge(&self, parts: &[&[u8]], labels: &[String], title: &str) -> Result<Vec<u8>, cadcraft_engine::sheetplan::queue::RenderFailure> {
        cadcraft_io::pdf_merge::merge_own(parts, labels, title).map_err(|error| RenderFailure::Error(error.to_string()))
    }
}

#[allow(dead_code)]
pub struct CancellingSheetRenderer;

impl cadcraft_engine::sheetplan::queue::SheetRenderer for CancellingSheetRenderer {
    fn render(
        &self,
        _: &Drawing,
        _: &cadcraft_engine::sheetplan::Sheet,
        budget: &cadcraft_engine::sheetplan::queue::SheetBudget,
    ) -> Result<Vec<u8>, cadcraft_engine::sheetplan::queue::RenderFailure> {
        budget.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(vec![0; 1024])
    }

    fn merge(&self, parts: &[&[u8]], labels: &[String], title: &str) -> Result<Vec<u8>, cadcraft_engine::sheetplan::queue::RenderFailure> {
        cadcraft_io::pdf_merge::merge_own(parts, labels, title).map_err(|error| RenderFailure::Error(error.to_string()))
    }
}

#[allow(dead_code)]
pub fn queue_job(count: usize) -> (cadcraft_engine::sheetplan::queue::PrintJob, cadcraft_engine::sheetplan::queue::SnapshotLoader) {
    use cadcraft_engine::sheetplan::queue::{DEFAULT_RETRY_LIMIT, JobSource, OutputSpec, PrintJob, SnapshotLoader};
    use cadcraft_engine::sheetplan::{FrameRegistry, ScanRequest, build_plan};

    let drawing = frames_drawing(count);
    let hash = cadcraft_engine::hash::sha256_hex(cadcraft_io::write_dxf(&drawing).as_bytes());
    let handles = drawing.model.iter().map(|entity| entity.handle.hex()).collect();
    let plan = build_plan(&drawing, &ScanRequest { handles, ..Default::default() }, &FrameRegistry::builtin(), &hash).unwrap();
    let source = JobSource { path: None, source_hash: hash.clone(), plan };
    let output = OutputSpec { stem: "x".into(), ..Default::default() };
    (PrintJob::new(vec![source], output, DEFAULT_RETRY_LIMIT).unwrap(), SnapshotLoader(Arc::new(drawing), hash))
}

#[allow(dead_code)]
pub fn two_piece_split(sheet: &mut cadcraft_engine::sheetplan::Sheet) {
    use cadcraft_engine::sheetplan::{SheetSource, SplitPlan};

    let outline = match &sheet.source {
        SheetSource::Block { outline, .. } | SheetSource::Window { outline, .. } => *outline,
        SheetSource::Layout { .. } => panic!("test needs a rectangular frame"),
    };
    let [a, b, c, d] = outline;
    let bottom = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
    let top = [(d[0] + c[0]) * 0.5, (d[1] + c[1]) * 0.5];
    sheet.split = Some(SplitPlan { pieces: vec![[a, bottom, top, d], [bottom, b, c, top]], overlap_mm: 0.0, approved: true });
}

#[allow(dead_code)]
pub struct CountingRenderer {
    pub counts: std::cell::RefCell<std::collections::BTreeMap<String, u32>>,
    pub order: std::cell::RefCell<Vec<String>>,
    pub flags: std::cell::RefCell<Vec<Arc<std::sync::atomic::AtomicBool>>>,
    pub failure: Option<(String, cadcraft_engine::sheetplan::queue::RenderFailure)>,
    pub fail_first_only: bool,
    pub fail_merge: bool,
    cached: std::collections::BTreeMap<usize, Vec<u8>>,
}

#[allow(dead_code)]
impl CountingRenderer {
    pub fn new() -> Self {
        let drawing = frames_drawing(1);
        let options = cadcraft_io::PdfOptions { label: Some("artifact".into()), ..Default::default() };
        let cached = [1, 2, 4]
            .into_iter()
            .map(|count| {
                let items = vec![(Space::Model, options.clone()); count];
                (count, cadcraft_io::pdf::pdf_items(&drawing, &items, &options).unwrap())
            })
            .collect();
        Self {
            counts: Default::default(),
            order: Default::default(),
            flags: Default::default(),
            failure: None,
            fail_first_only: false,
            fail_merge: false,
            cached,
        }
    }

    pub fn total(&self) -> u32 {
        self.counts.borrow().values().sum()
    }
}

impl Default for CountingRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl cadcraft_engine::sheetplan::queue::SheetRenderer for CountingRenderer {
    fn render(
        &self,
        _: &Drawing,
        sheet: &cadcraft_engine::sheetplan::Sheet,
        budget: &cadcraft_engine::sheetplan::queue::SheetBudget,
    ) -> Result<Vec<u8>, cadcraft_engine::sheetplan::queue::RenderFailure> {
        let key = cadcraft_engine::sheetplan::ids::sheet_key(sheet);
        let attempt = {
            let mut counts = self.counts.borrow_mut();
            let count = counts.entry(key).or_default();
            *count += 1;
            *count
        };
        self.order.borrow_mut().push(sheet.sheet_id.clone());
        self.flags.borrow_mut().push(budget.cancel.clone());
        if let Some((id, failure)) = &self.failure
            && id == &sheet.sheet_id
            && (!self.fail_first_only || attempt == 1)
        {
            return Err(failure.clone());
        }
        let pages = sheet.split.as_ref().filter(|split| split.approved).map_or(1, |split| split.pieces.len());
        self.cached.get(&pages).cloned().ok_or_else(|| RenderFailure::Error("missing cached test PDF".into()))
    }

    fn merge(&self, parts: &[&[u8]], labels: &[String], title: &str) -> Result<Vec<u8>, cadcraft_engine::sheetplan::queue::RenderFailure> {
        if self.fail_merge {
            return Err(RenderFailure::Error("synthetic merge failure".into()));
        }
        cadcraft_io::pdf_merge::merge_own(parts, labels, title).map_err(|error| RenderFailure::Error(error.to_string()))
    }
}

#[allow(dead_code)]
#[derive(Default)]
pub struct RecordingJobStore {
    pub inner: cadcraft_engine::sheetplan::queue::MemoryJobStore,
    pub commits: usize,
    pub deliveries: Vec<(String, Vec<u8>)>,
    pub delivery_attempts: usize,
    pub fail_delivery: bool,
    pub fail_commit_after_delivery: bool,
}

impl JobStore for RecordingJobStore {
    fn list_jobs(&self) -> Vec<String> {
        self.inner.list_jobs()
    }

    fn read_manifest(&self, job_id: &str) -> Result<Option<String>, String> {
        self.inner.read_manifest(job_id)
    }

    fn commit(&mut self, job_id: &str, artifacts: &[(String, Vec<u8>)], manifest_json: &str) -> Result<(), String> {
        self.commits += 1;
        self.inner.commit(job_id, artifacts, manifest_json)
    }

    fn read_artifact(&self, job_id: &str, name: &str) -> Result<Option<Vec<u8>>, String> {
        self.inner.read_artifact(job_id, name)
    }

    fn delete_artifacts(&mut self, job_id: &str, names: &[String]) -> Result<(), String> {
        self.inner.delete_artifacts(job_id, names)
    }

    fn deliver(&mut self, job: &cadcraft_engine::sheetplan::queue::PrintJob, file_name: &str, bytes: &[u8]) -> Result<String, String> {
        self.delivery_attempts += 1;
        if self.fail_delivery {
            return Err("synthetic delivery failure".into());
        }
        let destination = self.inner.deliver(job, file_name, bytes)?;
        self.deliveries.push((file_name.into(), bytes.into()));
        if self.fail_commit_after_delivery {
            self.inner.fail_writes = true;
        }
        Ok(destination)
    }

    fn remove_job(&mut self, job_id: &str) -> Result<(), String> {
        self.inner.remove_job(job_id)
    }
}
