//! Reads single files out of the player's own Skyrim Special Edition archives
//! (BSA version 105), for the launcher's menu music (Timothy, 2026-09-26).
//! Nothing read here is copied out of the game folder or sent anywhere.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::{Error, Result};

const INCLUDE_DIR_NAMES: u32 = 0x1;
const INCLUDE_FILE_NAMES: u32 = 0x2;
const COMPRESSED: u32 = 0x4;
const EMBED_NAMES: u32 = 0x100;
const SIZE_MASK: u32 = 0x3FFF_FFFF;
const TOGGLE_COMPRESSION: u32 = 0x4000_0000;

#[derive(Debug, Clone)]
pub struct Entry {
    /// "music\special\mus_maintitle.xwm", lower case with backslashes.
    pub path: String,
    size: u32,
    offset: u32,
}

pub struct Archive {
    file: std::fs::File,
    flags: u32,
    pub entries: Vec<Entry>,
}

fn u32le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn bad(why: &str) -> Error {
    Error::Game(format!("unreadable Skyrim archive: {why}"))
}

impl Archive {
    pub fn open(path: &Path) -> Result<Self> {
        let mut file = std::fs::File::open(path)?;
        let mut head = [0u8; 36];
        file.read_exact(&mut head)?;
        if &head[..4] != b"BSA\0" {
            return Err(bad("not a BSA"));
        }
        let version = u32le(&head, 4);
        if version != 105 {
            return Err(bad(&format!("version {version}")));
        }
        let flags = u32le(&head, 12);
        let folders = u32le(&head, 16) as usize;
        let files = u32le(&head, 20) as usize;
        if folders > 100_000 || files > 1_000_000 || flags & INCLUDE_DIR_NAMES == 0 || flags & INCLUDE_FILE_NAMES == 0 {
            return Err(bad("unexpected layout"));
        }
        let mut rec = vec![0u8; folders * 24];
        file.read_exact(&mut rec)?;
        let counts: Vec<usize> = (0..folders).map(|i| u32le(&rec, i * 24 + 8) as usize).collect();
        let mut pending = Vec::with_capacity(files);
        for count in counts {
            let mut len = [0u8; 1];
            file.read_exact(&mut len)?;
            let mut name = vec![0u8; len[0] as usize];
            file.read_exact(&mut name)?;
            let folder = String::from_utf8_lossy(&name).trim_end_matches('\0').to_ascii_lowercase();
            let mut fr = vec![0u8; count * 16];
            file.read_exact(&mut fr)?;
            for j in 0..count {
                pending.push((folder.clone(), u32le(&fr, j * 16 + 8), u32le(&fr, j * 16 + 12)));
            }
        }
        // File names follow, null-terminated, in the same order.
        let names_len = u32le(&head, 28) as usize;
        if names_len > 64 * 1024 * 1024 {
            return Err(bad("file names too long"));
        }
        let mut names = vec![0u8; names_len];
        file.read_exact(&mut names)?;
        let mut entries = Vec::with_capacity(pending.len());
        let mut at = 0;
        for (folder, size, offset) in pending {
            let end = names[at..].iter().position(|b| *b == 0).map(|p| at + p).ok_or_else(|| bad("file names cut short"))?;
            let name = String::from_utf8_lossy(&names[at..end]).to_ascii_lowercase();
            at = end + 1;
            entries.push(Entry { path: format!("{folder}\\{name}"), size, offset });
        }
        Ok(Archive { file, flags, entries })
    }

    pub fn find(&self, pred: impl Fn(&str) -> bool) -> Option<&Entry> {
        self.entries.iter().find(|e| pred(&e.path))
    }

    pub fn read(&mut self, e: &Entry) -> Result<Vec<u8>> {
        let compressed = (self.flags & COMPRESSED != 0) != (e.size & TOGGLE_COMPRESSION != 0);
        let mut len = (e.size & SIZE_MASK) as usize;
        if len > 256 * 1024 * 1024 {
            return Err(bad("file too large"));
        }
        self.file.seek(SeekFrom::Start(e.offset as u64))?;
        if self.flags & EMBED_NAMES != 0 {
            let mut l = [0u8; 1];
            self.file.read_exact(&mut l)?;
            self.file.seek(SeekFrom::Current(l[0] as i64))?;
            len = len.saturating_sub(1 + l[0] as usize);
        }
        let mut data = vec![0u8; len];
        self.file.read_exact(&mut data)?;
        if !compressed {
            return Ok(data);
        }
        if data.len() < 4 {
            return Err(bad("compressed file cut short"));
        }
        let original = u32le(&data, 0) as usize;
        let mut out = Vec::with_capacity(original);
        lz4_flex::frame::FrameDecoder::new(&data[4..]).read_to_end(&mut out).map_err(|e| bad(&e.to_string()))?;
        Ok(out)
    }
}

/// The xWMA pieces XAudio2 needs: the WAVEFORMATEX bytes, the decoded-packet
/// table and the audio data.
#[derive(Debug)]
pub struct Xwma {
    pub format: Vec<u8>,
    pub dpds: Vec<u32>,
    pub data: Vec<u8>,
}

pub fn parse_xwma(b: &[u8]) -> Result<Xwma> {
    if b.len() < 12 || &b[..4] != b"RIFF" || &b[8..12] != b"XWMA" {
        return Err(bad("not an xWMA file"));
    }
    let (mut format, mut dpds, mut data) = (None, None, None);
    let mut at = 12;
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let len = u32le(b, at + 4) as usize;
        let body = b.get(at + 8..at + 8 + len).ok_or_else(|| bad("xWMA chunk cut short"))?;
        match id {
            b"fmt " => format = Some(body.to_vec()),
            b"dpds" => dpds = Some(body.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect::<Vec<u32>>()),
            b"data" => data = Some(body.to_vec()),
            _ => {}
        }
        at += 8 + len + (len & 1);
    }
    match (format, dpds, data) {
        (Some(f), Some(d), Some(a)) if f.len() >= 18 && !d.is_empty() => Ok(Xwma { format: f, dpds: d, data: a }),
        _ => Err(bad("xWMA file is missing its format, packet table or data")),
    }
}

/// The main title theme from the player's own Skyrim archives (or another
/// music track from them when that one isn't there).
pub fn menu_music(game_dir: &Path) -> Result<Xwma> {
    let data = game_dir.join("Data");
    let mut archives: Vec<std::path::PathBuf> = std::fs::read_dir(&data)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().map(|n| { let n = n.to_string_lossy().to_ascii_lowercase(); n.starts_with("skyrim - ") && n.ends_with(".bsa") }).unwrap_or(false))
        .collect();
    // Sounds first: that's where the music usually is.
    archives.sort_by_key(|p| !p.to_string_lossy().to_ascii_lowercase().contains("sounds"));
    for want in ["maintitle", "maintheme", "mus_explore"] {
        for path in &archives {
            let Ok(mut a) = Archive::open(path) else { continue };
            if let Some(e) = a.find(|p| p.starts_with("music\\") && p.ends_with(".xwm") && p.contains(want)).cloned() {
                return parse_xwma(&a.read(&e)?);
            }
        }
    }
    Err(Error::Game("no music found in Skyrim's archives".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: &[u8], body: &[u8]) -> Vec<u8> {
        let mut v = id.to_vec();
        v.extend((body.len() as u32).to_le_bytes());
        v.extend(body);
        if body.len() % 2 == 1 {
            v.push(0);
        }
        v
    }

    #[test]
    fn parses_xwma() {
        let mut body = b"XWMA".to_vec();
        body.extend(chunk(b"fmt ", &[0x61, 0x01, 2, 0, 0x44, 0xAC, 0, 0, 0, 0, 0, 0, 0, 0, 16, 0, 0, 0]));
        body.extend(chunk(b"dpds", &[1, 0, 0, 0, 2, 0, 0, 0]));
        body.extend(chunk(b"data", &[9, 9, 9]));
        let mut f = b"RIFF".to_vec();
        f.extend((body.len() as u32).to_le_bytes());
        f.extend(body);
        let x = parse_xwma(&f).unwrap();
        assert_eq!(x.dpds, [1, 2]);
        assert_eq!(x.data, [9, 9, 9]);
        assert!(parse_xwma(b"RIFF\0\0\0\0WAVE").is_err());
    }

    /// A tiny v105 archive with one uncompressed file.
    #[test]
    fn reads_an_archive() {
        let folder = b"music\\special\0";
        let fname = b"mus_maintitle.xwm\0";
        let content = b"hello";
        let header_len = 36 + 24;
        let folder_block = 1 + folder.len() + 16;
        let data_at = header_len + folder_block + fname.len();
        let mut f = b"BSA\0".to_vec();
        for v in [105u32, 36, INCLUDE_DIR_NAMES | INCLUDE_FILE_NAMES, 1, 1, folder.len() as u32, fname.len() as u32] {
            f.extend(v.to_le_bytes());
        }
        f.extend([0u8; 4]);
        f.extend([0u8; 8]);
        f.extend(1u32.to_le_bytes());
        f.extend([0u8; 4]);
        f.extend(0u64.to_le_bytes());
        f.push(folder.len() as u8);
        f.extend(folder);
        f.extend([0u8; 8]);
        f.extend((content.len() as u32).to_le_bytes());
        f.extend((data_at as u32).to_le_bytes());
        f.extend(fname);
        f.extend(content);
        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("a.bsa");
        std::fs::write(&p, f).unwrap();
        let mut a = Archive::open(&p).unwrap();
        let e = a.find(|p| p.contains("maintitle")).unwrap().clone();
        assert_eq!(e.path, "music\\special\\mus_maintitle.xwm");
        assert_eq!(a.read(&e).unwrap(), content);
    }
}
