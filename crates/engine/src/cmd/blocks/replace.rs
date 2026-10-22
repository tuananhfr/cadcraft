use cadcraft_doc::{EntityKind, Handle};
use serde_json::{Value, json};

use crate::cmd::machines::{SelOutcome, SelectPhase};
use crate::cmd::{CommandSpec, bad, str_param, targets};
use crate::{Accept, Input, Interactive, Prompt, Result, Session, Step};

const MAX_REFERENCES: usize = 100_000;
const MAX_ATTRIBUTES: usize = 1_000_000;

pub(super) fn spec() -> CommandSpec {
    CommandSpec::new("blockreplace", "Replace Block", run)
        .menu(&["Modify", "Block", "Replace Block"])
        .params("{handles?: [handle, ...], name: existing block name}")
        .interactive(|_| Ok(Box::new(ReplaceM::default())))
}

fn run(s: &mut Session, params: &Value) -> Result<Value> {
    let name = str_param(params, "name").filter(|name| !name.trim().is_empty()).ok_or_else(|| bad("blockreplace", "`name` is required"))?;
    if params.get("handles").and_then(Value::as_array).is_some_and(|handles| handles.len() > MAX_REFERENCES)
        || s.state()?.selection.len() > MAX_REFERENCES
    {
        return Err(bad("blockreplace", "too many block references"));
    }
    let mut handles = targets(s, params)?;
    handles.sort_unstable();
    handles.dedup();
    if handles.is_empty() {
        return Err(bad("blockreplace", "no block references selected"));
    }
    let drawing = s.doc()?;
    let block = drawing.block(name).cloned().ok_or_else(|| bad("blockreplace", format!("no block `{name}`")))?;
    if block.xref_path.is_some() {
        return Err(bad("blockreplace", "external references cannot be replaced"));
    }
    let defs: Vec<_> = block
        .entities
        .iter()
        .filter_map(|entity| match &entity.kind {
            EntityKind::AttDef(attribute) if !attribute.constant => Some(attribute),
            _ => None,
        })
        .take(MAX_ATTRIBUTES.saturating_add(1))
        .collect();
    if handles.len().checked_mul(defs.len()).is_none_or(|count| count > MAX_ATTRIBUTES) {
        return Err(bad("blockreplace", "too many replacement attributes"));
    }
    for handle in &handles {
        let insert = match drawing.entity(*handle).map(|entity| &entity.kind) {
            Some(EntityKind::Insert(insert)) => insert,
            _ => return Err(bad("blockreplace", format!("{} is not a block reference", handle.hex()))),
        };
        if drawing.block(&insert.block).is_some_and(|block| block.xref_path.is_some()) {
            return Err(bad("blockreplace", "external references cannot be replaced"));
        }
        if crate::cmd::curves::is_locked(s, *handle) {
            return Err(bad("blockreplace", "the reference is on a locked layer"));
        }
    }
    // Publish only a complete replacement so a rejected batch cannot leave partial edits.
    let mut replaced = drawing.clone();
    for handle in &handles {
        replaced.modify_entity(*handle, |entity| {
            if let EntityKind::Insert(insert) = &mut entity.kind {
                insert.block = block.name.clone();
                insert.attribs = super::attribs::synced(&block, &defs, insert);
            }
        })?;
    }
    *s.doc_mut()? = replaced;
    Ok(json!({"block": block.name, "replaced": handles.len()}))
}

#[derive(Default)]
struct ReplaceM {
    selection: SelectPhase,
}

impl Interactive for ReplaceM {
    fn name(&self) -> &'static str {
        "BLOCKREPLACE"
    }
    fn begin(&mut self, s: &mut Session) -> Result<Step> {
        if s.state()?.selection.len() > MAX_REFERENCES {
            return Err(bad("blockreplace", "too many block references"));
        }
        self.selection = SelectPhase::begin(s);
        Ok(Step::Continue)
    }
    fn prompt(&self, _s: &Session) -> Prompt {
        if self.selection.done {
            Prompt::new("Enter replacement block name", Accept::TEXT)
        } else if self.selection.removing {
            self.selection.prompt()
        } else {
            Prompt::new("Select block references to replace", Accept::SELECT)
        }
    }
    fn input(&mut self, s: &mut Session, input: Input) -> Result<Step> {
        if !self.selection.done {
            if let Input::Pick(handles) = &input
                && self.selection.picked.len().saturating_add(handles.len()) > MAX_REFERENCES
            {
                return Err(bad("blockreplace", "too many block references"));
            }
            return match self.selection.feed(s, &input)? {
                SelOutcome::Empty => Ok(Step::Done),
                SelOutcome::More | SelOutcome::Done(_) => Ok(Step::Continue),
            };
        }
        match input {
            Input::Text(name) | Input::Keyword(name) => {
                let handles: Vec<_> = self.selection.picked.iter().copied().map(Handle::hex).collect();
                run(s, &json!({"handles": handles, "name": name}))?;
                Ok(Step::Done)
            }
            Input::Enter => Ok(Step::Done),
            _ => Ok(Step::Continue),
        }
    }
}
