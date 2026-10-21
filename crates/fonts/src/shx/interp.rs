//! Shape-code interpreter: turns a glyph's byte program into pen strokes.
//!
//! The code table is the public "Shape Descriptions" one. Every limit below exists because the
//! programs come from files we do not control: a malformed program stops the glyph and returns
//! what was drawn so far.
use std::f64::consts::{FRAC_PI_4, TAU};

use super::{ShxFont, ShxKind};

/// One glyph as polylines in units of the font's `above` height.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShxGlyph {
    pub strokes: Vec<Vec<[f32; 2]>>,
    pub advance: [f32; 2],
}

pub const MAX_SUBSHAPE_DEPTH: u32 = 10;
pub const MAX_POINTS: usize = 20_000;
pub const ARC_SEGMENTS_PER_OCTANT: u32 = 4;

/// Shared work allowance per glyph, charged per decoded or scanned byte, sequence entry and arc
/// segment, so no program shape (fan-out, long sequences, tiny arcs) can run away.
const WORK_BUDGET: u32 = 3_000_000;
/// Coordinates beyond this are garbage (a runaway scale); stopping here keeps f32 output finite.
const MAX_COORD: f64 = 1e7;
/// The public description allows four saved positions; extra room costs nothing and keeps
/// slightly deeper fonts working.
const MAX_STACK: usize = 16;

/// Unit displacement per direction code; the shallow directions use the documented (1, 0.5) steps
/// rather than true 22.5 degree vectors.
const DIRECTIONS: [[f64; 2]; 16] = [
    [1.0, 0.0],
    [1.0, 0.5],
    [1.0, 1.0],
    [0.5, 1.0],
    [0.0, 1.0],
    [-0.5, 1.0],
    [-1.0, 1.0],
    [-1.0, 0.5],
    [-1.0, 0.0],
    [-1.0, -0.5],
    [-1.0, -1.0],
    [-0.5, -1.0],
    [0.0, -1.0],
    [0.5, -1.0],
    [1.0, -1.0],
    [1.0, -0.5],
];

impl ShxFont {
    /// Builds the glyph for `code`; `None` only when the font has no such glyph.
    pub fn glyph(&self, code: u32) -> Option<ShxGlyph> {
        let body = self.glyph_bytes(code)?;
        let mut machine = Machine::new(self);
        // A halt just ends the glyph; the partial drawing is the result.
        let _ = machine.run(body, 0);
        Some(machine.finish())
    }
}

/// Marker for "stop the glyph here".
struct Halt;

enum Op<'a> {
    End,
    PenDown,
    PenUp,
    Divide(u8),
    Multiply(u8),
    Push,
    Pop,
    Subshape(u32),
    Xy(i8, i8),
    XySequence(&'a [u8]),
    OctantArc { radius: u8, spec: u8 },
    FractionalArc { start_offset: u8, end_offset: u8, radius: u16, spec: u8 },
    BulgeArc { dx: i8, dy: i8, bulge: i8 },
    BulgeSequence(&'a [u8]),
    VerticalOnly,
    Vector { length: u8, direction: usize },
}

struct Cursor<'a> {
    rest: &'a [u8],
    wide_subshapes: bool,
}

impl<'a> Cursor<'a> {
    fn byte(&mut self) -> Option<u8> {
        let (first, rest) = self.rest.split_first()?;
        self.rest = rest;
        Some(*first)
    }

    /// An operand byte; running out of program here is malformed.
    fn operand(&mut self) -> Result<u8, Halt> {
        self.byte().ok_or(Halt)
    }

    fn signed_operand(&mut self) -> Result<i8, Halt> {
        self.operand().map(|b| i8::from_le_bytes([b]))
    }

    /// Operand bytes of the pair/triple sequences (codes 9 and 0xD), up to their (0,0)
    /// terminator. A sequence cut short keeps its complete entries.
    fn sequence(&mut self, entry_len: usize) -> &'a [u8] {
        let all = self.rest;
        let mut at = 0usize;
        let mut end = 0usize;
        while let Some(pair) = all.get(at..).and_then(|r| r.get(..2)) {
            if pair == [0, 0] {
                at += 2;
                break;
            }
            let Some(next) = at.checked_add(entry_len) else { break };
            if next > all.len() {
                at = all.len();
                break;
            }
            at = next;
            end = next;
        }
        self.rest = all.get(at..).unwrap_or_default();
        all.get(..end).unwrap_or_default()
    }

    /// `Ok(None)` when the program simply ends; an undefined code or a missing operand halts the
    /// whole glyph.
    fn next_op(&mut self) -> Result<Option<Op<'a>>, Halt> {
        let Some(code) = self.byte() else { return Ok(None) };
        Ok(Some(match code {
            0 => Op::End,
            1 => Op::PenDown,
            2 => Op::PenUp,
            3 => Op::Divide(self.operand()?),
            4 => Op::Multiply(self.operand()?),
            5 => Op::Push,
            6 => Op::Pop,
            7 => {
                let first = u32::from(self.operand()?);
                if self.wide_subshapes {
                    // Unifont numbers are stored high byte first: in the local corpus every
                    // unifont subshape reference resolves that way and none the other way.
                    let second = u32::from(self.operand()?);
                    Op::Subshape(first << 8 | second)
                } else {
                    Op::Subshape(first)
                }
            }
            8 => Op::Xy(self.signed_operand()?, self.signed_operand()?),
            9 => Op::XySequence(self.sequence(2)),
            0xA => Op::OctantArc { radius: self.operand()?, spec: self.operand()? },
            0xB => {
                let start_offset = self.operand()?;
                let end_offset = self.operand()?;
                let high = u16::from(self.operand()?);
                let low = u16::from(self.operand()?);
                Op::FractionalArc { start_offset, end_offset, radius: high << 8 | low, spec: self.operand()? }
            }
            0xC => Op::BulgeArc { dx: self.signed_operand()?, dy: self.signed_operand()?, bulge: self.signed_operand()? },
            0xD => Op::BulgeSequence(self.sequence(3)),
            0xE => Op::VerticalOnly,
            0xF => return Err(Halt),
            _ => Op::Vector { length: code >> 4, direction: usize::from(code & 0xF) },
        }))
    }
}

struct Machine<'f> {
    font: &'f ShxFont,
    pos: [f64; 2],
    pen_down: bool,
    scale: f64,
    stack: Vec<[f64; 2]>,
    strokes: Vec<Vec<[f64; 2]>>,
    current: Vec<[f64; 2]>,
    points: usize,
    work: u32,
}

impl<'f> Machine<'f> {
    fn new(font: &'f ShxFont) -> Self {
        Machine {
            font,
            pos: [0.0, 0.0],
            pen_down: true,
            scale: 1.0,
            stack: Vec::new(),
            strokes: Vec::new(),
            current: Vec::new(),
            points: 0,
            work: WORK_BUDGET,
        }
    }

    fn finish(mut self) -> ShxGlyph {
        self.end_stroke();
        // `above` is a height in drawing units; a font that declares 0 is treated as 1.
        let unit = if self.font.above == 0 { 1.0 } else { f64::from(self.font.above) };
        // Coordinates are bounded already; the check after the cast is the last line of defence.
        let out = |p: &[f64; 2]| Some([(p[0] / unit) as f32, (p[1] / unit) as f32]).filter(|q| q.iter().all(|v| v.is_finite()));
        let strokes: Vec<Vec<[f32; 2]>> = self.strokes.iter().map(|s| s.iter().map_while(out).collect::<Vec<_>>()).filter(|s| s.len() >= 2).collect();
        ShxGlyph { strokes, advance: out(&self.pos).unwrap_or_default() }
    }

    fn charge(&mut self, units: usize) -> Result<(), Halt> {
        let units = u32::try_from(units).unwrap_or(u32::MAX);
        self.work = self.work.checked_sub(units).ok_or(Halt)?;
        Ok(())
    }

    fn end_stroke(&mut self) {
        let stroke = std::mem::take(&mut self.current);
        if stroke.len() >= 2 {
            self.strokes.push(stroke);
        }
    }

    fn run(&mut self, body: &[u8], depth: u32) -> Result<(), Halt> {
        // Stored bodies are "name NUL program"; the name is not part of the program.
        let nul = body.iter().position(|b| *b == 0);
        self.charge(nul.map_or(body.len(), |n| n + 1))?;
        let program = nul.and_then(|n| body.get(n + 1..)).unwrap_or_default();
        let mut cursor = Cursor { rest: program, wide_subshapes: self.font.kind == ShxKind::Unifont };
        let mut skip_next = false;
        loop {
            let before = cursor.rest.len();
            let Some(op) = cursor.next_op()? else { return Ok(()) };
            self.charge(before.saturating_sub(cursor.rest.len()))?;
            match op {
                Op::End => return Ok(()),
                // Glyphs are laid out horizontally, so the command after 0xE never runs
                // (a second 0xE counts as that command).
                _ if skip_next => skip_next = false,
                Op::VerticalOnly => skip_next = true,
                op => self.apply(op, depth)?,
            }
        }
    }

    fn apply(&mut self, op: Op<'_>, depth: u32) -> Result<(), Halt> {
        match op {
            Op::End | Op::VerticalOnly => {}
            Op::PenDown => self.pen_down = true,
            Op::PenUp => {
                self.pen_down = false;
                self.end_stroke();
            }
            Op::Divide(0) => return Err(Halt),
            Op::Divide(d) => self.set_scale(self.scale / f64::from(d))?,
            Op::Multiply(m) => self.set_scale(self.scale * f64::from(m))?,
            Op::Push => {
                if self.stack.len() >= MAX_STACK {
                    return Err(Halt);
                }
                self.stack.push(self.pos);
            }
            Op::Pop => {
                let to = self.stack.pop().ok_or(Halt)?;
                self.end_stroke();
                self.pos = to;
            }
            Op::Subshape(number) => {
                if depth >= MAX_SUBSHAPE_DEPTH {
                    return Err(Halt);
                }
                // A call to a shape the font lacks is ignored rather than fatal.
                if let Some(body) = self.font.glyph_bytes(number) {
                    self.run(body, depth + 1)?;
                }
            }
            Op::Xy(dx, dy) => self.line_by(f64::from(dx), f64::from(dy))?,
            Op::XySequence(bytes) => {
                for &[dx, dy] in bytes.as_chunks::<2>().0 {
                    self.charge(1)?;
                    self.line_by(f64::from(i8::from_le_bytes([dx])), f64::from(i8::from_le_bytes([dy])))?;
                }
            }
            Op::Vector { length, direction } => {
                let [ux, uy] = DIRECTIONS.get(direction).copied().ok_or(Halt)?;
                self.line_by(ux * f64::from(length), uy * f64::from(length))?;
            }
            Op::OctantArc { radius, spec } => {
                // Negative arcs carry a sign bit (the local corpus has it on nearly all of them,
                // with counts never above 8); any other count is not an octant arc.
                let octants = match spec & 0xF {
                    0 => 8,
                    n @ 1..=8 => n,
                    _ => return Err(Halt),
                };
                let start = f64::from((spec >> 4) & 7) * FRAC_PI_4;
                let sweep = f64::from(octants) * FRAC_PI_4;
                let sweep = if spec & 0x80 != 0 { -sweep } else { sweep };
                self.arc_from_angle(f64::from(radius) * self.scale, start, sweep)?;
            }
            // UNVERIFIED: which octant the end offset is read in (S+C here, versus S+C-1) cannot
            // be told from the corpus.
            Op::FractionalArc { start_offset, end_offset, radius, spec } => {
                let octant = f64::from((spec >> 4) & 7);
                // Like 0xA: a count above 8 is not an octant count, and would draw a multi-turn arc.
                if spec & 0xF > 8 {
                    return Err(Halt);
                }
                let count = f64::from(spec & 0xF);
                let clockwise = spec & 0x80 != 0;
                let offset = |o: u8| f64::from(o) / 256.0;
                let start = (octant + offset(start_offset)) * FRAC_PI_4;
                // The end offset is read inside the octant `count` away from the start octant, in
                // the counter-clockwise sense; a count of 0 keeps the arc inside the start octant
                // (unlike code 0xA, where 0 means a full circle).
                let end_octant = if clockwise { octant - count } else { octant + count };
                let end = (end_octant + offset(end_offset)) * FRAC_PI_4;
                let mut sweep = end - start;
                if clockwise && sweep > 0.0 {
                    sweep -= TAU;
                } else if !clockwise && sweep < 0.0 {
                    sweep += TAU;
                }
                self.arc_from_angle(f64::from(radius) * self.scale, start, sweep)?;
            }
            Op::BulgeArc { dx, dy, bulge } => self.bulge_by(f64::from(dx), f64::from(dy), bulge)?,
            Op::BulgeSequence(bytes) => {
                for &[dx, dy, bulge] in bytes.as_chunks::<3>().0 {
                    self.charge(1)?;
                    let signed = |b: u8| i8::from_le_bytes([b]);
                    self.bulge_by(f64::from(signed(dx)), f64::from(signed(dy)), signed(bulge))?;
                }
            }
        }
        Ok(())
    }

    fn set_scale(&mut self, scale: f64) -> Result<(), Halt> {
        if !scale.is_finite() {
            return Err(Halt);
        }
        self.scale = scale;
        Ok(())
    }

    fn line_by(&mut self, dx: f64, dy: f64) -> Result<(), Halt> {
        self.move_to([self.pos[0] + dx * self.scale, self.pos[1] + dy * self.scale])
    }

    /// Moves the pen, drawing when it is down. The start of a stroke is the position before the
    /// first drawn segment.
    fn move_to(&mut self, to: [f64; 2]) -> Result<(), Halt> {
        if to.iter().any(|v| !v.is_finite() || v.abs() > MAX_COORD) {
            return Err(Halt);
        }
        if self.pen_down {
            if self.current.is_empty() {
                let from = self.pos;
                self.push_point(from)?;
            }
            self.push_point(to)?;
        }
        self.pos = to;
        Ok(())
    }

    fn push_point(&mut self, p: [f64; 2]) -> Result<(), Halt> {
        if self.current.last() == Some(&p) {
            return Ok(());
        }
        if self.points >= MAX_POINTS {
            return Err(Halt);
        }
        self.points += 1;
        self.current.push(p);
        Ok(())
    }

    /// Arc of `radius` that starts at the pen, at `start` radians on its circle.
    fn arc_from_angle(&mut self, radius: f64, start: f64, sweep: f64) -> Result<(), Halt> {
        let from_centre = [radius * start.cos(), radius * start.sin()];
        let centre = [self.pos[0] - from_centre[0], self.pos[1] - from_centre[1]];
        self.arc_about(centre, from_centre, sweep, None)
    }

    /// Bulge b/127 is tan(sweep/4), so +-127 is a half circle; positive turns counter-clockwise.
    fn bulge_by(&mut self, dx: f64, dy: f64, bulge: i8) -> Result<(), Halt> {
        let chord = [dx * self.scale, dy * self.scale];
        let end = [self.pos[0] + chord[0], self.pos[1] + chord[1]];
        let length = chord[0].hypot(chord[1]);
        if bulge == 0 || length == 0.0 {
            return self.move_to(end);
        }
        let sweep = 4.0 * (f64::from(bulge) / 127.0).atan();
        let half = sweep / 2.0;
        // The centre sits on the chord's left normal, `length/2 / tan(half)` away.
        let offset = length / 2.0 * half.cos() / half.sin();
        let mid = [self.pos[0] + chord[0] / 2.0, self.pos[1] + chord[1] / 2.0];
        let centre = [mid[0] - chord[1] / length * offset, mid[1] + chord[0] / length * offset];
        let from_centre = [self.pos[0] - centre[0], self.pos[1] - centre[1]];
        self.arc_about(centre, from_centre, sweep, Some(end))
    }

    /// Flattens the arc swept from `from_centre` about `centre`; `exact_end` pins the last point
    /// so chained bulge arcs do not drift.
    fn arc_about(&mut self, centre: [f64; 2], from_centre: [f64; 2], sweep: f64, exact_end: Option<[f64; 2]>) -> Result<(), Halt> {
        let rotate = |angle: f64| {
            let (s, c) = angle.sin_cos();
            [centre[0] + from_centre[0] * c - from_centre[1] * s, centre[1] + from_centre[0] * s + from_centre[1] * c]
        };
        let end = exact_end.unwrap_or_else(|| rotate(sweep));
        let segments = ((sweep.abs() / FRAC_PI_4 * f64::from(ARC_SEGMENTS_PER_OCTANT)).ceil() as u32).max(1);
        self.charge(segments as usize)?;
        if !self.pen_down {
            return self.move_to(end);
        }
        for i in 1..segments {
            self.move_to(rotate(sweep * f64::from(i) / f64::from(segments)))?;
        }
        self.move_to(end)
    }
}
