//! recover_bytes and `recover {data}` must agree. A separate test binary owns its io hooks, which are process-wide.

use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, base64_encode, set_io};
use serde_json::json;

#[test]
fn recover_bytes_matches_recover_data() {
    set_io(IoHooks {
        read: |bytes, name| cadcraft_io::read(bytes, name).map_err(|e| e.to_string()),
        write: |doc, name| cadcraft_io::write(doc, name).map_err(|e| e.to_string()),
        plot: None,
    });
    let mut source = Session::new();
    source.execute("line", &json!({"points": [[0, 0], [10, 0]]})).unwrap();
    let bytes = cadcraft_io::write(source.doc().unwrap(), "drawing.dxf").unwrap();
    let mut direct = Session::empty();
    let mut command = Session::empty();
    let a = cadcraft_engine::cmd::audit::recover_bytes(&mut direct, &bytes, "Drawing.dxf", Some("Recovered"), true);
    let b = command.execute("recover", &json!({"data": base64_encode(&bytes), "name": "Drawing.dxf", "title": "Recovered", "copy": true}));
    let (a, b) = match (a, b) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(a), Err(b)) => {
            assert_eq!(a.to_string(), b.to_string());
            return;
        }
        other => panic!("byte and data recovery disagreed: {other:?}"),
    };
    for key in ["index", "errors", "fixed", "issues", "message"] {
        assert_eq!(a.get(key), b.get(key));
    }
    let a = direct.state().unwrap();
    let b = command.state().unwrap();
    assert_eq!(a.title, b.title);
    assert_eq!(a.path, b.path);
    assert!(a.is_dirty() && b.is_dirty());
    assert_eq!(cadcraft_io::write(&a.doc, "copy.dxf").unwrap(), cadcraft_io::write(&b.doc, "copy.dxf").unwrap());
}
