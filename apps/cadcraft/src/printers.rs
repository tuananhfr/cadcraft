use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use cadcraft_engine::sheetplan::device::{PrintOutcome, PrinterHost, PrinterInfo, set_printer_host, set_printer_refresh_hook};

const MAX_OUTPUT: u64 = 4 * 1024 * 1024;
const PROCESS_TIMEOUT: Duration = Duration::from_secs(20);

pub fn install() {
    set_printer_host(PrinterHost { list: || cached_printers(false), print_pdf });
    set_printer_refresh_hook(|| cached_printers(true));
}

#[derive(Default)]
struct PrinterCache {
    entry: Option<(Instant, Result<Vec<PrinterInfo>, String>)>,
}

impl PrinterCache {
    fn list(
        &mut self,
        query: fn() -> Result<Vec<PrinterInfo>, String>,
        refresh: bool,
        clock: impl Fn() -> Instant,
    ) -> Result<Vec<PrinterInfo>, String> {
        let now = clock();
        if !refresh
            && let Some((at, result)) = &self.entry
            && now.saturating_duration_since(*at) < Duration::from_secs(60)
        {
            return result.clone();
        }
        let result = query();
        self.entry = Some((clock(), result.clone()));
        result
    }
}

fn cached_printers(refresh: bool) -> Result<Vec<PrinterInfo>, String> {
    static CACHE: Mutex<PrinterCache> = Mutex::new(PrinterCache { entry: None });
    CACHE.lock().map_err(|_| "printer cache is unavailable".to_string())?.list(query_printers, refresh, Instant::now)
}

fn process_output(command: &mut Command) -> Result<String, String> {
    let mut child = command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|error| error.to_string())?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("Printer query has no output pipe.".into());
    };
    let (send, receive) = mpsc::sync_channel(1);
    let reader = std::thread::Builder::new().name("printer-output".into()).spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout.take(MAX_OUTPUT + 1).read_to_end(&mut bytes).map_err(|error| error.to_string()).and_then(|_| {
            if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_OUTPUT {
                Err("Printer process output exceeds 4 MiB.".into())
            } else {
                String::from_utf8(bytes).map_err(|error| error.to_string())
            }
        });
        let _ = send.send(result);
    });
    if let Err(error) = reader {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error.to_string());
    }
    let started = Instant::now();
    let mut output = None;
    let mut status = None;
    let result = loop {
        if output.is_none() {
            match receive.try_recv() {
                Ok(Ok(text)) => output = Some(text),
                Ok(Err(error)) => break Err(error),
                Err(mpsc::TryRecvError::Disconnected) => break Err("Printer process output reader stopped.".into()),
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(value) => status = value,
                Err(error) => break Err(error.to_string()),
            }
        }
        if let (Some(status), Some(output)) = (status, output.as_ref()) {
            break if status.success() { Ok(output.clone()) } else { Err(format!("Printer process failed: {status}")) };
        }
        if started.elapsed() >= PROCESS_TIMEOUT {
            break Err("Printer process timed out after 20 seconds.".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

#[cfg(target_os = "windows")]
fn powershell(script: &str) -> Command {
    use std::os::windows::process::CommandExt;

    // CREATE_NO_WINDOW: the GUI build has no console, so each query would otherwise flash one.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Command", script]).creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(target_os = "windows")]
fn query_printers() -> Result<Vec<PrinterInfo>, String> {
    let text = process_output(&mut powershell(
        "[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false); \
         $ErrorActionPreference = 'Stop'; \
         Get-CimInstance Win32_Printer | Select-Object Name,Default,WorkOffline,PrinterPaperNames | ConvertTo-Json -Depth 3",
    ))?;
    Ok(parse_cim_printers(&text))
}

#[cfg(not(target_os = "windows"))]
fn query_printers() -> Result<Vec<PrinterInfo>, String> {
    let names = process_output(Command::new("lpstat").env("LC_ALL", "C").arg("-e"))?;
    let default = process_output(Command::new("lpstat").env("LC_ALL", "C").arg("-d"))
        .ok()
        .and_then(|text| text.trim().split_once(':').map(|(_, name)| name.trim().to_string()));
    let mut printers = Vec::new();
    for name in names.lines().map(str::trim).filter(|name| valid_printer_name(name)).take(256) {
        let options = process_output(Command::new("lpoptions").env("LC_ALL", "C").args(["-p", name, "-l"])).unwrap_or_default();
        let status = process_output(Command::new("lpstat").env("LC_ALL", "C").args(["-p", name]));
        let mut printer = parse_lpoptions(name, &options);
        printer.is_default = default.as_deref() == Some(name);
        printer.offline = status.as_ref().map_or(true, |text| text.contains(" disabled "));
        printers.push(printer);
    }
    Ok(printers)
}

fn valid_printer_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 1024 && !name.contains('"') && !name.chars().any(char::is_control)
}

fn print_pdf(printer: &str, path: &str) -> PrintOutcome {
    if !valid_printer_name(printer) {
        return PrintOutcome::NotSupported("Printer names cannot contain quotes or control characters.".into());
    }
    let path = match std::path::absolute(path) {
        Ok(path) if path.is_file() => path,
        _ => return PrintOutcome::NotSupported("The PDF file is unavailable for printing.".into()),
    };
    let Some(path) = path.to_str().filter(|path| path.len() <= 32768 && !path.contains(['"', '\r', '\n'])) else {
        return PrintOutcome::NotSupported("The PDF path cannot be passed to the printer handler.".into());
    };
    submit_pdf(printer, path)
}

#[cfg(target_os = "windows")]
fn submit_pdf(printer: &str, path: &str) -> PrintOutcome {
    // Environment variables keep printer names and paths out of PowerShell source text.
    let mut command = powershell(
        "$ErrorActionPreference = 'Stop'; \
         $path = $env:CADCRAFT_PRINT_PDF; \
         $shell = New-Object -ComObject Shell.Application; \
         $item = $shell.Namespace((Split-Path -LiteralPath $path)).ParseName([IO.Path]::GetFileName($path)); \
         $verbs = @($item.Verbs() | ForEach-Object { ($_.Name -replace '[&\\s]', '').ToLowerInvariant() }); \
         if ($verbs -notcontains 'printto') { Write-Output 'NOT_SUPPORTED'; exit 0 }; \
         $printerArgument = [char]34 + $env:CADCRAFT_PRINT_PRINTER + [char]34; \
         Start-Process -FilePath $path -Verb PrintTo -ArgumentList @($printerArgument) -ErrorAction Stop; \
         Write-Output 'SPOOLED'",
    );
    command.env("CADCRAFT_PRINT_PDF", path).env("CADCRAFT_PRINT_PRINTER", printer);
    match process_output(&mut command) {
        Ok(text) if text.trim() == "SPOOLED" => PrintOutcome::Spooled { printer: printer.into() },
        Ok(_) => PrintOutcome::NotSupported("The PDF handler does not expose PrintTo; keeping PDF files.".into()),
        Err(error) => PrintOutcome::NotSupported(error),
    }
}

#[cfg(not(target_os = "windows"))]
fn submit_pdf(printer: &str, path: &str) -> PrintOutcome {
    match process_output(Command::new("lp").env("LC_ALL", "C").args(["-d", printer, "-o", "fit-to-page=false", "-o", "print-scaling=none", path])) {
        Ok(_) => PrintOutcome::Spooled { printer: printer.into() },
        Err(error) => PrintOutcome::NotSupported(error),
    }
}

#[cfg(any(target_os = "windows", test))]
fn parse_cim_printers(text: &str) -> Vec<PrinterInfo> {
    if u64::try_from(text.len()).unwrap_or(u64::MAX) > MAX_OUTPUT {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}')) else { return Vec::new() };
    let entries: &[serde_json::Value] = match &value {
        serde_json::Value::Array(entries) => entries,
        serde_json::Value::Object(_) => std::slice::from_ref(&value),
        _ => return Vec::new(),
    };
    entries
        .iter()
        .take(256)
        .filter_map(|entry| {
            let name = entry.get("Name")?.as_str()?;
            if !valid_printer_name(name) {
                return None;
            }
            let mut papers = Vec::new();
            if let Some(names) = entry.get("PrinterPaperNames") {
                match names {
                    serde_json::Value::Array(names) => {
                        for name in names.iter().take(512).filter_map(serde_json::Value::as_str) {
                            add_paper(&mut papers, name);
                        }
                    }
                    serde_json::Value::String(name) => add_paper(&mut papers, name),
                    _ => {}
                }
            }
            Some(PrinterInfo {
                name: name.into(),
                is_default: entry.get("Default").and_then(serde_json::Value::as_bool).unwrap_or(false),
                offline: entry.get("WorkOffline").and_then(serde_json::Value::as_bool).unwrap_or(false),
                papers,
            })
        })
        .collect()
}

#[cfg(any(not(target_os = "windows"), test))]
fn parse_lpoptions(name: &str, text: &str) -> PrinterInfo {
    let mut papers = Vec::new();
    if u64::try_from(text.len()).unwrap_or(u64::MAX) <= MAX_OUTPUT {
        for line in text.lines().take(4096) {
            let Some((key, values)) = line.split_once(':') else { continue };
            if key.split('/').next() != Some("PageSize") {
                continue;
            }
            for value in values.split_whitespace().take(512) {
                add_paper(&mut papers, value.trim_start_matches('*'));
            }
        }
    }
    PrinterInfo { name: name.into(), is_default: false, offline: false, papers }
}

fn add_paper(papers: &mut Vec<(String, f64, f64)>, name: &str) {
    if papers.len() < 512
        && let Some(paper) = paper_from_name(name)
        && !papers.iter().any(|existing| existing.0 == paper.0)
    {
        papers.push(paper);
    }
}

fn paper_from_name(name: &str) -> Option<(String, f64, f64)> {
    let name = name.trim();
    if name.is_empty() || name.len() > 256 {
        return None;
    }
    let compact: String = name.chars().filter(|ch| !ch.is_whitespace() && *ch != '-').collect::<String>().to_ascii_uppercase();
    let iso = compact.strip_prefix("ISO").unwrap_or(&compact);
    let dimensions = match iso {
        "A0" => Some(("A0", 841.0, 1189.0)),
        "A1" => Some(("A1", 594.0, 841.0)),
        "A2" => Some(("A2", 420.0, 594.0)),
        "A3" => Some(("A3", 297.0, 420.0)),
        "A4" => Some(("A4", 210.0, 297.0)),
        "A5" => Some(("A5", 148.0, 210.0)),
        "A6" => Some(("A6", 105.0, 148.0)),
        "B0" => Some(("B0", 1000.0, 1414.0)),
        "B1" => Some(("B1", 707.0, 1000.0)),
        "B2" => Some(("B2", 500.0, 707.0)),
        "B3" => Some(("B3", 353.0, 500.0)),
        "B4" => Some(("B4", 250.0, 353.0)),
        "B5" => Some(("B5", 176.0, 250.0)),
        "B6" => Some(("B6", 125.0, 176.0)),
        "JISB4" => Some(("JIS B4", 257.0, 364.0)),
        "JISB5" => Some(("JIS B5", 182.0, 257.0)),
        _ => None,
    };
    if let Some((label, width, height)) = dimensions {
        return Some((label.into(), width, height));
    }
    let inches = match compact.as_str() {
        "ANSIA" | "LETTER" => Some(("ANSI A", 8.5, 11.0)),
        "LEGAL" => Some(("Legal", 8.5, 14.0)),
        "ANSIB" | "TABLOID" => Some(("ANSI B", 11.0, 17.0)),
        "ANSIC" => Some(("ANSI C", 17.0, 22.0)),
        "ANSID" => Some(("ANSI D", 22.0, 34.0)),
        "ANSIE" => Some(("ANSI E", 34.0, 44.0)),
        _ => None,
    };
    let arch = compact.strip_prefix("ARCHITECTURE").or_else(|| compact.strip_prefix("ARCH"));
    let inches = inches.or(match arch.map(|arch| arch.strip_suffix("SHEET").unwrap_or(arch)) {
        Some("A") => Some(("ARCH A", 9.0, 12.0)),
        Some("B") => Some(("ARCH B", 12.0, 18.0)),
        Some("C") => Some(("ARCH C", 18.0, 24.0)),
        Some("D") => Some(("ARCH D", 24.0, 36.0)),
        Some("E") => Some(("ARCH E", 36.0, 48.0)),
        _ => None,
    });
    if let Some((label, width, height)) = inches {
        return Some((label.into(), width * 25.4, height * 25.4));
    }
    let upper = name.to_ascii_uppercase();
    let (size, factor) = upper.strip_suffix("MM").map(|size| (size, 1.0)).or_else(|| upper.strip_suffix("IN").map(|size| (size, 25.4)))?;
    let (width, height) = size.split_once('X')?;
    let width = width.trim().parse::<f64>().ok()? * factor;
    let height = height.trim().parse::<f64>().ok()? * factor;
    if !width.is_finite() || !height.is_finite() || !(1.0..=15000.0).contains(&width) || !(1.0..=15000.0).contains(&height) {
        return None;
    }
    Some((format!("{}x{}mm", width.min(height), width.max(height)), width.min(height), width.max(height)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::json;

    use super::*;

    #[test]
    fn printer_cache_expires_and_refresh_bypasses_cache() {
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        fn query() -> Result<Vec<PrinterInfo>, String> {
            CALLS.fetch_add(1, Ordering::Relaxed);
            Ok(Vec::new())
        }
        let mut cache = PrinterCache::default();
        let now = Instant::now();
        cache.list(query, false, || now).unwrap();
        cache.list(query, false, || now + Duration::from_secs(59)).unwrap();
        assert_eq!(CALLS.load(Ordering::Relaxed), 1);
        cache.list(query, false, || now + Duration::from_secs(60)).unwrap();
        cache.list(query, true, || now + Duration::from_secs(61)).unwrap();
        assert_eq!(CALLS.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn parse_cim_printers_reads_names_and_papers() {
        let text = json!({"Name": "Office", "Default": true, "WorkOffline": false, "PaperSizesSupported": [1, 1],
            "PrinterPaperNames": ["A3", "A4", " 648x917mm", "ISOA1", "Architecture DSheet", "Weird"]})
        .to_string();
        let printers = parse_cim_printers(&text);
        assert_eq!(printers.len(), 1);
        let printer = &printers[0];
        assert_eq!(printer.name, "Office");
        assert!(printer.is_default);
        assert!(!printer.offline);
        assert_eq!(printer.papers.len(), 5);
        assert_eq!(printer.papers[0], ("A3".into(), 297.0, 420.0));
        assert_eq!(printer.papers[1], ("A4".into(), 210.0, 297.0));
        assert_eq!(printer.papers[2], ("648x917mm".into(), 648.0, 917.0));
        assert_eq!(printer.papers[3], ("A1".into(), 594.0, 841.0));
        assert_eq!(printer.papers[4].0, "ARCH D");
        assert!((printer.papers[4].1 - 609.6).abs() < 1e-9);
        assert!((printer.papers[4].2 - 914.4).abs() < 1e-9);
    }

    #[test]
    fn parse_cim_single_object_or_array() {
        let printer = json!({"Name": "Test", "PrinterPaperNames": ["A4"]});
        assert_eq!(parse_cim_printers(&printer.to_string()), parse_cim_printers(&json!([printer]).to_string()));
        assert!(parse_cim_printers("{broken").is_empty());
        assert!(parse_cim_printers("null").is_empty());
    }

    #[test]
    fn parse_lpoptions_reads_page_sizes() {
        let printer = parse_lpoptions("Office", "PageSize/Media Size: *A4 A3 Custom.WIDTHxHEIGHT\nDuplex/Two sides: None");
        assert_eq!(printer.name, "Office");
        assert_eq!(printer.papers, vec![("A4".into(), 210.0, 297.0), ("A3".into(), 297.0, 420.0)]);
    }

    #[test]
    fn printer_name_with_quote_is_refused() {
        for name in ["a\"b", "a\nb", "a\rb"] {
            assert!(matches!(print_pdf(name, "unused.pdf"), PrintOutcome::NotSupported(_)));
        }
    }

    #[test]
    fn paper_from_name_inches_and_trim() {
        let (name, width, height) = paper_from_name(" 30x40in ").unwrap();
        assert_eq!(name, "762x1016mm");
        assert!((width - 762.0).abs() < 1e-9);
        assert!((height - 1016.0).abs() < 1e-9);
        assert_eq!(paper_from_name(" ISO A1 "), Some(("A1".into(), 594.0, 841.0)));
        assert_eq!(paper_from_name("JIS B4"), Some(("JIS B4".into(), 257.0, 364.0)));
        assert_eq!(paper_from_name("ISO B4"), Some(("B4".into(), 250.0, 353.0)));
        for name in ["Unknown", "Custom.WIDTHxHEIGHT", "NaNx20mm", "-10x20mm", "1e100x20mm"] {
            assert!(paper_from_name(name).is_none(), "{name}");
        }
    }
}
