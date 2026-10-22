use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
pub struct ProbePage {
    pub media_box: [f64; 4],
    pub user_unit: f64,
    pub label: Option<String>,
}

#[derive(Clone, Debug)]
enum Node {
    Number(f64),
    Name(String),
    Text(String),
    Array(Vec<Node>),
    Dict(BTreeMap<String, Node>),
    Ref(u32),
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn whitespace(&mut self) {
        while self.bytes.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }

    fn word(&mut self) -> String {
        self.whitespace();
        let start = self.at;
        while self.bytes.get(self.at).is_some_and(|b| !b.is_ascii_whitespace() && !b"[]<>()/".contains(b)) {
            self.at += 1;
        }
        String::from_utf8_lossy(&self.bytes[start..self.at]).into_owned()
    }

    fn node(&mut self, depth: usize) -> Option<Node> {
        if depth > 32 {
            return None;
        }
        self.whitespace();
        match *self.bytes.get(self.at)? {
            b'<' if self.bytes.get(self.at + 1) == Some(&b'<') => {
                self.at += 2;
                let mut dict = BTreeMap::new();
                loop {
                    self.whitespace();
                    if self.bytes.get(self.at..self.at + 2) == Some(b">>") {
                        self.at += 2;
                        return Some(Node::Dict(dict));
                    }
                    if self.bytes.get(self.at) != Some(&b'/') {
                        return None;
                    }
                    self.at += 1;
                    let key = self.word();
                    dict.insert(key, self.node(depth + 1)?);
                }
            }
            b'[' => {
                self.at += 1;
                let mut items = Vec::new();
                loop {
                    self.whitespace();
                    if self.bytes.get(self.at) == Some(&b']') {
                        self.at += 1;
                        return Some(Node::Array(items));
                    }
                    items.push(self.node(depth + 1)?);
                }
            }
            b'/' => {
                self.at += 1;
                Some(Node::Name(self.word()))
            }
            b'(' => {
                self.at += 1;
                let mut value = Vec::new();
                let mut nesting = 1;
                while let Some(&byte) = self.bytes.get(self.at) {
                    self.at += 1;
                    match byte {
                        b'\\' => {
                            let escaped = *self.bytes.get(self.at)?;
                            self.at += 1;
                            value.push(match escaped {
                                b'n' => b'\n',
                                b'r' => b'\r',
                                b't' => b'\t',
                                other => other,
                            });
                        }
                        b'(' => {
                            nesting += 1;
                            value.push(byte);
                        }
                        b')' => {
                            nesting -= 1;
                            if nesting == 0 {
                                return Some(Node::Text(String::from_utf8_lossy(&value).into_owned()));
                            }
                            value.push(byte);
                        }
                        _ => value.push(byte),
                    }
                }
                None
            }
            b'<' => {
                self.at += 1;
                let start = self.at;
                while *self.bytes.get(self.at)? != b'>' {
                    self.at += 1;
                }
                let hex: String = String::from_utf8_lossy(&self.bytes[start..self.at]).chars().filter(|ch| !ch.is_whitespace()).collect();
                self.at += 1;
                let bytes: Vec<u8> =
                    hex.as_bytes().chunks(2).map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap()).collect();
                let text = if bytes.starts_with(&[0xfe, 0xff]) {
                    let units: Vec<u16> = bytes[2..].as_chunks::<2>().0.iter().map(|pair| u16::from_be_bytes(*pair)).collect();
                    String::from_utf16_lossy(&units)
                } else {
                    String::from_utf8_lossy(&bytes).into_owned()
                };
                Some(Node::Text(text))
            }
            _ => {
                let word = self.word();
                let number = word.parse().ok()?;
                let after = self.at;
                if let Ok(id) = word.parse::<u32>()
                    && self.word() == "0"
                    && self.word() == "R"
                {
                    return Some(Node::Ref(id));
                }
                self.at = after;
                Some(Node::Number(number))
            }
        }
    }
}

fn get<'a>(node: &'a Node, key: &str) -> Option<&'a Node> {
    match node {
        Node::Dict(dict) => dict.get(key),
        _ => None,
    }
}

fn resolved<'a>(node: &'a Node, objects: &'a BTreeMap<u32, Node>) -> Option<&'a Node> {
    match node {
        Node::Ref(id) => objects.get(id),
        _ => Some(node),
    }
}

fn number(node: &Node) -> Option<f64> {
    match node {
        Node::Number(value) => Some(*value),
        _ => None,
    }
}

pub fn pages(pdf: &[u8]) -> Vec<ProbePage> {
    let text = String::from_utf8_lossy(pdf);
    let mut objects = BTreeMap::new();
    for segment in text.split("endobj") {
        let Some((header, body)) = segment.rsplit_once(" 0 obj") else { continue };
        let Some(id) = header.split_whitespace().next_back().and_then(|id| id.parse::<u32>().ok()) else { continue };
        if let Some(node) = (Reader { bytes: body.as_bytes(), at: 0 }).node(0) {
            objects.insert(id, node);
        }
    }
    let trailer = text.rsplit_once("trailer").and_then(|(_, body)| (Reader { bytes: body.as_bytes(), at: 0 }).node(0)).unwrap();
    let root = resolved(get(&trailer, "Root").unwrap(), &objects).unwrap();
    let tree = resolved(get(root, "Pages").unwrap(), &objects).unwrap();
    let mut output = Vec::new();
    let mut pending = vec![(tree, None, 1.0)];
    let mut seen = BTreeSet::new();
    while let Some((node, inherited_box, inherited_unit)) = pending.pop() {
        let media = get(node, "MediaBox")
            .and_then(|value| resolved(value, &objects))
            .and_then(|value| match value {
                Node::Array(values) => {
                    let [a, b, c, d] = values.as_slice() else { return None };
                    Some([number(a)?, number(b)?, number(c)?, number(d)?])
                }
                _ => None,
            })
            .or(inherited_box);
        let unit = get(node, "UserUnit").and_then(number).unwrap_or(inherited_unit);
        match get(node, "Type") {
            Some(Node::Name(name)) if name == "Page" => output.push(ProbePage { media_box: media.unwrap(), user_unit: unit, label: None }),
            _ => {
                if let Some(Node::Array(kids)) = get(node, "Kids") {
                    for kid in kids.iter().rev() {
                        if let Node::Ref(id) = kid {
                            assert!(seen.insert(*id), "cyclic page tree");
                        }
                        pending.push((resolved(kid, &objects).unwrap(), media, unit));
                    }
                }
            }
        }
    }
    if let Some(labels) = get(root, "PageLabels").and_then(|value| resolved(value, &objects)) {
        let mut pending = vec![labels];
        for _ in 0..10_000 {
            let Some(node) = pending.pop() else { break };
            if let Some(Node::Array(nums)) = get(node, "Nums") {
                for pair in nums.as_chunks::<2>().0 {
                    if let (Some(index), Some(Node::Text(label))) = (number(&pair[0]), get(&pair[1], "P"))
                        && let Some(page) = output.get_mut(index as usize)
                    {
                        page.label = Some(label.clone());
                    }
                }
            }
            if let Some(Node::Array(kids)) = get(node, "Kids") {
                pending.extend(kids.iter().filter_map(|kid| resolved(kid, &objects)));
            }
        }
    }
    output
}
