//! Fitting the launcher window to the screen it opens on. The window has no
//! frame, so a window taller than the screen can't be moved or shrunk by the
//! player, and the PLAY button sits along its bottom edge. On a 1080p screen
//! at 150% scaling the usable area is about 1280x690, less than the old
//! 1360x880 start size and the old 720 minimum height. The window is also
//! centred in that area, not on the whole screen, which would put its bottom
//! edge under a taskbar.

/// The smallest the page still lays out in (measured in the page, logical px).
pub const MIN: (f64, f64) = (1024.0, 560.0);

/// The start size in logical px for a screen whose usable area (without the
/// taskbar) is `work`: `want` where it fits, else shrunk to the usable area,
/// never below [`MIN`] (a screen that small can't show the launcher whole).
pub fn fit(want: (f64, f64), work: (f64, f64)) -> (f64, f64) {
    let side = |want: f64, work: f64, min: f64| want.min(work).max(min);
    (side(want.0, work.0, MIN.0), side(want.1, work.1, MIN.1))
}

/// Where to put a window of `size` so it sits in the middle of the usable
/// area `work` (x, y, width, height) and never under the taskbar. Monitors
/// left of or above the main one have negative x or y.
pub fn centre_in(work: (f64, f64, f64, f64), size: (f64, f64)) -> (f64, f64) {
    let (x, y, w, h) = work;
    (x + ((w - size.0) / 2.0).max(0.0), y + ((h - size.1) / 2.0).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_big_screen_gets_the_full_size() {
        assert_eq!(fit((1360.0, 880.0), (2560.0, 1400.0)), (1360.0, 880.0));
    }

    #[test]
    fn a_1080p_screen_at_150_percent_fits_the_window_above_the_taskbar() {
        // 1920x1080 at 150% is 1280x720; the taskbar takes about 32 of that.
        assert_eq!(fit((1360.0, 880.0), (1280.0, 688.0)), (1280.0, 688.0));
    }

    #[test]
    fn a_768p_laptop_at_125_percent_fits_too() {
        // 1366x768 at 125% is about 1093x614, less a 32 taskbar.
        assert_eq!(fit((1360.0, 880.0), (1092.8, 582.4)), (1092.8, 582.4));
    }

    #[test]
    fn a_window_as_tall_as_the_usable_area_starts_at_its_top() {
        // Centring on the whole 720 screen would push the bottom 16 under the taskbar.
        assert_eq!(centre_in((0.0, 0.0, 1280.0, 688.0), (1280.0, 688.0)), (0.0, 0.0));
        assert_eq!(centre_in((0.0, 0.0, 1920.0, 1040.0), (1360.0, 880.0)), (280.0, 80.0));
    }

    #[test]
    fn a_monitor_left_of_the_main_one_keeps_the_window_on_it() {
        assert_eq!(centre_in((-1920.0, 0.0, 1920.0, 1040.0), (1360.0, 880.0)), (-1640.0, 80.0));
        // A taskbar at the top moves the usable area down.
        assert_eq!(centre_in((0.0, 40.0, 1280.0, 680.0), (1280.0, 680.0)), (0.0, 40.0));
    }

    #[test]
    fn a_tiny_screen_never_goes_below_the_minimum() {
        assert_eq!(fit((1360.0, 880.0), (800.0, 500.0)), MIN);
    }
}
