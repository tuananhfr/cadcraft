//! Runs the real binary: one request in on stdin, one JSON line out, exit code follows `ok`.

use std::io::Write;
use std::process::{Command, Stdio};

fn run(input: &str) -> (String, Option<i32>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ubot-core"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn ubot-core");
    child.stdin.take().expect("stdin").write_all(input.as_bytes()).expect("write stdin");
    let out = child.wait_with_output().expect("wait");
    (String::from_utf8(out.stdout).expect("utf8 stdout"), out.status.code())
}

#[test]
fn stdio_fill_exit_0() {
    let req = r#"{"schema":"ubot.req/1","op":"fill","request":{"unitScale":1.0,"layoutId":"0123456789abcdef","productId":"ubot-h13-7","angleDeg":0.0,
        "boundary":{"vertices":[[0,0,0],[6600,0,0],[6600,6600,0],[0,6600,0]]}}}"#;
    let (out, code) = run(req);
    assert_eq!(code, Some(0), "{out}");
    assert_eq!(out.matches('\n').count(), 1);
    assert!(out.ends_with('\n'));
    let v: serde_json::Value = serde_json::from_str(out.trim_end()).expect("json line");
    assert_eq!(v["ok"], serde_json::json!(true));
    assert_eq!(v["result"]["add"].as_array().map(Vec::len), Some(100));
}

#[test]
fn stdio_bad_input_exit_1() {
    let (out, code) = run("{broken");
    assert_eq!(code, Some(1));
    let v: serde_json::Value = serde_json::from_str(out.trim_end()).expect("json line");
    assert_eq!(v["error"]["code"], "BAD_JSON");
}
