//! kitty RGB blit with real cell-pixel geometry for aspect-correct contain.

use std::io::{self, Write};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use flate2::write::ZlibEncoder;
use flate2::Compression;

const ID: u32 = 42;
const CHUNK: usize = 4096;
/// Fallback only, when the real cell size is unknown: terminals are typically
/// ~2× taller than wide in pixels.
const CELL_ASPECT_FALLBACK: f32 = 0.5;

/// Measured cell pixel geometry (cell_w, cell_h) via TIOCGWINSZ, when the
/// terminal reports it. Kitty and most modern terminals do.
/// Fallback cell pixels when ioctl does not report them (2:1 tall cells).
const CELL_PX_FALLBACK: (u16, u16) = (8, 16);

fn cell_pixels() -> Option<(u16, u16)> {
    let mut ws = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: ws is a plain struct; TIOCGWINSZ only fills it.
    if unsafe { libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) } == 0
        && ws.ws_xpixel > 0
        && ws.ws_ypixel > 0
        && ws.ws_col > 0
        && ws.ws_row > 0
    {
        Some((ws.ws_xpixel / ws.ws_col, ws.ws_ypixel / ws.ws_row))
    } else {
        None
    }
}

/// Cell pixel aspect as **width / height**. Terminal cells are typically
/// taller than wide, so this is ~0.48 (e.g. 14×29 → 14/29), matching
/// `CELL_ASPECT_FALLBACK`. `contain_cells` multiplies pane cols/rows by this
/// to recover the pane's pixel aspect for letterboxing.
fn cell_aspect() -> f32 {
    cell_aspect_from(cell_pixels())
}

fn cell_aspect_from(px: Option<(u16, u16)>) -> f32 {
    match px {
        Some((cw, ch)) if cw > 0 && ch > 0 => cw as f32 / ch as f32,
        _ => CELL_ASPECT_FALLBACK,
    }
}

/// Pane pixel box from character cols/rows × measured cell size.
/// Falls back to `CELL_PX_FALLBACK` only when ioctl geometry is unknown.
pub fn pane_pixel_size(cols: u16, rows: u16) -> (u32, u32) {
    pane_pixel_size_with(cols, rows, cell_pixels())
}

pub fn pane_pixel_size_with(cols: u16, rows: u16, cell: Option<(u16, u16)>) -> (u32, u32) {
    let (cw, ch) = match cell {
        Some((w, h)) if w > 0 && h > 0 => (w, h),
        _ => CELL_PX_FALLBACK,
    };
    (cols as u32 * cw as u32, rows as u32 * ch as u32)
}

pub fn available() -> bool {
    std::env::var_os("KITTY_WINDOW_ID").is_some()
        || std::env::var("TERM")
            .map(|t| t.contains("kitty"))
            .unwrap_or(false)
}

pub fn delete_all(out: &mut impl Write) -> io::Result<()> {
    write!(out, "\x1b_Ga=d,d=A\x1b\\")?;
    out.flush()
}

pub fn blit_rgb(
    out: &mut impl Write,
    rgb: &[u8],
    width: u32,
    height: u32,
    col: u16,
    row: u16,
    cols: u16,
    rows: u16,
) -> io::Result<()> {
    blit(out, rgb, width, height, col, row, cols, rows, false)
}

/// Letterbox into the pane so the full frame is visible (no stretch/crop).
pub fn blit_contain(
    out: &mut impl Write,
    rgb: &[u8],
    width: u32,
    height: u32,
    col: u16,
    row: u16,
    cols: u16,
    rows: u16,
) -> io::Result<()> {
    let (dc, dr) = contain_cells(width, height, cols, rows);
    if dc == 0 || dr == 0 {
        return Ok(());
    }
    let x = col.saturating_add(cols.saturating_sub(dc) / 2);
    let y = row.saturating_add(rows.saturating_sub(dr) / 2);
    blit(out, rgb, width, height, x, y, dc, dr, true)
}

fn contain_cells(img_w: u32, img_h: u32, cols: u16, rows: u16) -> (u16, u16) {
    contain_cells_with(img_w, img_h, cols, rows, cell_aspect())
}

fn contain_cells_with(
    img_w: u32,
    img_h: u32,
    cols: u16,
    rows: u16,
    cell_wh: f32,
) -> (u16, u16) {
    if cols == 0 || rows == 0 || img_w == 0 || img_h == 0 || cell_wh <= 0.0 {
        return (0, 0);
    }
    let img_aspect = img_w as f32 / img_h as f32;
    let pane_aspect = (cols as f32 / rows as f32) * cell_wh;
    if img_aspect > pane_aspect {
        let dr = ((cols as f32 * cell_wh / img_aspect).round() as u16).clamp(1, rows);
        (cols, dr)
    } else {
        let dc = ((rows as f32 * img_aspect / cell_wh).round() as u16).clamp(1, cols);
        (dc, rows)
    }
}

fn blit(
    out: &mut impl Write,
    rgb: &[u8],
    width: u32,
    height: u32,
    col: u16,
    row: u16,
    cols: u16,
    rows: u16,
    zlib: bool,
) -> io::Result<()> {
    if width == 0 || height == 0 || cols == 0 || rows == 0 {
        return Ok(());
    }
    write!(
        out,
        "\x1b[{};{}H",
        row.saturating_add(1),
        col.saturating_add(1)
    )?;
    let compressed;
    let raw: &[u8] = if zlib {
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::fast());
        enc.write_all(rgb)?;
        compressed = enc.finish()?;
        &compressed
    } else {
        rgb
    };
    let payload = STANDARD.encode(raw);
    let o = if zlib { ",o=z" } else { "" };
    let mut first = true;
    let mut rest = payload.as_str();
    while !rest.is_empty() {
        let take = rest.len().min(CHUNK);
        let (chunk, tail) = rest.split_at(take);
        rest = tail;
        let more = if rest.is_empty() { 0 } else { 1 };
        if first {
            write!(
                out,
                "\x1b_Ga=T,f=24,s={width},v={height},c={cols},r={rows},i={ID},q=2,C=1{o},m={more};{chunk}\x1b\\"
            )?;
            first = false;
        } else {
            write!(out, "\x1b_Gm={more};{chunk}\x1b\\")?;
        }
    }
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_aspect_is_width_over_height() {
        let a = cell_aspect_from(Some((14, 29)));
        assert!((a - 14.0 / 29.0).abs() < 1e-5, "got {a}");
        assert!((a - 0.48275862).abs() < 1e-5);
        assert_eq!(cell_aspect_from(None), CELL_ASPECT_FALLBACK);
        assert_eq!(cell_aspect_from(Some((0, 29))), CELL_ASPECT_FALLBACK);
    }

    fn pixel_box(img_w: u32, img_h: u32, cols: u16, rows: u16, cw: u16, ch: u16) -> (u32, u32) {
        let (dc, dr) = contain_cells_with(
            img_w,
            img_h,
            cols,
            rows,
            cell_aspect_from(Some((cw, ch))),
        );
        (dc as u32 * cw as u32, dr as u32 * ch as u32)
    }

    fn within_one_cell(got: u32, want: f32, cell: u16) {
        let d = (got as f32 - want).abs();
        assert!(
            d <= cell as f32 + 0.5,
            "got {got} want {want} cell {cell} delta {d}"
        );
    }

    #[test]
    fn contain_letterbox_landscape_portrait_resize_fallback() {
        let (pw, ph) = pixel_box(1920, 1080, 80, 24, 14, 29);
        let pane_w = 80.0 * 14.0;
        let want_h = pane_w * 1080.0 / 1920.0;
        within_one_cell(pw, pane_w, 14);
        within_one_cell(ph, want_h, 29);

        let (pw, ph) = pixel_box(1080, 1920, 80, 40, 14, 29);
        let pane_h = 40.0 * 29.0;
        let want_w = pane_h * 1080.0 / 1920.0;
        within_one_cell(ph, pane_h, 29);
        within_one_cell(pw, want_w, 14);

        let a = pixel_box(1920, 1080, 80, 24, 14, 29);
        let b = pixel_box(1920, 1080, 40, 12, 14, 29);
        assert!(b.0 <= a.0 && b.1 <= a.1);

        let fb = contain_cells_with(1920, 1080, 80, 24, CELL_ASPECT_FALLBACK);
        assert!(fb.0 > 0 && fb.1 > 0 && fb.0 <= 80 && fb.1 <= 24);
    }

    #[test]
    fn pane_pixels_use_ioctl_cells_and_fallback() {
        assert_eq!(pane_pixel_size_with(40, 12, Some((14, 29))), (560, 348));
        assert_eq!(pane_pixel_size_with(40, 12, None), (320, 192));
        let big = pane_pixel_size_with(80, 24, Some((14, 29)));
        let small = pane_pixel_size_with(40, 12, Some((14, 29)));
        assert!(small.0 <= big.0 && small.1 <= big.1);
    }
}
