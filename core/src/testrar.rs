//! Writes small stored (uncompressed) RAR v4 and v5 archives for the tests,
//! byte for byte from the published format, so CI makes its own stand-in
//! RARs and no real mod download is ever needed (PR #7, Codex 5875976813).

fn crc32(b: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &x in b {
        crc ^= x as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { 0xEDB8_8320 ^ (crc >> 1) } else { crc >> 1 };
        }
    }
    !crc
}

fn vint(mut n: u64, out: &mut Vec<u8>) {
    loop {
        let b = (n & 0x7F) as u8;
        n >>= 7;
        if n == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

/// A RAR5 block: CRC32, header size, then the header (type onwards).
fn block5(header: &[u8], out: &mut Vec<u8>) {
    let mut sized = Vec::new();
    vint(header.len() as u64, &mut sized);
    sized.extend_from_slice(header);
    out.extend_from_slice(&crc32(&sized).to_le_bytes());
    out.extend_from_slice(&sized);
}

/// RAR v5 with each file stored under the given name.
pub fn rar5(files: &[(&str, &[u8])]) -> Vec<u8> {
    rar5_with(files.iter().map(|(n, d)| (*n, *d, d.len() as u64)))
}

/// RAR v5 holding one byte under a header that claims `size` bytes.
pub fn rar5_claiming(name: &str, size: u64) -> Vec<u8> {
    rar5_with([(name, &b"x"[..], size)].into_iter())
}

fn rar5_with<'a>(files: impl Iterator<Item = (&'a str, &'a [u8], u64)>) -> Vec<u8> {
    let mut out = b"Rar!\x1a\x07\x01\x00".to_vec();
    // Main archive header: type 1, no flags, no archive flags.
    block5(&[1, 0, 0], &mut out);
    for (name, data, size) in files {
        let mut h = Vec::new();
        vint(2, &mut h); // file header
        vint(0x02, &mut h); // a data area follows
        vint(data.len() as u64, &mut h);
        vint(0x04, &mut h); // file flags: data CRC32 present
        vint(size, &mut h); // unpacked size
        vint(0x20, &mut h); // attributes (archive)
        h.extend_from_slice(&crc32(data).to_le_bytes());
        vint(0, &mut h); // compression: version 0, stored
        vint(0, &mut h); // host OS Windows
        vint(name.len() as u64, &mut h);
        h.extend_from_slice(name.as_bytes());
        block5(&h, &mut out);
        out.extend_from_slice(data);
    }
    // End of archive.
    block5(&[5, 0, 0], &mut out);
    out
}

/// A RAR4 block: its header CRC is the low 16 bits of CRC32 from the type on.
fn block4(kind: u8, flags: u16, rest: &[u8], out: &mut Vec<u8>) {
    let mut h = vec![kind];
    h.extend_from_slice(&flags.to_le_bytes());
    h.extend_from_slice(&((7 + rest.len()) as u16).to_le_bytes());
    h.extend_from_slice(rest);
    out.extend_from_slice(&((crc32(&h) & 0xFFFF) as u16).to_le_bytes());
    out.extend_from_slice(&h);
}

/// RAR v4 (2.9 format) with each file stored under the given name.
pub fn rar4(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = b"Rar!\x1a\x07\x00".to_vec();
    block4(0x73, 0, &[0; 6], &mut out);
    for (name, data) in files {
        let mut r = Vec::new();
        r.extend_from_slice(&(data.len() as u32).to_le_bytes()); // packed
        r.extend_from_slice(&(data.len() as u32).to_le_bytes()); // unpacked
        r.push(2); // host OS Windows
        r.extend_from_slice(&crc32(data).to_le_bytes());
        r.extend_from_slice(&0x0021_0000u32.to_le_bytes()); // 1980-01-01 00:00
        r.push(29); // version needed: 2.9
        r.push(0x30); // stored
        r.extend_from_slice(&(name.len() as u16).to_le_bytes());
        r.extend_from_slice(&0x20u32.to_le_bytes());
        r.extend_from_slice(name.as_bytes());
        block4(0x74, 0x8000, &r, &mut out);
        out.extend_from_slice(data);
    }
    block4(0x7B, 0x4000, &[], &mut out);
    out
}
