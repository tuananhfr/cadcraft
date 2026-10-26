//! Size of the KCS publish of a real 17-sheet drawing (`UBPLOT_KCS_DWG`, with its xref beside
//! it; skipped when unset). Before gapless linetypes plotted continuous and solid hatches were
//! filled as their loops, its sheets took 56.5 MB.

#[allow(dead_code)]
mod support;

use cadcraft_engine::Session;
use cadcraft_engine::cmd::file::{IoHooks, base64_decode, set_io};
use cadcraft_engine::sheetplan::Severity;
use cadcraft_engine::sheetplan::ids::sheet_key;
use serde_json::json;
use support::pdf_probe::pages;

fn session() -> Session {
    set_io(IoHooks {
        read: |bytes, name| cadcraft_io::read(bytes, name).map_err(|error| error.to_string()),
        write: |d, _| Ok(cadcraft_io::write_dxf(d).into_bytes()),
        plot: Some(|d, space, opts| cadcraft_io::plotter::plot(d, space, opts).map_err(|error| error.to_string())),
    });
    Session::new()
}

/// Publish the last scan, approving its warnings (missing fonts on the real drawing).
fn publish(s: &mut Session) -> Vec<u8> {
    let plan = &s.ubplot.last.as_ref().unwrap().plan;
    let approvals: Vec<_> = plan
        .issues
        .iter()
        .filter(|issue| issue.severity == Severity::Warn)
        .map(|issue| json!({"sheetKey": "*", "code": issue.code}))
        .chain(plan.sheets.iter().flat_map(|sheet| {
            sheet
                .issues
                .iter()
                .filter(|issue| issue.severity == Severity::Warn)
                .map(|issue| json!({"sheetKey": sheet_key(sheet), "code": issue.code}))
        }))
        .collect();
    if !approvals.is_empty() {
        s.execute("ubplot.approve", &json!({"approvals": approvals, "by": "user"})).unwrap();
    }
    let result = s.execute("ubplot.publish", &json!({})).unwrap();
    base64_decode(result["files"][0]["data"].as_str().unwrap()).unwrap()
}

/// The `/Length` of each page's content stream, in page object order.
fn content_lengths(pdf: &[u8]) -> Vec<usize> {
    let text = String::from_utf8_lossy(pdf);
    text.split("/Contents ")
        .skip(1)
        .filter_map(|rest| {
            let id = rest.split_whitespace().next()?;
            let body = text.split_once(&format!("\n{id} 0 obj\n"))?.1;
            body.strip_prefix("<< /Length ")?.split_whitespace().next()?.parse().ok()
        })
        .collect()
}

#[test]
fn kcs_real_drawing_publishes_a_small_pdf() {
    let Ok(path) = std::env::var("UBPLOT_KCS_DWG") else {
        eprintln!("UBPLOT_KCS_DWG is not set; skipping the real-drawing PDF size check");
        return;
    };
    let mut s = session();
    s.execute("open", &json!({"path": path})).unwrap();
    s.execute("ubplot.scan", &json!({"mode": "kcs"})).unwrap();
    let bytes = publish(&mut s);
    let lengths = content_lengths(&bytes);
    eprintln!("KCS PDF: {} bytes, page contents {lengths:?}", bytes.len());
    assert_eq!(pages(&bytes).len(), 17);
    assert_eq!(lengths.len(), 17);
    assert!(bytes.len() <= 2_000_000, "{} bytes", bytes.len());
    assert!(lengths.iter().all(|l| *l <= 350_000), "{lengths:?}");
}
