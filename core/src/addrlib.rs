//! Reads an Address Library database (`versionlib-1-6-1170-0.bin`) so a "Identifier not
//! found, N" box from an SKSE plugin (Actor Limit Fix, 2026-10-14: 523948) can be answered
//! offline: does this Address Library file hold ID N at all?
//!
//! The layout is the one CommonLibSSE's REL::IDDatabase reads (format 1: header, then
//! delta-coded id/offset pairs). The real `versionlib-1-6-1170-0.bin` (Address Library 13) is
//! format 2, which this does NOT read (CONFIRMED by the first CI run, 2026-10-14): it reports
//! "format 2, only 1 is read". Format 2 still needs a reader written against the real file.

use std::collections::HashMap;

struct Cur<'a> {
    b: &'a [u8],
    at: usize,
}

impl Cur<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.b.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
}

/// ID -> offset (relative to the module base).
#[derive(Debug, Default)]
pub struct Db {
    pub version: [u32; 4],
    pub ids: HashMap<u64, u64>,
}

pub fn parse(b: &[u8]) -> Result<Db, String> {
    let mut c = Cur { b, at: 0 };
    let bad = || "not an Address Library file (cut short)".to_string();
    let format = c.u32().ok_or_else(bad)?;
    if format != 1 {
        return Err(format!("Address Library format {format}, only 1 is read"));
    }
    let mut version = [0u32; 4];
    for v in &mut version {
        *v = c.u32().ok_or_else(bad)?;
    }
    let name_len = c.u32().ok_or_else(bad)? as usize;
    c.take(name_len).ok_or_else(bad)?;
    let ptr = c.u32().ok_or_else(bad)? as u64;
    let count = c.u32().ok_or_else(bad)? as usize;
    if ptr == 0 || count > 8_000_000 {
        return Err("Address Library header is not plausible".into());
    }
    let (mut prev_id, mut prev_off) = (0u64, 0u64);
    let mut ids = HashMap::with_capacity(count);
    for _ in 0..count {
        let t = c.u8().ok_or_else(bad)?;
        let (lo, hi) = (t & 0xf, t >> 4);
        let id = match lo {
            0 => c.u64(),
            1 => Some(prev_id + 1),
            2 => c.u8().map(|x| prev_id + x as u64),
            3 => c.u8().map(|x| prev_id.wrapping_sub(x as u64)),
            4 => c.u16().map(|x| prev_id + x as u64),
            5 => c.u16().map(|x| prev_id.wrapping_sub(x as u64)),
            6 => c.u16().map(|x| x as u64),
            7 => c.u32().map(|x| x as u64),
            _ => None,
        }
        .ok_or_else(bad)?;
        let base = if hi & 8 != 0 { prev_off / ptr } else { prev_off };
        let mut off = match hi & 7 {
            0 => c.u64(),
            1 => Some(base + 1),
            2 => c.u8().map(|x| base + x as u64),
            3 => c.u8().map(|x| base.wrapping_sub(x as u64)),
            4 => c.u16().map(|x| base + x as u64),
            5 => c.u16().map(|x| base.wrapping_sub(x as u64)),
            6 => c.u16().map(|x| x as u64),
            _ => c.u32().map(|x| x as u64),
        }
        .ok_or_else(bad)?;
        if hi & 8 != 0 {
            off *= ptr;
        }
        ids.insert(id, off);
        prev_id = id;
        prev_off = off;
    }
    Ok(Db { version, ids })
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A small format-1 file: ids 100 -> 0x1000, 101 -> 0x1008 (id+1, offset via /8 +1 *8), 5000 -> 0x20.
    pub fn sample() -> Vec<u8> {
        let mut b = Vec::new();
        for x in [1u32, 1, 6, 1170, 0] {
            b.extend(x.to_le_bytes());
        }
        b.extend(4u32.to_le_bytes());
        b.extend(b"test");
        b.extend(8u32.to_le_bytes());
        b.extend(3u32.to_le_bytes());
        // id 100 as u16 (6), offset as u16 (6)
        b.extend([0x66]);
        b.extend(100u16.to_le_bytes());
        b.extend(0x1000u16.to_le_bytes());
        // id prev+1 (1), offset (prev/8)+1, scaled (hi = 8|1)
        b.extend([0x91]);
        // id 5000 as u16 (6), offset as u32 (7)
        b.extend([0x76]);
        b.extend(5000u16.to_le_bytes());
        b.extend(0x20u32.to_le_bytes());
        b
    }

    #[test]
    fn reads_the_delta_coded_pairs() {
        let db = parse(&sample()).unwrap();
        assert_eq!(db.version, [1, 6, 1170, 0]);
        assert_eq!(db.ids.get(&100), Some(&0x1000));
        assert_eq!(db.ids.get(&101), Some(&0x1008));
        assert_eq!(db.ids.get(&5000), Some(&0x20));
        assert!(!db.ids.contains_key(&523948));
    }

    #[test]
    fn refuses_other_formats_and_cut_files() {
        assert!(parse(b"").is_err());
        let mut s = sample();
        s.truncate(s.len() - 3);
        assert!(parse(&s).is_err());
        let mut f2 = sample();
        f2[0] = 2;
        assert!(parse(&f2).unwrap_err().contains("format 2"));
    }
}
