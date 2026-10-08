//! Character portrait (look-into 2026-10-08, Timothy picked the "portrait
//! snapshot": a small picture of the real character shown on the launcher's
//! character card). The launcher grabs the game window shortly after
//! RaceMenu closes, crops the middle of it to a head-and-shoulders
//! portrait, and keeps it as one PNG next to the other launcher state. Only
//! this module's checks decide what is kept or ever sent anywhere: PNG
//! only, small, sensible dimensions.

use std::path::{Path, PathBuf};

use crate::{Error, Result};

pub const FOLDER: &str = ".aetherial-dawn/portrait";
pub const SELF_FILE: &str = "self.png";
/// Largest portrait kept or sent.
pub const MAX_BYTES: usize = 256 * 1024;
/// Side limits in pixels.
pub const MIN_SIDE: u32 = 64;
pub const MAX_SIDE: u32 = 512;
/// Shortest wait between two grabs, so a player opening and closing
/// RaceMenu repeatedly doesn't make the launcher grab every time.
pub const MIN_GAP_SECS: u64 = 30;

/// Width and height of a PNG, or an error when `body` isn't one the launcher
/// keeps (not a PNG, too big, sides outside [`MIN_SIDE`]..=[`MAX_SIDE`]).
pub fn check_png(body: &[u8]) -> Result<(u32, u32)> {
    let bad = |why: &str| Error::Game(format!("the portrait isn't kept: {why}"));
    if body.len() > MAX_BYTES {
        return Err(bad("too big"));
    }
    if body.len() < 24 || &body[..8] != b"\x89PNG\r\n\x1a\n" || &body[12..16] != b"IHDR" {
        return Err(bad("not a PNG"));
    }
    let side = |at: usize| u32::from_be_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]]);
    let (w, h) = (side(16), side(20));
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

/// An RGBA picture as PNG bytes that [`check_png`] accepts.
pub fn encode_png(rgba: &[u8], size: (u32, u32)) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, size.0, size.1);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().map_err(|e| Error::Game(format!("PNG: {e}")))?;
        w.write_image_data(rgba).map_err(|e| Error::Game(format!("PNG: {e}")))?;
    }
    check_png(&out)?;
    Ok(out)
}

/// The size the portrait is kept at.
pub const PORTRAIT: (u32, u32) = (240, 320);

/// Takes a window picture (RGBA, `win` wide and tall), crops it with
/// [`crop_rect`] and returns the finished portrait PNG.
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
    encode_png(&downscale(&part, (cw, ch), to)?, to)
}

pub fn path(game_dir: &Path) -> PathBuf {
    game_dir.join(FOLDER).join(SELF_FILE)
}

/// Checks and saves the player's own portrait.
pub fn save_self(game_dir: &Path, body: &[u8]) -> Result<()> {
    check_png(body)?;
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
    check_png(&body).ok()?;
    Some(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A PNG signature plus an IHDR chunk header for `w` x `h`, padded to `len`.
    fn png(w: u32, h: u32, len: usize) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.resize(len.max(v.len()), 0);
        v
    }

    #[test]
    fn a_normal_png_is_accepted_with_its_size() {
        assert_eq!(check_png(&png(240, 320, 5000)).unwrap(), (240, 320));
    }

    #[test]
    fn other_formats_and_junk_are_refused() {
        assert!(check_png(b"\xff\xd8\xff\xe0 jpeg").is_err());
        assert!(check_png(b"").is_err());
        assert!(check_png(b"\x89PNG\r\n\x1a\n").is_err());
        let mut wrong_chunk = png(240, 320, 100);
        wrong_chunk[12..16].copy_from_slice(b"IDAT");
        assert!(check_png(&wrong_chunk).is_err());
    }

    #[test]
    fn sizes_outside_the_limits_are_refused() {
        assert!(check_png(&png(63, 320, 100)).is_err());
        assert!(check_png(&png(240, 513, 100)).is_err());
        assert!(check_png(&png(0, 0, 100)).is_err());
        assert!(check_png(&png(64, 512, 100)).is_ok());
        assert!(check_png(&png(240, 320, MAX_BYTES + 1)).is_err());
        assert!(check_png(&png(240, 320, MAX_BYTES)).is_ok());
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
        assert_eq!(check_png(&out).unwrap(), PORTRAIT);
        assert!(make(&rgba[..100], (w, h)).is_err());
        assert!(make(&[], (0, 0)).is_err());
    }

    #[test]
    fn save_then_load_round_trips_and_bad_files_are_not_saved() {
        let dir = tempfile::tempdir().unwrap();
        let good = png(240, 320, 3000);
        save_self(dir.path(), &good).unwrap();
        assert_eq!(load_self(dir.path()).unwrap(), good);
        assert!(save_self(dir.path(), b"not a png").is_err());
        assert_eq!(load_self(dir.path()).unwrap(), good, "the old portrait stays");
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

    /// Grabs the game window and returns the finished portrait PNG.
    pub fn portrait() -> Result<Vec<u8>> {
        let (rgba, size) = window_rgba()?;
        super::make(&rgba, size)
    }
}
