//! Character portrait (look-into 2026-10-08, Timothy picked the "portrait
//! snapshot": a small picture of the real character shown on the launcher's
//! character card). The launcher grabs the game window shortly after
//! RaceMenu closes, crops the middle of it to a head-and-shoulders
//! portrait, and keeps it as one WebP next to the other launcher state. Only
//! this module's checks decide what is kept or ever sent anywhere: WebP
//! only (the format the server serves, /ad/portrait/<actor>.webp), small,
//! sensible dimensions.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

pub const FOLDER: &str = ".aetherial-dawn/portrait";
pub const SELF_FILE: &str = "self.webp";
/// Largest portrait kept or sent.
pub const MAX_BYTES: usize = 400 * 1024;
/// Side limits in pixels.
pub const MIN_SIDE: u32 = 64;
pub const MAX_SIDE: u32 = 512;
/// Shortest wait between two grabs, so a player opening and closing
/// RaceMenu repeatedly doesn't make the launcher grab every time.
pub const MIN_GAP_SECS: u64 = 30;

/// Width and height of a WebP picture (lossless, lossy or extended), or an
/// error when `body` isn't one the launcher keeps (not WebP, too big, sides
/// outside [`MIN_SIDE`]..=[`MAX_SIDE`]).
pub fn check_webp(body: &[u8]) -> Result<(u32, u32)> {
    let bad = |why: &str| Error::Game(format!("the portrait isn't kept: {why}"));
    if body.len() > MAX_BYTES {
        return Err(bad("too big"));
    }
    if body.len() < 30 || &body[..4] != b"RIFF" || &body[8..12] != b"WEBP" {
        return Err(bad("not a WebP picture"));
    }
    if u32::from_le_bytes([body[4], body[5], body[6], body[7]]) as usize != body.len() - 8 {
        return Err(bad("its length doesn't add up"));
    }
    let le16 = |at: usize| (body[at] as u32) | ((body[at + 1] as u32) << 8);
    let le24 = |at: usize| le16(at) | ((body[at + 2] as u32) << 16);
    let (w, h) = match &body[12..16] {
        // Lossless: a 0x2f signature, then 14 bits of width-1 and 14 of height-1.
        b"VP8L" if body[20] == 0x2f => {
            let bits = u32::from_le_bytes([body[21], body[22], body[23], body[24]]);
            ((bits & 0x3fff) + 1, ((bits >> 14) & 0x3fff) + 1)
        }
        // Lossy key frame: the start code 9d 01 2a, then 14-bit width and height.
        b"VP8 " if body[23..26] == [0x9d, 0x01, 0x2a] => (le16(26) & 0x3fff, le16(28) & 0x3fff),
        // Extended: the canvas size, each minus one, in 24 bits.
        b"VP8X" => (le24(24) + 1, le24(27) + 1),
        _ => return Err(bad("not a WebP picture")),
    };
    if !(MIN_SIDE..=MAX_SIDE).contains(&w) || !(MIN_SIDE..=MAX_SIDE).contains(&h) {
        return Err(bad("its size is outside the limits"));
    }
    Ok((w, h))
}

/// The part of a game window `win` (width, height) to keep as the portrait:
/// (x, y, width, height), a 3:4 upright rectangle around the middle of the
/// window where the third-person camera frames the character.
pub fn crop_rect(win: (u32, u32)) -> Option<(u32, u32, u32, u32)> {
    let (ww, wh) = win;
    let w = ww.min(wh / 4 * 3);
    let h = w / 3 * 4;
    if w < MIN_SIDE || h < MIN_SIDE {
        return None;
    }
    Some(((ww - w) / 2, (wh - h) / 2, w, h))
}

/// Whether a grab is due: none yet, or the last one is at least
/// [`MIN_GAP_SECS`] old (a clock that went backwards counts as due).
pub fn due(last: Option<u64>, now: u64) -> bool {
    last.is_none_or(|last| now < last || now - last >= MIN_GAP_SECS)
}

/// Shrinks an RGBA picture (4 bytes a pixel, row by row) to `to` by averaging
/// the source pixels that fall in each target pixel. `src` must be exactly
/// `from.0 * from.1 * 4` bytes and no side may be zero.
pub fn downscale(src: &[u8], from: (u32, u32), to: (u32, u32)) -> Result<Vec<u8>> {
    let (fw, fh, tw, th) = (from.0 as usize, from.1 as usize, to.0 as usize, to.1 as usize);
    if fw == 0 || fh == 0 || tw == 0 || th == 0 || tw > fw || th > fh || src.len() != fw * fh * 4 {
        return Err(Error::Game("the portrait picture has the wrong size".into()));
    }
    let mut out = Vec::with_capacity(tw * th * 4);
    for ty in 0..th {
        let (y0, y1) = (ty * fh / th, ((ty + 1) * fh / th).max(ty * fh / th + 1));
        for tx in 0..tw {
            let (x0, x1) = (tx * fw / tw, ((tx + 1) * fw / tw).max(tx * fw / tw + 1));
            let mut sum = [0u32; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let at = (y * fw + x) * 4;
                    for (c, s) in sum.iter_mut().enumerate() {
                        *s += src[at + c] as u32;
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            out.extend(sum.iter().map(|s| (s / n) as u8));
        }
    }
    Ok(out)
}

/// An RGBA picture as lossless WebP bytes that [`check_webp`] accepts.
pub fn encode_webp(rgba: &[u8], size: (u32, u32)) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    image_webp::WebPEncoder::new(&mut out).encode(rgba, size.0, size.1, image_webp::ColorType::Rgba8).map_err(|e| Error::Game(format!("WebP: {e}")))?;
    check_webp(&out)?;
    Ok(out)
}

/// The size the portrait is kept at.
pub const PORTRAIT: (u32, u32) = (240, 320);

/// Takes a window picture (RGBA, `win` wide and tall), crops it with
/// [`crop_rect`] and returns the finished portrait WebP.
pub fn make(rgba: &[u8], win: (u32, u32)) -> Result<Vec<u8>> {
    let (cx, cy, cw, ch) = crop_rect(win).ok_or_else(|| Error::Game("the game window is too small for a portrait".into()))?;
    if rgba.len() != win.0 as usize * win.1 as usize * 4 {
        return Err(Error::Game("the window picture has the wrong size".into()));
    }
    let mut part = Vec::with_capacity((cw * ch * 4) as usize);
    for y in cy..cy + ch {
        let at = ((y * win.0 + cx) * 4) as usize;
        part.extend_from_slice(&rgba[at..at + (cw * 4) as usize]);
    }
    let to = (PORTRAIT.0.min(cw), PORTRAIT.1.min(ch));
    encode_webp(&downscale(&part, (cw, ch), to)?, to)
}

/// Most characters the card shows for one account.
pub const MAX_CHARACTERS: usize = 20;
/// Largest characters answer the launcher reads.
pub const MAX_ANSWER: usize = 64 * 1024;

/// The shared character record (also read by the in-game Character window).
/// Served at GET /ad/characters/mine as `{"characters":[...]}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Character {
    pub actor: u32,
    pub name: String,
    pub race: String,
    pub sex: String,
    pub hold: String,
    #[serde(rename = "playtimeMin")]
    pub playtime_min: u64,
    #[serde(rename = "lastSeen")]
    pub last_seen: u64,
    #[serde(default)]
    pub bio: String,
    #[serde(default)]
    pub portrait: Option<PortraitRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortraitRef {
    /// Bumps whenever the picture changes; the launcher refetches on a new one.
    pub version: u32,
    pub url: String,
    #[serde(rename = "takenAt")]
    pub taken_at: u64,
}

/// Where a character's portrait is served (the server's path, never taken
/// from the answer).
pub fn portrait_path(actor: u32) -> String {
    format!("/portrait/{actor}.webp")
}

/// Reads the server's characters answer. A record that doesn't fit (no
/// name, a portrait url that isn't that actor's own) is an error, not
/// something the card shows half-way.
pub fn characters_answer(body: &[u8]) -> Result<Vec<Character>> {
    #[derive(Deserialize)]
    struct Answer {
        characters: Vec<Character>,
    }
    if body.len() > MAX_ANSWER {
        return Err(Error::Game("the characters answer is too long".into()));
    }
    let list = serde_json::from_slice::<Answer>(body)?.characters;
    if list.len() > MAX_CHARACTERS {
        return Err(Error::Game("the characters answer lists too many characters".into()));
    }
    for c in &list {
        let name = c.name.trim();
        if name.is_empty() || name.chars().count() > 64 || c.bio.chars().count() > 280 {
            return Err(Error::Game(format!("character {} has a name or biography that doesn't fit", c.actor)));
        }
        if let Some(p) = &c.portrait {
            if p.url != format!("/ad{}", portrait_path(c.actor)) {
                return Err(Error::Game(format!("character {} points at a portrait that isn't its own", c.actor)));
            }
        }
    }
    Ok(list)
}

pub fn path(game_dir: &Path) -> PathBuf {
    game_dir.join(FOLDER).join(SELF_FILE)
}

/// Checks and saves the player's own portrait.
pub fn save_self(game_dir: &Path, body: &[u8]) -> Result<()> {
    check_webp(body)?;
    let path = path(game_dir);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    crate::atomicfile::write(&path, body)?;
    Ok(())
}

/// The saved portrait, when there is one and it still passes the checks.
pub fn load_self(game_dir: &Path) -> Option<Vec<u8>> {
    let body = std::fs::read(path(game_dir)).ok()?;
    check_webp(&body).ok()?;
    Some(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A lossless WebP header for `w` x `h`, padded to `len` bytes in all.
    fn webp(w: u32, h: u32, len: usize) -> Vec<u8> {
        let len = len.max(32);
        let mut v = b"RIFF".to_vec();
        v.extend_from_slice(&((len - 8) as u32).to_le_bytes());
        v.extend_from_slice(b"WEBPVP8L");
        v.extend_from_slice(&((len - 20) as u32).to_le_bytes());
        v.push(0x2f);
        v.extend_from_slice(&((w - 1) | ((h - 1) << 14)).to_le_bytes());
        v.resize(len, 0);
        v
    }

    #[test]
    fn a_normal_webp_is_accepted_with_its_size() {
        assert_eq!(check_webp(&webp(240, 320, 5000)).unwrap(), (240, 320));
    }

    #[test]
    fn lossy_and_extended_headers_are_read_too() {
        let mut lossy = b"RIFF".to_vec();
        lossy.extend_from_slice(&42u32.to_le_bytes());
        lossy.extend_from_slice(b"WEBPVP8 ");
        lossy.extend_from_slice(&30u32.to_le_bytes());
        lossy.extend_from_slice(&[0, 0, 0, 0x9d, 0x01, 0x2a]);
        lossy.extend_from_slice(&240u16.to_le_bytes());
        lossy.extend_from_slice(&320u16.to_le_bytes());
        lossy.resize(50, 0);
        assert_eq!(check_webp(&lossy).unwrap(), (240, 320));
        let mut ext = b"RIFF".to_vec();
        ext.extend_from_slice(&42u32.to_le_bytes());
        ext.extend_from_slice(b"WEBPVP8X");
        ext.extend_from_slice(&10u32.to_le_bytes());
        ext.extend_from_slice(&[0, 0, 0, 0]);
        ext.extend_from_slice(&[239, 0, 0]);
        ext.extend_from_slice(&[63, 1, 0]);
        ext.resize(50, 0);
        assert_eq!(check_webp(&ext).unwrap(), (240, 320));
    }

    #[test]
    fn other_formats_and_junk_are_refused() {
        assert!(check_webp(b"\xff\xd8\xff\xe0 jpeg").is_err());
        assert!(check_webp(b"\x89PNG\r\n\x1a\n and some more bytes to pass the length check").is_err());
        assert!(check_webp(b"").is_err());
        assert!(check_webp(b"RIFF\x00\x00\x00\x00WEBP").is_err());
        let mut wrong_chunk = webp(240, 320, 100);
        wrong_chunk[12..16].copy_from_slice(b"JUNK");
        assert!(check_webp(&wrong_chunk).is_err());
        let mut wrong_len = webp(240, 320, 100);
        wrong_len[4] = 7;
        assert!(check_webp(&wrong_len).is_err());
    }

    #[test]
    fn sizes_outside_the_limits_are_refused() {
        assert!(check_webp(&webp(63, 320, 100)).is_err());
        assert!(check_webp(&webp(240, 513, 100)).is_err());
        assert!(check_webp(&webp(64, 512, 100)).is_ok());
        assert!(check_webp(&webp(240, 320, MAX_BYTES + 1)).is_err());
        assert!(check_webp(&webp(240, 320, MAX_BYTES)).is_ok());
    }

    #[test]
    fn the_crop_is_upright_3_by_4_and_centred() {
        // 1920x1080: height limits it, 3:4 of 1080 tall is 810 wide.
        assert_eq!(crop_rect((1920, 1080)), Some((555, 0, 810, 1080)));
        // A window narrower than 3:4 of its height is limited by width.
        assert_eq!(crop_rect((600, 1000)), Some((0, 100, 600, 800)));
    }

    #[test]
    fn a_tiny_or_empty_window_gives_no_crop() {
        assert_eq!(crop_rect((0, 0)), None);
        assert_eq!(crop_rect((80, 60)), None);
    }

    #[test]
    fn grabs_are_spaced_out() {
        assert!(due(None, 1000));
        assert!(!due(Some(1000), 1000 + MIN_GAP_SECS - 1));
        assert!(due(Some(1000), 1000 + MIN_GAP_SECS));
        assert!(due(Some(5000), 100), "a clock that went backwards");
    }

    #[test]
    fn downscale_averages_blocks() {
        // 2x2 -> 1x1: the mean of four pixels.
        let src = [0, 0, 0, 255, 100, 200, 40, 255, 0, 0, 0, 255, 100, 200, 40, 255];
        assert_eq!(downscale(&src, (2, 2), (1, 1)).unwrap(), vec![50, 100, 20, 255]);
        assert!(downscale(&src, (2, 2), (3, 3)).is_err(), "no growing");
        assert!(downscale(&src[..8], (2, 2), (1, 1)).is_err());
    }

    #[test]
    fn a_window_picture_becomes_a_checked_portrait() {
        let (w, h) = (800u32, 600u32);
        let rgba = vec![90u8; (w * h * 4) as usize];
        let out = make(&rgba, (w, h)).unwrap();
        // 600 tall: 450 x 600 crop, shrunk to 240 x 320.
        assert_eq!(check_webp(&out).unwrap(), PORTRAIT);
        assert!(make(&rgba[..100], (w, h)).is_err());
        assert!(make(&[], (0, 0)).is_err());
    }

    #[test]
    fn save_then_load_round_trips_and_bad_files_are_not_saved() {
        let dir = tempfile::tempdir().unwrap();
        let good = webp(240, 320, 3000);
        save_self(dir.path(), &good).unwrap();
        assert_eq!(load_self(dir.path()).unwrap(), good);
        assert!(save_self(dir.path(), b"not a webp").is_err());
        assert_eq!(load_self(dir.path()).unwrap(), good, "the old portrait stays");
    }

    fn record(extra: &str) -> String {
        format!(r#"{{"actor":4101,"name":"Ysolda of Whiterun","race":"Nord","sex":"female","hold":"Whiterun","playtimeMin":2040,"lastSeen":1791400000000,"bio":"Came north.",{extra}}}"#)
    }

    #[test]
    fn the_shared_record_is_read_with_and_without_a_portrait() {
        let with = record(r#""portrait":{"version":7,"url":"/ad/portrait/4101.webp","takenAt":1791399000000}"#);
        let without = record(r#""portrait":null"#);
        let body = format!(r#"{{"characters":[{with},{without}]}}"#);
        let list = characters_answer(body.as_bytes()).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "Ysolda of Whiterun");
        assert_eq!(list[0].playtime_min, 2040);
        assert_eq!(list[0].portrait.as_ref().unwrap().version, 7);
        assert!(list[1].portrait.is_none());
        assert_eq!(portrait_path(4101), "/portrait/4101.webp");
    }

    #[test]
    fn a_record_that_does_not_fit_is_refused() {
        let other = record(r#""portrait":{"version":1,"url":"https://evil.example/x.webp","takenAt":1}"#);
        assert!(characters_answer(format!(r#"{{"characters":[{other}]}}"#).as_bytes()).is_err());
        let wrong_actor = record(r#""portrait":{"version":1,"url":"/ad/portrait/9.webp","takenAt":1}"#);
        assert!(characters_answer(format!(r#"{{"characters":[{wrong_actor}]}}"#).as_bytes()).is_err());
        let nameless = record(r#""portrait":null"#).replace("Ysolda of Whiterun", "  ");
        assert!(characters_answer(format!(r#"{{"characters":[{nameless}]}}"#).as_bytes()).is_err());
        assert!(characters_answer(b"{}").is_err());
        let many = vec![record(r#""portrait":null"#); MAX_CHARACTERS + 1].join(",");
        assert!(characters_answer(format!(r#"{{"characters":[{many}]}}"#).as_bytes()).is_err());
    }

    #[test]
    fn a_damaged_saved_file_is_not_shown() {
        let dir = tempfile::tempdir().unwrap();
        let p = path(dir.path());
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"junk").unwrap();
        assert!(load_self(dir.path()).is_none());
        assert!(load_self(tempfile::tempdir().unwrap().path()).is_none());
    }
}

/// Grabbing the running game's window (Windows only). Copies the screen
/// area the game window shows, so the game must be open, in front and not
/// minimised. Not covered by the cloud tests; `examples/portrait_grab.rs`
/// is the check to run on a real PC.
#[cfg(windows)]
pub mod grab {
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, GetClientRect, GetForegroundWindow, IsIconic};

    use crate::{Error, Result};

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    /// The game window's handle, found by its class, then by its title.
    fn find() -> Option<*mut core::ffi::c_void> {
        let name = wide("Skyrim Special Edition");
        [unsafe { FindWindowW(name.as_ptr(), null()) }, unsafe { FindWindowW(null(), name.as_ptr()) }].into_iter().find(|hwnd| !hwnd.is_null())
    }

    /// The game window's picture as RGBA and its size.
    pub fn window_rgba() -> Result<(Vec<u8>, (u32, u32))> {
        let fail = |why: &str| Error::Game(format!("can't grab the game window: {why}"));
        let hwnd = find().ok_or_else(|| fail("Skyrim isn't open"))?;
        unsafe {
            if IsIconic(hwnd) != 0 {
                return Err(fail("the game is minimised"));
            }
            if GetForegroundWindow() != hwnd {
                return Err(fail("the game isn't in front"));
            }
            let mut rect: RECT = std::mem::zeroed();
            let mut origin = POINT { x: 0, y: 0 };
            if GetClientRect(hwnd, &mut rect) == 0 || ClientToScreen(hwnd, &mut origin) == 0 {
                return Err(fail("its size can't be read"));
            }
            let (w, h) = ((rect.right - rect.left).max(0), (rect.bottom - rect.top).max(0));
            if w == 0 || h == 0 {
                return Err(fail("the window is empty"));
            }
            let screen = GetDC(null_mut());
            let mem = CreateCompatibleDC(screen);
            let bmp = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(mem, bmp);
            let copied = BitBlt(mem, 0, 0, w, h, screen, origin.x, origin.y, SRCCOPY | CAPTUREBLT);
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = w;
            info.bmiHeader.biHeight = -h; // top row first
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut px = vec![0u8; w as usize * h as usize * 4];
            let rows = if copied != 0 { GetDIBits(mem, bmp, 0, h as u32, px.as_mut_ptr().cast(), &mut info, DIB_RGB_COLORS) } else { 0 };
            SelectObject(mem, old);
            DeleteObject(bmp);
            DeleteDC(mem);
            ReleaseDC(null_mut(), screen);
            if rows == 0 {
                return Err(fail("the screen copy failed"));
            }
            // GDI gives BGRX; make it RGBA, opaque.
            for at in (0..px.len()).step_by(4) {
                px.swap(at, at + 2);
                px[at + 3] = 255;
            }
            Ok((px, (w as u32, h as u32)))
        }
    }

    /// Grabs the game window and returns the finished portrait WebP.
    pub fn portrait() -> Result<Vec<u8>> {
        let (rgba, size) = window_rgba()?;
        super::make(&rgba, size)
    }
}
