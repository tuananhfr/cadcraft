//! The shapes 1.x table: three u16 (first number, last number, count), `count` entries of
//! `(code u16, len u16)`, then the bodies back to back. Shape 0, when present, is the font header.
use std::collections::BTreeMap;

use super::ShxError;

/// Bounds-checked little-endian reader; every read reports `Truncated` instead of indexing.
pub(super) struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Reader { rest: bytes }
    }

    pub(super) fn take(&mut self, n: usize) -> Result<&'a [u8], ShxError> {
        let (head, tail) = self.rest.split_at_checked(n).ok_or(ShxError::Truncated)?;
        self.rest = tail;
        Ok(head)
    }

    pub(super) fn u16(&mut self) -> Result<u16, ShxError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)]))
    }

    pub(super) fn u32(&mut self) -> Result<u32, ShxError> {
        let lo = u32::from(self.u16()?);
        let hi = u32::from(self.u16()?);
        Ok(lo | hi << 16)
    }
}

/// Name, then `above`, `below`, `modes`; the font header of both shapes and unifont files.
#[derive(Default)]
pub(super) struct FontHeader {
    pub(super) name: String,
    pub(super) above: u8,
    pub(super) below: u8,
    pub(super) modes: u8,
}

impl FontHeader {
    pub(super) fn parse(data: &[u8]) -> Option<FontHeader> {
        let nul = data.iter().position(|&b| b == 0)?;
        let name = data.get(..nul)?;
        let mut tail = data.get(nul + 1..)?.iter().copied();
        let (above, below, modes) = (tail.next()?, tail.next()?, tail.next()?);
        // Names are single-byte text in the file's own codepage; Latin-1 keeps every byte distinct.
        Some(FontHeader { name: name.iter().copied().map(char::from).collect(), above, below, modes })
    }
}

pub(super) fn parse(body: &[u8]) -> Result<(FontHeader, BTreeMap<u32, Vec<u8>>), ShxError> {
    let mut r = Reader::new(body);
    r.take(4)?;
    let count = usize::from(r.u16()?);
    let table_len = count.checked_mul(4).ok_or(ShxError::TooLarge)?;
    let table = r.take(table_len)?;

    let mut header = FontHeader::default();
    let mut have_header = false;
    let mut glyphs = BTreeMap::new();
    let (entries, _) = table.as_chunks::<4>();
    for [c0, c1, l0, l1] in entries {
        let code = u16::from_le_bytes([*c0, *c1]);
        let len = usize::from(u16::from_le_bytes([*l0, *l1]));
        let data = r.take(len)?;
        if code == 0 {
            if !have_header {
                header = FontHeader::parse(data).ok_or(ShxError::Truncated)?;
                have_header = true;
            }
        } else {
            glyphs.entry(u32::from(code)).or_insert_with(|| data.to_vec());
        }
    }
    Ok((header, glyphs))
}
