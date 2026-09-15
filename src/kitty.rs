//! Kitty graphics protocol — RGB blit for the milkdrop feed.

use std::io::{self, Write};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

const ID: u32 = 42;
const CHUNK: usize = 4096;

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
    if width == 0 || height == 0 || cols == 0 || rows == 0 {
        return Ok(());
    }
    write!(out, "\x1b[{};{}H", row.saturating_add(1), col.saturating_add(1))?;
    let payload = STANDARD.encode(rgb);
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
                "\x1b_Ga=T,f=24,s={width},v={height},c={cols},r={rows},i={ID},q=2,C=1,m={more};{chunk}\x1b\\"
            )?;
            first = false;
        } else {
            write!(out, "\x1b_Gm={more};{chunk}\x1b\\")?;
        }
    }
    out.flush()
}
