//! `ubot-core`: reads one request document from stdin, writes one response line to stdout.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::process::ExitCode;

use cadcraft_ubot::{MAX_INPUT_BYTES, handle_json_status};

const OVERSIZE: &str = r#"{"schema":"ubot.res/1","ok":false,"error":{"code":"LIMIT","message":"request is too large"}}"#;
const NOT_UTF8: &str = r#"{"schema":"ubot.res/1","ok":false,"error":{"code":"BAD_JSON","message":"request is not valid UTF-8"}}"#;
const READ_FAILED: &str = r#"{"schema":"ubot.res/1","ok":false,"error":{"code":"BAD_INPUT","message":"cannot read stdin"}}"#;

fn main() -> ExitCode {
    let mut buf = Vec::new();
    // One byte past the limit is enough to tell an oversize request from one that fits exactly.
    let read = std::io::stdin().lock().take(MAX_INPUT_BYTES as u64 + 1).read_to_end(&mut buf);
    let (response, ok) = match read {
        Err(_) => (READ_FAILED.to_string(), false),
        Ok(n) if n > MAX_INPUT_BYTES => (OVERSIZE.to_string(), false),
        Ok(_) => match String::from_utf8(buf) {
            Ok(text) => handle_json_status(&text),
            Err(_) => (NOT_UTF8.to_string(), false),
        },
    };
    let mut out = std::io::stdout().lock();
    let wrote = out.write_all(response.as_bytes()).and_then(|()| out.write_all(b"\n")).and_then(|()| out.flush());
    if ok && wrote.is_ok() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
