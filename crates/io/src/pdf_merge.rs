//! Merging the restricted PDF format emitted by the plotter, without decoding its streams.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use cadcraft_doc::MAX_PDF_BYTES;

use crate::pdf::MAX_PAGES;
use crate::{IoError, Result};

/// Most input or output objects accepted by one merge.
pub const MAX_MERGE_OBJECTS: usize = 2_000_000;

const HEADER: &[u8] = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n";
const MAX_DEPTH: usize = 32;
const MAX_VALUES: usize = 4_000_000;

fn invalid() -> IoError {
    IoError::Format("not a UbotCAD PDF".into())
}

fn limit(what: &str) -> IoError {
    IoError::Format(format!("PDF merge exceeds {what}; publish it in parts (split)"))
}

fn integer(bytes: &[u8]) -> Result<usize> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return Err(invalid());
    }
    std::str::from_utf8(bytes).map_err(|_| invalid())?.parse().map_err(|_| invalid())
}

fn append(out: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    if out.len().checked_add(bytes.len()).is_none_or(|size| size > MAX_PDF_BYTES) {
        return Err(limit("512 MiB"));
    }
    out.extend_from_slice(bytes);
    Ok(())
}

enum Value<'a> {
    Atom(&'a [u8]),
    Ref(usize),
    Array(Vec<Value<'a>>),
    Dict(Vec<(&'a [u8], Value<'a>)>),
}

impl<'a> Value<'a> {
    fn get(&self, key: &[u8]) -> Result<&Self> {
        let Self::Dict(entries) = self else { return Err(invalid()) };
        entries.iter().find(|(name, _)| *name == key).map(|(_, value)| value).ok_or_else(invalid)
    }

    fn optional(&self, key: &[u8]) -> Option<&Self> {
        self.get(key).ok()
    }

    fn atom(&self) -> Result<&'a [u8]> {
        match self {
            Self::Atom(bytes) => Ok(bytes),
            _ => Err(invalid()),
        }
    }

    fn is(&self, bytes: &[u8]) -> bool {
        matches!(self, Self::Atom(value) if *value == bytes)
    }

    fn id(&self) -> Result<usize> {
        match self {
            Self::Ref(id) => Ok(*id),
            _ => Err(invalid()),
        }
    }

    fn array(&self) -> Result<&[Self]> {
        match self {
            Self::Array(values) => Ok(values),
            _ => Err(invalid()),
        }
    }

    fn number(&self) -> Result<usize> {
        integer(self.atom()?)
    }

    fn text(&self) -> Result<&'a [u8]> {
        let bytes = self.atom()?;
        if bytes.starts_with(b"(") || (bytes.starts_with(b"<") && !bytes.starts_with(b"<<")) { Ok(bytes) } else { Err(invalid()) }
    }

    fn write(&self, out: &mut Vec<u8>, ids: &[usize]) -> Result<()> {
        match self {
            Self::Atom(bytes) => append(out, bytes),
            Self::Ref(old) => {
                let id = ids.get(*old).copied().filter(|id| *id != 0).ok_or_else(invalid)?;
                append(out, format!("{id} 0 R").as_bytes())
            }
            Self::Array(values) => {
                append(out, b"[")?;
                for (index, value) in values.iter().enumerate() {
                    if index > 0 {
                        append(out, b" ")?;
                    }
                    value.write(out, ids)?;
                }
                append(out, b"]")
            }
            Self::Dict(entries) => {
                append(out, b"<<")?;
                for (key, value) in entries {
                    append(out, b" ")?;
                    append(out, key)?;
                    append(out, b" ")?;
                    value.write(out, ids)?;
                }
                append(out, b" >>")
            }
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

fn delimiter(byte: u8) -> bool {
    byte.is_ascii_whitespace() || b"()<>[]{}/%".contains(&byte)
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(len).ok_or_else(invalid)?;
        let bytes = self.bytes.get(self.at..end).ok_or_else(invalid)?;
        self.at = end;
        Ok(bytes)
    }

    fn rest(&self) -> Result<&'a [u8]> {
        self.bytes.get(self.at..).ok_or_else(invalid)
    }

    fn line(&mut self) -> Result<&'a [u8]> {
        let len = self.rest()?.iter().position(|b| *b == b'\n').ok_or_else(invalid)?;
        self.take(len + 1)?.strip_suffix(b"\n").ok_or_else(invalid)
    }

    fn whitespace(&mut self) {
        while self.bytes.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn token(&mut self) -> Result<&'a [u8]> {
        self.whitespace();
        let start = self.at;
        let byte = *self.bytes.get(self.at).ok_or_else(invalid)?;
        self.at += 1;
        match byte {
            b'(' => {
                let mut depth = 1usize;
                while depth > 0 {
                    match self.take(1)?.first().copied().ok_or_else(invalid)? {
                        b'\\' => {
                            self.take(1)?;
                        }
                        b'(' => {
                            depth += 1;
                            if depth > MAX_DEPTH {
                                return Err(invalid());
                            }
                        }
                        b')' => depth -= 1,
                        _ => {}
                    }
                }
            }
            b'<' => {
                let mut digits = 0usize;
                loop {
                    let byte = *self.take(1)?.first().ok_or_else(invalid)?;
                    if byte == b'>' {
                        break;
                    }
                    if !byte.is_ascii_hexdigit() {
                        return Err(invalid());
                    }
                    digits += 1;
                }
                if !digits.is_multiple_of(2) {
                    return Err(invalid());
                }
            }
            b'/' => {
                while self.bytes.get(self.at).is_some_and(|b| !delimiter(*b)) {
                    self.at += 1;
                }
                if self.at == start + 1 {
                    return Err(invalid());
                }
            }
            _ if !delimiter(byte) => {
                while self.bytes.get(self.at).is_some_and(|b| !delimiter(*b)) {
                    self.at += 1;
                }
            }
            _ => return Err(invalid()),
        }
        self.bytes.get(start..self.at).ok_or_else(invalid)
    }

    fn value(&mut self, depth: usize, remaining: &mut usize) -> Result<Value<'a>> {
        if depth > MAX_DEPTH || *remaining == 0 {
            return Err(limit("dictionary complexity"));
        }
        *remaining -= 1;
        self.whitespace();
        if self.rest()?.starts_with(b"<<") {
            self.take(2)?;
            let mut entries = Vec::new();
            let mut names = BTreeSet::new();
            loop {
                self.whitespace();
                if self.rest()?.starts_with(b">>") {
                    self.take(2)?;
                    return Ok(Value::Dict(entries));
                }
                let key = self.token()?;
                if !key.starts_with(b"/") || !names.insert(key) {
                    return Err(invalid());
                }
                entries.push((key, self.value(depth + 1, remaining)?));
            }
        }
        if self.rest()?.starts_with(b"[") {
            self.take(1)?;
            let mut values = Vec::new();
            loop {
                self.whitespace();
                if self.rest()?.starts_with(b"]") {
                    self.take(1)?;
                    return Ok(Value::Array(values));
                }
                values.push(self.value(depth + 1, remaining)?);
            }
        }
        let token = self.token()?;
        if let Ok(id) = integer(token) {
            let after = self.at;
            if self.token().ok() == Some(b"0") && self.token().ok() == Some(b"R") {
                return Ok(Value::Ref(id));
            }
            self.at = after;
        }
        if !matches!(token.first(), Some(b'/' | b'(' | b'<'))
            && !matches!(token, b"true" | b"false" | b"null")
            && !std::str::from_utf8(token).ok().and_then(|s| s.parse::<f64>().ok()).is_some_and(f64::is_finite)
        {
            return Err(invalid());
        }
        Ok(Value::Atom(token))
    }
}

struct Object<'a> {
    value: Value<'a>,
    stream: Option<&'a [u8]>,
}

impl Object<'_> {
    fn is(&self, kind: &[u8]) -> bool {
        self.value.optional(b"/Type").is_some_and(|value| value.is(kind))
    }
}

struct Part<'a> {
    objects: Vec<Object<'a>>,
    catalog: usize,
    tree: usize,
    info: usize,
    pages: Vec<usize>,
    labels: Vec<&'a [u8]>,
    off: BTreeSet<usize>,
    large: bool,
}

impl<'a> Part<'a> {
    fn object(&self, id: usize) -> Result<&Object<'a>> {
        id.checked_sub(1).and_then(|index| self.objects.get(index)).ok_or_else(invalid)
    }

    fn read(bytes: &'a [u8], total_objects: &mut usize, values: &mut usize) -> Result<Self> {
        if !bytes.starts_with(HEADER) {
            return Err(invalid());
        }
        let marker = b"\nstartxref\n";
        let at = bytes.windows(marker.len()).rposition(|w| w == marker).ok_or_else(invalid)? + marker.len();
        let mut footer = Reader { bytes, at };
        let xref = integer(footer.line()?)?;
        if footer.rest()? != b"%%EOF\n" {
            return Err(invalid());
        }
        let mut reader = Reader { bytes, at: xref };
        if reader.line()? != b"xref" {
            return Err(invalid());
        }
        let count = integer(reader.line()?.strip_prefix(b"0 ").ok_or_else(invalid)?)?;
        let object_count = count.checked_sub(1).ok_or_else(invalid)?;
        *total_objects = total_objects.checked_add(object_count).filter(|n| *n <= MAX_MERGE_OBJECTS).ok_or_else(|| limit("object limit"))?;
        let row_bytes = count.checked_mul(20).ok_or_else(invalid)?;
        if row_bytes > reader.rest()?.len() {
            return Err(invalid());
        }
        if reader.take(20)? != b"0000000000 65535 f \n" {
            return Err(invalid());
        }
        let mut offsets = Vec::with_capacity(object_count);
        let mut previous = 0usize;
        for _ in 0..object_count {
            let row = reader.take(20)?;
            if row.get(10..) != Some(b" 00000 n \n") {
                return Err(invalid());
            }
            let offset = integer(row.get(..10).ok_or_else(invalid)?)?;
            if offset <= previous || offset >= xref || (offsets.is_empty() && offset != HEADER.len()) {
                return Err(invalid());
            }
            previous = offset;
            offsets.push(offset);
        }
        if reader.line()? != b"trailer" {
            return Err(invalid());
        }
        let trailer = reader.value(0, values)?;
        if trailer.get(b"/Size")?.number()? != count || reader.rest()? != bytes.get(at - marker.len()..).ok_or_else(invalid)? {
            return Err(invalid());
        }
        let Value::Dict(entries) = &trailer else { return Err(invalid()) };
        if entries.len() != 3 {
            return Err(invalid());
        }
        let mut objects = Vec::with_capacity(object_count);
        for (index, start) in offsets.iter().enumerate() {
            let end = offsets.get(index + 1).copied().unwrap_or(xref);
            let segment = bytes.get(*start..end).ok_or_else(invalid)?;
            let header = format!("{} 0 obj\n", index + 1);
            let body = segment.strip_prefix(header.as_bytes()).and_then(|s| s.strip_suffix(b"\nendobj\n")).ok_or_else(invalid)?;
            let mut reader = Reader { bytes: body, at: 0 };
            let value = reader.value(0, values)?;
            let stream = if reader.rest()?.starts_with(b"\nstream\n") {
                reader.take(b"\nstream\n".len())?;
                let len = value.get(b"/Length")?.number()?;
                let data = reader.take(len)?;
                if reader.rest()? != b"\nendstream" {
                    return Err(invalid());
                }
                Some(data)
            } else {
                if !reader.rest()?.is_empty() || value.optional(b"/Length").is_some() {
                    return Err(invalid());
                }
                None
            };
            objects.push(Object { value, stream });
        }
        let catalog = trailer.get(b"/Root")?.id()?;
        let info = trailer.get(b"/Info")?.id()?;
        let mut part = Self { objects, catalog, tree: 0, info, pages: Vec::new(), labels: Vec::new(), off: BTreeSet::new(), large: false };
        if !part.object(info)?.value.get(b"/Producer")?.is(b"(CADCraft)") || !part.object(catalog)?.is(b"/Catalog") {
            return Err(invalid());
        }
        part.tree = part.object(catalog)?.value.get(b"/Pages")?.id()?;
        part.validate()?;
        Ok(part)
    }

    fn validate(&mut self) -> Result<()> {
        let tree = self.object(self.tree)?;
        if !tree.is(b"/Pages") || self.catalog == self.info || self.tree == self.info {
            return Err(invalid());
        }
        let kids = tree.value.get(b"/Kids")?.array()?;
        if kids.is_empty() || kids.len() > MAX_PAGES || tree.value.get(b"/Count")?.number()? != kids.len() {
            return Err(invalid());
        }
        let mut seen = BTreeSet::new();
        let mut pages = Vec::with_capacity(kids.len());
        for kid in kids {
            let id = kid.id()?;
            let page = self.object(id)?;
            if !seen.insert(id) || !page.is(b"/Page") || page.value.get(b"/Parent")?.id()? != self.tree {
                return Err(invalid());
            }
            if page.value.get(b"/MediaBox")?.array()?.len() != 4 || self.object(page.value.get(b"/Contents")?.id()?)?.stream.is_none() {
                return Err(invalid());
            }
            pages.push(id);
        }
        if self.objects.iter().filter(|object| object.is(b"/Page")).count() != pages.len()
            || self.objects.iter().filter(|object| object.is(b"/Catalog")).count() != 1
            || self.objects.iter().filter(|object| object.is(b"/Pages")).count() != 1
        {
            return Err(invalid());
        }
        let catalog = &self.object(self.catalog)?.value;
        let nums = catalog.get(b"/PageLabels")?.get(b"/Nums")?.array()?;
        if nums.len() != pages.len() * 2 {
            return Err(invalid());
        }
        let mut labels = Vec::with_capacity(pages.len());
        for (index, [number, label]) in nums.as_chunks::<2>().0.iter().enumerate() {
            if number.number()? != index {
                return Err(invalid());
            }
            labels.push(label.get(b"/P")?.text()?);
        }
        let mut off = BTreeSet::new();
        let ocg_count = self.objects.iter().filter(|object| object.is(b"/OCG")).count();
        if let Some(properties) = catalog.optional(b"/OCProperties") {
            let groups = properties.get(b"/OCGs")?.array()?;
            let mut listed = BTreeSet::new();
            for group in groups {
                let id = group.id()?;
                if !self.object(id)?.is(b"/OCG") || !listed.insert(id) {
                    return Err(invalid());
                }
                self.object(id)?.value.get(b"/Name")?.text()?;
            }
            if listed.len() != ocg_count {
                return Err(invalid());
            }
            for group in properties.get(b"/D")?.get(b"/OFF")?.array()? {
                let id = group.id()?;
                if !listed.contains(&id) {
                    return Err(invalid());
                }
                off.insert(id);
            }
        } else if ocg_count != 0 {
            return Err(invalid());
        }
        self.large = catalog.optional(b"/Version").is_some_and(|version| version.is(b"/1.6"))
            || pages.iter().any(|id| self.object(*id).is_ok_and(|page| page.value.optional(b"/UserUnit").is_some()));
        self.pages = pages;
        self.labels = labels;
        self.off = off;
        Ok(())
    }
}

fn pdf_string(text: &str) -> String {
    let text: String = text.chars().take(1000).collect();
    if text.chars().all(|ch| ch.is_ascii() && !ch.is_ascii_control()) {
        let mut out = String::from("(");
        for ch in text.chars() {
            if matches!(ch, '(' | ')' | '\\') {
                out.push('\\');
            }
            out.push(ch);
        }
        out.push(')');
        out
    } else {
        let mut out = String::from("<FEFF");
        for unit in text.encode_utf16() {
            let _ = write!(out, "{unit:04X}");
        }
        out.push('>');
        out
    }
}

fn references(out: &mut Vec<u8>, ids: impl IntoIterator<Item = usize>) -> Result<()> {
    for (index, id) in ids.into_iter().enumerate() {
        if index > 0 {
            append(out, b" ")?;
        }
        append(out, format!("{id} 0 R").as_bytes())?;
    }
    Ok(())
}

fn begin_object(out: &mut Vec<u8>, offsets: &mut Vec<usize>) -> Result<()> {
    offsets.push(out.len());
    append(out, format!("{} 0 obj\n", offsets.len()).as_bytes())
}

/// Joins one-page-or-more PDFs written by `pdf::pdf_items` into one file, pages in input order.
/// Missing entries in `labels` retain the original page labels. Other PDF formats are refused.
pub fn merge_own(parts: &[&[u8]], labels: &[String], title: &str) -> Result<Vec<u8>> {
    if parts.is_empty() {
        return Err(IoError::Format("there are no PDFs to merge".into()));
    }
    if parts.len() > MAX_PAGES {
        return Err(limit("page limit"));
    }
    let mut input_bytes = 0usize;
    for bytes in parts {
        input_bytes = input_bytes.checked_add(bytes.len()).filter(|n| *n <= MAX_PDF_BYTES).ok_or_else(|| limit("512 MiB"))?;
    }
    let mut inputs = Vec::with_capacity(parts.len());
    let mut object_count = 0usize;
    let mut values = MAX_VALUES;
    let mut page_count = 0usize;
    for bytes in parts {
        let part = Part::read(bytes, &mut object_count, &mut values)?;
        page_count = page_count.checked_add(part.pages.len()).filter(|n| *n <= MAX_PAGES).ok_or_else(|| limit("page limit"))?;
        inputs.push(part);
    }
    let mut next_id = 4usize;
    let mut mappings = Vec::with_capacity(inputs.len());
    let mut retained = Vec::new();
    let mut groups = BTreeMap::new();
    let mut group_order = Vec::new();
    let mut off = BTreeSet::new();
    let mut pages = Vec::with_capacity(page_count);
    for (part_index, part) in inputs.iter().enumerate() {
        let mut ids = vec![0; part.objects.len() + 1];
        for (old, new) in [(part.catalog, 1), (part.tree, 2), (part.info, 3)] {
            *ids.get_mut(old).ok_or_else(invalid)? = new;
        }
        for (index, object) in part.objects.iter().enumerate() {
            let old = index + 1;
            if [part.catalog, part.tree, part.info].contains(&old) {
                continue;
            }
            let name = if object.is(b"/OCG") { Some(object.value.get(b"/Name")?.text()?) } else { None };
            let id = if let Some(id) = name.and_then(|name| groups.get(name)).copied() {
                id
            } else {
                if next_id > MAX_MERGE_OBJECTS {
                    return Err(limit("object limit"));
                }
                let id = next_id;
                next_id += 1;
                retained.push((part_index, old));
                if let Some(name) = name {
                    groups.insert(name, id);
                    group_order.push(id);
                }
                id
            };
            *ids.get_mut(old).ok_or_else(invalid)? = id;
            if part.off.contains(&old) {
                off.insert(id);
            }
        }
        for page in &part.pages {
            pages.push(*ids.get(*page).ok_or_else(invalid)?);
        }
        mappings.push(ids);
    }
    let mut out = HEADER.to_vec();
    let mut offsets = Vec::with_capacity(next_id - 1);
    begin_object(&mut out, &mut offsets)?;
    append(&mut out, b"<< /Type /Catalog /Pages 2 0 R")?;
    if inputs.iter().any(|part| part.large) {
        append(&mut out, b" /Version /1.6")?;
    } else if !group_order.is_empty() {
        append(&mut out, b" /Version /1.5")?;
    }
    if !group_order.is_empty() {
        append(&mut out, b" /OCProperties << /OCGs [")?;
        references(&mut out, group_order.iter().copied())?;
        append(&mut out, b"] /D << /Name (Layers) /Order [")?;
        references(&mut out, group_order.iter().copied())?;
        append(&mut out, b"] /OFF [")?;
        references(&mut out, off)?;
        append(&mut out, b"] >> >>")?;
        if page_count == 1 {
            append(&mut out, b" /PageMode /UseOC")?;
        }
    }
    append(&mut out, b" /PageLabels << /Nums [")?;
    let mut index = 0usize;
    for part in &inputs {
        for original in &part.labels {
            if index > 0 {
                append(&mut out, b" ")?;
            }
            append(&mut out, format!("{index} << /P ").as_bytes())?;
            if let Some(label) = labels.get(index) {
                append(&mut out, pdf_string(label).as_bytes())?;
            } else {
                append(&mut out, original)?;
            }
            append(&mut out, b" >>")?;
            index += 1;
        }
    }
    append(&mut out, b"] >> >>\nendobj\n")?;
    begin_object(&mut out, &mut offsets)?;
    append(&mut out, b"<< /Type /Pages /Kids [")?;
    references(&mut out, pages)?;
    append(&mut out, format!("] /Count {page_count} >>\nendobj\n").as_bytes())?;
    begin_object(&mut out, &mut offsets)?;
    append(&mut out, format!("<< /Producer (CADCraft) /Creator (CADCraft) /Title {} >>\nendobj\n", pdf_string(title)).as_bytes())?;
    for (part_index, old) in retained {
        let part = inputs.get(part_index).ok_or_else(invalid)?;
        let ids = mappings.get(part_index).ok_or_else(invalid)?;
        let object = part.object(old)?;
        begin_object(&mut out, &mut offsets)?;
        object.value.write(&mut out, ids)?;
        if let Some(stream) = object.stream {
            // The direct length excludes these delimiters; binary payloads must never enter the reference parser.
            append(&mut out, b"\nstream\n")?;
            append(&mut out, stream)?;
            append(&mut out, b"\nendstream")?;
        }
        append(&mut out, b"\nendobj\n")?;
    }
    let xref = out.len();
    append(&mut out, format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes())?;
    for offset in &offsets {
        append(&mut out, format!("{offset:010} 00000 n \n").as_bytes())?;
    }
    append(&mut out, format!("trailer\n<< /Size {} /Root 1 0 R /Info 3 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes())?;
    Ok(out)
}
