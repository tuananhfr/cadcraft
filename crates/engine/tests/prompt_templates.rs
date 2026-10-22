use std::path::Path;

use cadcraft_engine::prompt::{Accept, Prompt, PromptArg};

#[test]
fn no_prompt_message_is_built_with_format() {
    fn scan(path: &Path, remaining: &mut Vec<String>) {
        let mut entries: Vec<_> = std::fs::read_dir(path).unwrap().map(|entry| entry.unwrap().path()).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                scan(&path, remaining);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let source = std::fs::read_to_string(&path).unwrap();
                for (line, text) in source.lines().enumerate() {
                    if text.contains("Prompt::new(format!") {
                        remaining.push(format!("{}:{}", path.display(), line + 1));
                    }
                }
            }
        }
    }
    let mut remaining = Vec::new();
    scan(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut remaining);
    assert!(remaining.is_empty(), "format-built prompts: {}", remaining.join(", "));
}

#[test]
fn tf_fills_like_format_and_keeps_user_text_verbatim() {
    let p = Prompt::tf("Enter value for {tag}", vec![("tag", PromptArg::Text("{tag} X".into()))], Accept::TEXT);
    assert_eq!(p.message, "Enter value for {tag} X");
    assert_eq!(p.template.as_ref().map(|t| t.source), Some("Enter value for {tag}"));
    assert_eq!(serde_json::to_value(&p).unwrap().get("template"), None);

    let p = Prompt::tf(
        "{title} {message} {value} {missing}",
        vec![
            ("title", PromptArg::Text("** STRETCH **".into())),
            ("message", PromptArg::Key("Specify stretch point")),
            ("value", PromptArg::Text(format!("{:.4}", 1.25))),
        ],
        Accept::POINT,
    );
    assert_eq!(p.message, format!("** STRETCH ** Specify stretch point {:.4} {{missing}}", 1.25));
    assert_eq!(Prompt::new("Plain prompt", Accept::TEXT).template, None);
    assert_eq!(cadcraft_engine::prompt::fill("before {open", &[]), "before {open");
}
