//! AutoLISP commands: APPLOAD loads a `.lsp` file; `lisp` evaluates LISP for programmatic
//! callers (scripts, the control channel, MCP, the CLI).

use serde_json::{Value, json};

use super::{bad, str_param};
use crate::lisp::value::{Value as Lv, to_text};
use crate::prompt::PromptArg;
use crate::{Accept, CommandSpec, EngineError, Input, Interactive, Prompt, Result, Session, Step};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("lisptrust", "LISP file approvals", run_lisptrust)
            .params("{action?: list|approve|revoke (default list), sha256?: source digest} → {approved}")
            .noundo(),
        #[cfg(not(target_arch = "wasm32"))]
        CommandSpec::new("lispsandbox", "LISP write folders", run_lispsandbox)
            .params("{action?: list|add|remove (default list), path?: existing directory} → {roots}")
            .noundo(),
        CommandSpec::new("appload", "Load Application...", run_appload)
            .menu(&["Tools", "Load Application..."])
            .alias(&["ap"])
            .params("{path: .lsp file} | {name, data: base64 | text} → {value?, error?}")
            .noundo()
            .interactive(|_| Ok(Box::new(ApploadM::default()))),
        CommandSpec::new("lisp", "Evaluate LISP", run_lisp)
            .params(
                "{code: LISP source, inputs?: [answers to get… calls: numbers, strings, [x,y] points]} → {value (printed as prin1), output: lines printed, error?}",
            )
            .noundo(),
    ]
}

fn run_lisptrust(s: &mut Session, p: &Value) -> Result<Value> {
    use crate::lisp::trust;

    match str_param(p, "action").unwrap_or("list") {
        "list" => {}
        action @ ("approve" | "revoke") => {
            let sha = str_param(p, "sha256").ok_or_else(|| bad("lisptrust", "`sha256` is required"))?;
            if sha.len() != 64 || !sha.bytes().all(|c| c.is_ascii_hexdigit()) {
                return Err(bad("lisptrust", "`sha256` must be 64 hexadecimal digits"));
            }
            if action == "approve" {
                if !trust::approve(sha) {
                    return Err(bad("lisptrust", "the LISP approval list is full or unavailable"));
                }
            } else {
                trust::revoke(sha);
            }
            s.touch();
        }
        _ => return Err(bad("lisptrust", "`action` must be list, approve or revoke")),
    }
    Ok(json!({"approved": trust::approved()}))
}

#[cfg(not(target_arch = "wasm32"))]
fn run_lispsandbox(_: &mut Session, p: &Value) -> Result<Value> {
    use crate::lisp::sandbox::{MAX_WRITE_ROOTS, same_path, set_write_roots, write_roots};

    let action = str_param(p, "action").unwrap_or("list");
    let mut roots = write_roots();
    match action {
        "list" => {}
        "add" | "remove" => {
            let path = str_param(p, "path").ok_or_else(|| bad("lispsandbox", "`path` is required"))?;
            if path.is_empty() || path.len() > 4096 {
                return Err(bad("lispsandbox", "`path` must contain 1 to 4096 bytes"));
            }
            let path = std::path::Path::new(path)
                .canonicalize()
                .ok()
                .filter(|path| path.is_dir())
                .ok_or_else(|| bad("lispsandbox", "`path` must be an existing directory"))?;
            if action == "add" {
                if !roots.iter().any(|root| same_path(root, &path)) {
                    if roots.len() >= MAX_WRITE_ROOTS {
                        return Err(bad("lispsandbox", format!("at most {MAX_WRITE_ROOTS} write folders are allowed")));
                    }
                    roots.push(path);
                }
            } else {
                roots.retain(|root| !same_path(root, &path));
            }
            set_write_roots(roots);
            roots = write_roots();
        }
        _ => return Err(bad("lispsandbox", "`action` must be list, add or remove")),
    }
    Ok(json!({"roots": roots}))
}

/// The text of a LISP file from `{path}` or `{name, data | text}`, and its name.
fn source(p: &Value) -> Result<(String, String)> {
    if let Some(text) = str_param(p, "text") {
        return Ok((str_param(p, "name").unwrap_or("untitled.lsp").to_string(), text.to_string()));
    }
    if let Some(data) = str_param(p, "data") {
        let bytes = super::file::base64_decode(data).ok_or_else(|| bad("appload", "invalid base64"))?;
        return Ok((str_param(p, "name").unwrap_or("untitled.lsp").to_string(), String::from_utf8_lossy(&bytes).into_owned()));
    }
    let path = str_param(p, "path").ok_or_else(|| bad("appload", "`path`, or `name` with `data` (base64) or `text`, is required"))?;
    let text = crate::lisp::builtins::read_text_file(path).ok_or_else(|| bad("appload", format!("{path}: can't read the file")))?;
    Ok((path.to_string(), text))
}

/// The result of a blocking LISP run as JSON, with the lines it printed.
fn report(s: &Session, from_log: usize, r: Option<std::result::Result<Lv, String>>) -> Value {
    let output: Vec<String> = s.log.get(from_log..).map(|l| l.to_vec()).unwrap_or_default();
    match r {
        Some(Ok(v)) => json!({ "value": to_text(&v, true), "output": output }),
        Some(Err(e)) => json!({ "error": e, "output": output }),
        None => json!({ "output": output }),
    }
}

fn run_appload(s: &mut Session, p: &Value) -> Result<Value> {
    let (name, text) = source(p)?;
    let from = s.log.len();
    let r = s.lisp_load(&name, &text, true)?;
    Ok(report(s, from, r))
}

/// A JSON input as a LISP value: numbers, strings, `[x, y(, z)]` points, null for Enter.
fn input_value(v: &Value) -> Lv {
    match v {
        Value::Array(a) if a.len() >= 2 && a.iter().all(Value::is_number) => Lv::point(&a.iter().filter_map(Value::as_f64).collect::<Vec<_>>()),
        other => crate::lisp::drawing_from_json(other),
    }
}

fn run_lisp(s: &mut Session, p: &Value) -> Result<Value> {
    let code = str_param(p, "code").ok_or_else(|| bad("lisp", "`code` is required"))?;
    let inputs = p.get("inputs").and_then(Value::as_array).map(|a| a.iter().map(input_value).collect()).unwrap_or_default();
    let from = s.log.len();
    let r = s.lisp_eval(code, false, true, inputs).map_err(|e| match e {
        EngineError::Other(m) => bad("lisp", m),
        e => e,
    })?;
    Ok(report(s, from, r))
}

/// APPLOAD keeps the reviewed bytes so a file edit cannot change what Yes authorizes.
#[derive(Default)]
struct ApploadM {
    pending: Option<(String, String, String)>,
}

impl Interactive for ApploadM {
    fn name(&self) -> &'static str {
        if self.pending.is_some() { "APPLOADTRUST" } else { "APPLOAD" }
    }
    fn prompt(&self, _: &Session) -> Prompt {
        let Some((name, _, sha)) = &self.pending else { return Prompt::new("Enter name of LISP file to load", Accept::TEXT) };
        Prompt::tf(
            "Load unreviewed LISP file {name} (sha256 {sha})?",
            vec![("name", PromptArg::Text(name.clone())), ("sha", PromptArg::Text(sha.get(..8).unwrap_or(sha).into()))],
            Accept::TEXT,
        )
        .kw(&["Yes", "No", "Always"])
        .default("No")
    }
    fn input(&mut self, s: &mut Session, i: Input) -> Result<Step> {
        if let Some((name, text, sha)) = &self.pending {
            let answer = match &i {
                Input::Text(answer) | Input::Keyword(answer) => answer.to_ascii_lowercase(),
                Input::Cancel | Input::Enter => "no".into(),
                _ => return Ok(Step::Continue),
            };
            match answer.as_str() {
                "y" | "yes" | "a" | "always" => {
                    // A routine may resume after the MCP call that started it has returned.
                    if s.lisp_running() {
                        return Err(EngineError::Other("Only the user can answer APPLOADTRUST".into()));
                    }
                    if matches!(answer.as_str(), "a" | "always") && !crate::lisp::trust::approve(sha) {
                        return Err(bad("appload", "the LISP approval list is full or unavailable"));
                    }
                    if let Err(error) = s.lisp_load_once(name, text, false) {
                        s.echo(error.to_string());
                    }
                }
                "n" | "no" | "" => s.echo(format!("{name} was not loaded")),
                _ => return Ok(Step::Continue),
            }
            return Ok(Step::Done);
        }
        if matches!(i, Input::Cancel | Input::Enter) {
            return Ok(Step::Cancel);
        }
        let Input::Text(t) = i else { return Ok(Step::Continue) };
        let path = t.trim().trim_matches('"').to_string();
        if path.is_empty() {
            return Ok(Step::Cancel);
        }
        match source(&json!({ "path": path })) {
            Ok((name, text)) => {
                let sha = crate::lisp::trust::source_sha(&text);
                if !crate::lisp::trust::is_approved(&sha) {
                    self.pending = Some((name, text, sha));
                    return Ok(Step::Continue);
                }
                if let Err(error) = s.lisp_load(&name, &text, false) {
                    s.echo(error.to_string());
                }
                Ok(Step::Done)
            }
            Err(EngineError::BadParams { msg, .. }) => {
                s.echo(msg);
                Ok(Step::Done)
            }
            Err(e) => Err(e),
        }
    }
}

impl Session {
    /// Dropped/browser files use the same review prompt as typed APPLOAD.
    pub fn lisp_load_interactive(&mut self, name: &str, text: &str) -> Result<()> {
        self.cancel();
        let sha = crate::lisp::trust::source_sha(text);
        if crate::lisp::trust::is_approved(&sha) {
            return self.lisp_load(name, text, false).map(|_| ());
        }
        self.start("appload")?;
        if let Some(run) = self.running.as_mut() {
            run.machine = crate::guard::Guarded::wrap(Box::new(ApploadM { pending: Some((name.to_string(), text.to_string(), sha)) }));
        }
        Ok(())
    }
}
