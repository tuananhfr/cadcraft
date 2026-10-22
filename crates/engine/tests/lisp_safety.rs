#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, Once};

use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, set_io};
use cadcraft_engine::lisp::sandbox::{check_write, set_write_roots, write_roots};
use cadcraft_engine::lisp::value::{Value as LispValue, to_text};
use cadcraft_engine::sheetplan::{Severity, SheetPlan, ids};
use serde_json::{Value, json};

static SANDBOX_TEST: Mutex<()> = Mutex::new(());

struct Sandbox {
    _lock: MutexGuard<'static, ()>,
    previous: Vec<PathBuf>,
    base: PathBuf,
    root: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        let lock = SANDBOX_TEST.lock().unwrap_or_else(|error| error.into_inner());
        let base = std::env::temp_dir().join(format!("cadcraft-lisp-sandbox-{}-{name}", std::process::id()));
        std::fs::create_dir(&base).unwrap();
        let base = base.canonicalize().unwrap();
        let root = base.join("allowed");
        std::fs::create_dir(&root).unwrap();
        let previous = write_roots();
        set_write_roots(vec![root.clone()]);
        Self { _lock: lock, previous, base, root }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        set_write_roots(self.previous.clone());
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn setup() -> Session {
    static HOOKS: Once = Once::new();
    HOOKS.call_once(|| {
        set_io(IoHooks {
            read: |_, _| Err("no reader in this test".into()),
            write: |drawing, _| Ok(cadcraft_io::write_dxf(drawing).into_bytes()),
            plot: Some(|drawing, space, options| cadcraft_io::plotter::plot(drawing, space, options).map_err(|error| error.to_string())),
        });
        cadcraft_engine::cmd::ubplot::set_clock(|| 42);
    });
    Session::new()
}

fn quoted(path: &Path) -> String {
    literal(&path.to_string_lossy())
}

fn literal(text: &str) -> String {
    to_text(&LispValue::str(text), true)
}

fn eval(session: &mut Session, code: &str) -> Value {
    session.execute("lisp", &json!({"code": code})).unwrap()
}

fn refusal(path: &Path) -> String {
    format!("LISP write blocked: {} is outside the allowed folders (LISPSANDBOX)", path.display())
}

fn blocked_open(session: &mut Session, path: &Path, mode: &str) {
    let result = eval(session, &format!("(open {} \"{mode}\")", quoted(path)));
    assert!(result.get("error").is_none(), "{result}");
    assert_eq!(result["value"], "nil", "{result}");
    assert!(result["output"].as_array().unwrap().iter().any(|line| line.as_str() == Some(refusal(path).as_str())), "{result}");
}

fn scan(session: &mut Session) -> Value {
    let plan: SheetPlan = serde_json::from_value(
        session.execute("ubplot.scan", &json!({"mode": "window", "windows": [[[0, 0], [420, 297]]], "scale": "fit"})).unwrap(),
    )
    .unwrap();
    let approvals: Vec<_> = plan
        .sheets
        .iter()
        .flat_map(|sheet| {
            sheet
                .issues
                .iter()
                .filter(|issue| issue.severity == Severity::Warn)
                .map(|issue| json!({"sheetKey": ids::sheet_key(sheet), "code": issue.code}))
        })
        .chain(plan.issues.iter().filter(|issue| issue.severity == Severity::Warn).map(|issue| json!({"sheetKey": "*", "code": issue.code})))
        .collect();
    session.execute("ubplot.approve", &json!({"plan": plan, "approvals": approvals, "by": "user"})).unwrap()
}

#[test]
fn sandbox_allows_writes_inside_roots() {
    let sandbox = Sandbox::new("allowed");
    let mut session = setup();
    let path = sandbox.root.join("a.txt");
    for (mode, line) in [("w", "first"), ("a", "second")] {
        let code = format!(r#"(progn (setq f (open {} "{mode}")) (write-line "{line}" f) (close f))"#, quoted(&path));
        let result = eval(&mut session, &code);
        assert!(result.get("error").is_none(), "{result}");
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\nsecond\n");
    assert_eq!(check_write(&path.to_string_lossy()).unwrap(), path);
    assert_eq!(check_write(&sandbox.root.to_string_lossy()).unwrap(), sandbox.root);

    let outside = sandbox.base.join("read-only.txt");
    std::fs::write(&outside, "read outside roots\n").unwrap();
    let code = format!(r#"(progn (setq f (open {} "r")) (setq line (read-line f)) (close f) line)"#, quoted(&outside));
    assert_eq!(eval(&mut session, &code)["value"], literal("read outside roots"));

    let revoked = sandbox.root.join("revoked.txt");
    let result = eval(&mut session, &format!(r#"(setq f (open {} "w"))"#, quoted(&revoked)));
    assert!(result.get("error").is_none(), "{result}");
    set_write_roots(Vec::new());
    let result = eval(&mut session, r#"(progn (write-line "blocked at close" f) (close f))"#);
    assert!(result["output"].as_array().unwrap().iter().any(|line| line.as_str() == Some(refusal(&revoked).as_str())), "{result}");
    assert!(!revoked.exists());
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path, _: bool) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink(target: &Path, link: &Path, directory: bool) -> std::io::Result<()> {
    if directory { std::os::windows::fs::symlink_dir(target, link) } else { std::os::windows::fs::symlink_file(target, link) }
}

#[test]
fn sandbox_blocks_traversal_and_symlink_escape() {
    let sandbox = Sandbox::new("escapes");
    let mut session = setup();
    let traversal = sandbox.root.join("../x.txt");
    let missing_parent = sandbox.root.join("missing/a.txt");
    let prefix_sibling = sandbox.base.join("allowed-extra");
    std::fs::create_dir(&prefix_sibling).unwrap();
    for path in [&traversal, &missing_parent, &prefix_sibling.join("a.txt")] {
        for mode in ["w", "a"] {
            blocked_open(&mut session, path, mode);
            assert!(!path.exists());
        }
    }

    #[cfg(windows)]
    assert!(check_write(&sandbox.root.join("case.txt").to_string_lossy().to_uppercase()).is_ok());
    let outside = sandbox.base.join("outside");
    std::fs::create_dir(&outside).unwrap();
    let link = sandbox.root.join("escape");
    if let Err(error) = symlink(&outside, &link, true) {
        eprintln!("symlink cases skipped: OS refused to create symlink: {error}");
        return;
    }
    blocked_open(&mut session, &link.join("new.txt"), "w");
    assert!(!outside.join("new.txt").exists());
    let victim = outside.join("existing.txt");
    std::fs::write(&victim, "unchanged").unwrap();
    let final_link = sandbox.root.join("final.txt");
    if let Err(error) = symlink(&victim, &final_link, false) {
        eprintln!("final symlink cases skipped: OS refused to create symlink: {error}");
        return;
    }
    blocked_open(&mut session, &final_link, "a");
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "unchanged");
    let broken = sandbox.root.join("broken.txt");
    symlink(&outside.join("absent.txt"), &broken, false).unwrap();
    blocked_open(&mut session, &broken, "w");
    assert!(!outside.join("absent.txt").exists());

    let alias = sandbox.base.join("root-alias");
    symlink(&sandbox.root, &alias, true).unwrap();
    set_write_roots(vec![alias.clone()]);
    assert_eq!(write_roots(), vec![sandbox.root.clone()]);
    assert_eq!(check_write(&alias.join("new.txt").to_string_lossy()).unwrap(), sandbox.root.join("new.txt"));

    let deferred = sandbox.root.join("deferred.txt");
    let result = eval(&mut session, &format!(r#"(setq f (open {} "w"))"#, quoted(&deferred)));
    assert!(result.get("error").is_none(), "{result}");
    symlink(&victim, &deferred, false).unwrap();
    let result = eval(&mut session, r#"(progn (write-line "must not append" f) (close f))"#);
    assert!(result["output"].as_array().unwrap().iter().any(|line| line.as_str() == Some(refusal(&deferred).as_str())), "{result}");
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "unchanged");

    let output = sandbox.root.join("export.dxf");
    let temporary = sandbox.root.join("export.dxf.cadcraft-tmp");
    symlink(&victim, &temporary, false).unwrap();
    session.cmdline("(getstring)").unwrap();
    let error = session.execute("export", &json!({"path": output})).unwrap_err().to_string();
    assert!(error.contains(&refusal(&temporary)), "{error}");
    assert!(!output.exists());
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "unchanged");
    session.input(cadcraft_engine::Input::Text("continue".into())).unwrap();
}

#[test]
fn sandbox_covers_commands_called_from_lisp() {
    let sandbox = Sandbox::new("commands");
    let mut session = setup();
    let outside = sandbox.base.join("outside.dxf");
    session.cmdline("(getstring)").unwrap();
    assert!(session.lisp_running());
    for command in ["export", "exportpdf", "plot", "publish", "ubplot.publish", "cad.file.export"] {
        let error = session.execute(command, &json!({"path": outside})).unwrap_err().to_string();
        assert!(error.contains(&refusal(&outside)), "{command}: {error}");
    }
    for key in ["path", "out", "dir"] {
        let params = json!({(key): outside});
        let error = session.execute("ubplot.scan", &params).unwrap_err().to_string();
        assert!(error.contains(&refusal(&outside)), "{key}: {error}");
    }
    assert!(!outside.exists());
    session.input(cadcraft_engine::Input::Text("continue".into())).unwrap();
    assert!(!session.lisp_running());
    session.execute("export", &json!({"path": outside})).unwrap();
    assert!(outside.exists());

    let plan = scan(&mut session);
    let pdf = sandbox.base.join("outside.pdf");
    let code = format!("(ubplot-publish {} {})", literal(&plan.to_string()), quoted(&pdf));
    let result = eval(&mut session, &code);
    assert!(result["error"].as_str().unwrap().contains(&refusal(&pdf)), "{result}");
    assert!(!pdf.exists());
    session.execute("ubplot.publish", &json!({"plan": plan, "path": pdf})).unwrap();
    assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF-"));
}

#[test]
fn sandbox_add_requires_existing_dir() {
    let sandbox = Sandbox::new("manage");
    let mut session = setup();
    let missing = sandbox.base.join("missing");
    let original = write_roots();
    let error = session.execute("lispsandbox", &json!({"action": "add", "path": missing})).unwrap_err().to_string();
    assert!(error.contains("existing directory"), "{error}");
    let file = sandbox.root.join("file.txt");
    std::fs::write(&file, "a file is not a root").unwrap();
    assert!(session.execute("lispsandbox", &json!({"action": "add", "path": file})).is_err());
    assert_eq!(write_roots(), original);

    let extra = sandbox.base.join("extra");
    std::fs::create_dir(&extra).unwrap();
    session.execute("lispsandbox", &json!({"action": "add", "path": extra})).unwrap();
    session.execute("lispsandbox", &json!({"action": "add", "path": extra.join(".")})).unwrap();
    assert_eq!(write_roots(), vec![sandbox.root.clone(), extra.clone()]);
    let listed = session.execute("lispsandbox", &json!({"action": "list"})).unwrap();
    assert_eq!(listed["roots"], json!([sandbox.root, extra]));
    session.execute("lispsandbox", &json!({"action": "remove", "path": extra})).unwrap();
    assert_eq!(write_roots(), original);
    assert!(check_write(&extra.join("denied.txt").to_string_lossy()).is_err());

    let many = (0..65).map(|index| {
        let path = sandbox.base.join(format!("root-{index}"));
        std::fs::create_dir(&path).unwrap();
        path
    });
    set_write_roots(many.collect());
    assert_eq!(write_roots().len(), 64);
    assert!(session.execute("lispsandbox", &json!({"action": "add", "path": extra})).is_err());
    let meta = cadcraft_engine::cmd::registry::command_registry().into_iter().find(|meta| meta.stable_id == "lispsandbox").unwrap();
    assert_eq!(meta.risk_class, cadcraft_engine::cmd::registry::RiskClass::File);
}

#[test]
fn ubplot_publish_path_outside_roots_is_refused() {
    let sandbox = Sandbox::new("publish-outside");
    let mut session = setup();
    let path = sandbox.base.join("outside.pdf");
    let result = eval(&mut session, &format!(r#"(ubplot-publish "{{}}" {})"#, quoted(&path)));
    assert_eq!(result["error"], refusal(&path));
    assert!(!path.exists());
}

#[test]
fn ubplot_publish_path_needs_pdf_extension() {
    let sandbox = Sandbox::new("publish-extension");
    let mut session = setup();
    let plan = scan(&mut session);
    for name in ["bad.txt", "bad.exe", "no-extension", "bad.pdf.exe"] {
        let path = sandbox.root.join(name);
        let code = format!("(ubplot-publish {} {})", literal(&plan.to_string()), quoted(&path));
        let result = eval(&mut session, &code);
        assert_eq!(result["error"], refusal(&path));
        assert!(!path.exists());
    }
    for name in ["good.pdf", "GOOD.PDF"] {
        let path = sandbox.root.join(name);
        let code = format!("(ubplot-publish {} {})", literal(&plan.to_string()), quoted(&path));
        let result = eval(&mut session, &code);
        assert!(result.get("error").is_none(), "{result}");
        assert!(std::fs::read(&path).unwrap().starts_with(b"%PDF-"));
    }
}

struct Review {
    sandbox: Sandbox,
    previous: Vec<String>,
}

impl Review {
    fn new(name: &str) -> Self {
        let sandbox = Sandbox::new(name);
        let previous = cadcraft_engine::lisp::trust::approved();
        cadcraft_engine::lisp::trust::set_approved(Vec::new());
        Self { sandbox, previous }
    }

    fn file(&self, text: &str) -> PathBuf {
        let path = self.sandbox.base.join("review.lsp");
        std::fs::write(&path, text).unwrap();
        path
    }
}

impl Drop for Review {
    fn drop(&mut self) {
        cadcraft_engine::lisp::trust::set_approved(self.previous.clone());
    }
}

#[test]
fn appload_json_refuses_unreviewed_file() {
    use cadcraft_engine::lisp::trust::source_sha;

    let review = Review::new("json-review");
    let text = "(setq json_review 101)";
    let path = review.file(text);
    let mut session = setup();
    let expected = format!("LISP_NOT_REVIEWED: review.lsp (sha256 {})", source_sha(text));
    for params in [
        json!({"name": "review.lsp", "text": text}),
        json!({"name": "review.lsp", "data": cadcraft_engine::cmd::file::base64_encode(text.as_bytes())}),
    ] {
        assert_eq!(session.execute("appload", &params).unwrap_err().to_string(), expected);
    }
    let error = session.execute("appload", &json!({"path": path})).unwrap_err().to_string();
    assert!(error.contains("LISP_NOT_REVIEWED"), "{error}");
    assert!(session.lisp_var("JSON_REVIEW").is_nil());
    assert!(cadcraft_engine::lisp::loaded_files(&session).is_empty());
    assert!(session.running.is_none());
    assert_eq!(eval(&mut session, "(setq direct_review 102)")["value"], "102");
}

#[test]
fn approved_file_loads() {
    use cadcraft_engine::lisp::trust::{approve, approved, is_approved, source_sha};

    let review = Review::new("approved-review");
    let text = "(setq approved_review 201)";
    let path = review.file(text);
    let sha = source_sha(text);
    let mut session = setup();
    assert!(approve(&sha));
    assert!(is_approved(&sha.to_ascii_uppercase()));
    for params in [
        json!({"path": path}),
        json!({"name": "review.lsp", "text": text}),
        json!({"name": "review.lsp", "data": cadcraft_engine::cmd::file::base64_encode(text.as_bytes())}),
    ] {
        assert!(session.execute("appload", &params).unwrap().get("error").is_none());
        assert_eq!(to_text(&session.lisp_var("APPROVED_REVIEW"), true), "201");
    }
    assert_eq!(session.execute("lisptrust", &json!({})).unwrap()["approved"], json!([sha]));
    session.execute("LISPTRUST", &json!({"action": "revoke", "sha256": sha})).unwrap();
    assert!(!is_approved(&sha));
    assert!(session.execute("appload", &json!({"path": path})).is_err());
    session.execute("cad.other.lisptrust", &json!({"action": "approve", "sha256": sha})).unwrap();
    assert!(is_approved(&sha));
    let before = approved();
    for params in [json!({"action": "unknown"}), json!({"action": "approve"}), json!({"action": "approve", "sha256": "bad"})] {
        assert!(session.execute("lisptrust", &params).is_err());
        assert_eq!(approved(), before);
    }
}

#[test]
fn interactive_appload_asks_and_always_approves() {
    use cadcraft_engine::lisp::trust::{is_approved, source_sha};

    let review = Review::new("interactive-review");
    let text = "(setq interactive_review 301)";
    let path = review.file(text);
    let sha = source_sha(text);
    let mut session = setup();
    session.cmdline("appload").unwrap();
    session.cmdline(&path.to_string_lossy()).unwrap();
    assert!(session.lisp_review_pending());
    assert_eq!(session.running.as_ref().unwrap().machine.name(), "APPLOADTRUST");
    assert_eq!(
        session.current_prompt().unwrap().display(),
        format!("Load unreviewed LISP file {} (sha256 {})? [Yes/No/Always] <No>:", path.display(), &sha[..8])
    );
    assert!(session.lisp_var("INTERACTIVE_REVIEW").is_nil());
    session.cmdline("A").unwrap();
    assert!(is_approved(&sha));
    assert!(!session.lisp_review_pending());
    assert_eq!(to_text(&session.lisp_var("INTERACTIVE_REVIEW"), true), "301");
    session.cmdline("appload").unwrap();
    session.cmdline(&path.to_string_lossy()).unwrap();
    assert!(session.running.is_none(), "approved content does not prompt again");

    for answer in ["N", "", "ESC", "Y"] {
        let text = format!("(setq answer_review \"{answer}\")");
        let path = review.file(&text);
        let mut session = setup();
        session.cmdline("appload").unwrap();
        session.cmdline(&path.to_string_lossy()).unwrap();
        assert!(session.lisp_review_pending());
        if answer == "ESC" {
            session.cancel();
        } else {
            session.cmdline(answer).unwrap();
        }
        assert!(session.running.is_none());
        assert!(!is_approved(&source_sha(&text)), "only Always persists approval");
        if answer == "Y" {
            assert_eq!(to_text(&session.lisp_var("ANSWER_REVIEW"), true), "\"Y\"");
            session.cmdline("appload").unwrap();
            session.cmdline(&path.to_string_lossy()).unwrap();
            assert!(session.lisp_review_pending(), "Yes authorizes one load");
            session.cancel();
        } else {
            assert!(session.lisp_var("ANSWER_REVIEW").is_nil());
            assert!(session.log.iter().any(|line| line == &format!("{} was not loaded", path.display())));
        }
    }
}

#[test]
fn edited_file_needs_review_again() {
    use cadcraft_engine::lisp::trust::{approve, is_approved, source_sha};

    let review = Review::new("edited-review");
    let original = "(setq edited_review 401)";
    let edited = "(setq edited_review 402)";
    let path = review.file(original);
    let mut session = setup();
    assert!(approve(&source_sha(original)));
    session.execute("appload", &json!({"path": path})).unwrap();
    std::fs::write(&path, edited).unwrap();
    assert!(!is_approved(&source_sha(edited)));
    assert!(session.execute("appload", &json!({"path": path})).unwrap_err().to_string().contains("LISP_NOT_REVIEWED"));
    session.cmdline("appload").unwrap();
    session.cmdline(&path.to_string_lossy()).unwrap();
    assert!(session.current_prompt().unwrap().message.contains("unreviewed"));
    std::fs::write(&path, "(setq edited_review 403)").unwrap();
    session.cmdline("Always").unwrap();
    assert_eq!(to_text(&session.lisp_var("EDITED_REVIEW"), true), "402", "review authorizes the displayed bytes");
    assert!(is_approved(&source_sha(edited)));
    assert!(!is_approved(&source_sha("(setq edited_review 403)")));
    session.cmdline("appload").unwrap();
    session.cmdline(&path.to_string_lossy()).unwrap();
    assert!(session.lisp_review_pending());
    session.cancel();
}

#[test]
fn load_inside_routine_refuses_unreviewed() {
    use cadcraft_engine::lisp::trust::{approve, revoke, source_sha};

    let review = Review::new("routine-review");
    let text = "(setq routine_review 501)";
    let path = review.file(text);
    let mut session = setup();
    let code = format!("(defun c:review-test () (load {} 'missing)) (c:review-test)", quoted(&path));
    let result = eval(&mut session, &code);
    assert_eq!(result["error"], format!("LISP_NOT_REVIEWED: {} (sha256 {})", path.display(), source_sha(text)));
    assert!(session.lisp_var("ROUTINE_REVIEW").is_nil());
    assert!(!session.lisp_review_pending());
    assert!(approve(&source_sha(text)));
    assert!(eval(&mut session, &code).get("error").is_none());
    assert_eq!(to_text(&session.lisp_var("ROUTINE_REVIEW"), true), "501");
    session.execute("appload", &json!({"name": "cached-review.lsp", "text": text})).unwrap();
    assert!(revoke(&source_sha(text)));
    assert!(eval(&mut session, "(load \"cached-review\")")["error"].as_str().unwrap().contains("LISP_NOT_REVIEWED"));
}

#[test]
fn approval_list_validates_and_limits_digests() {
    use cadcraft_engine::lisp::trust::{MAX_APPROVED, approve, approved, is_approved, revoke, set_approved, source_sha};

    let _review = Review::new("approval-validation");
    assert_eq!(source_sha("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    let sha = source_sha("approval-validation");
    set_approved(vec!["bad".into(), sha.to_ascii_uppercase(), sha.clone()]);
    assert_eq!(approved(), std::slice::from_ref(&sha));
    assert!(!approve(&"z".repeat(64)));
    assert!(!revoke("bad"));
    assert!(is_approved(&sha));
    set_approved((0..MAX_APPROVED + 1).map(|i| source_sha(&format!("approval-{i}"))).collect());
    assert_eq!(approved().len(), MAX_APPROVED);
    assert!(!approve(&source_sha("one-more-approval")));
    assert!(approve(&approved()[0]));
    assert!(revoke(&approved()[0]));
    assert!(approve(&sha));
}
