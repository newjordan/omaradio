# Attributions

omaradio is an **unofficial** night-dial. It is not affiliated with, endorsed
by, or a product of SomaFM, WALM Radio, The Jazz Groove, Jazz Radio, WWOZ,
All Classical Radio, SRG SSR, projectM,
Omarchy, mpv, Kitty, or any other named project or broadcaster. Streams, names,
and trademarks remain their owners'. Support the stations — listen, donate,
buy the merch.

Press `?` in the app for the same card.

## Broadcasters

The dial only tunes public internet streams. We do not host, proxy, or
re-encode them. If a station asks to be removed, remove it.

| Dial | Broadcaster | Station / channel | Homepage | Stream used |
|------|-------------|-------------------|----------|-------------|
| Old Time Radio | WALM Radio | WALM — Old Time Radio | https://walmradio.com | `https://icecast.walmradio.com:8443/otr` |
| Liquid DnB | Liquid DnB (community) | Liquid DnB | https://antares.dribbcast.com/proxy/dave1/stream/ | same |
| Space Station | SomaFM | Space Station Soma | https://somafm.com/spacestation/ | `https://somafm.com/spacestation.pls` |
| Mission Control | SomaFM | Mission Control | https://somafm.com/missioncontrol/ | `https://somafm.com/missioncontrol.pls` |
| Jazz Groove | The Jazz Groove | East mix (MP3 128) | https://www.thejazzgroove.com | `http://east-mp3-128.streamthejazzgroove.com/stream` |
| Midnight Blues | Jazz Radio | Blues channel | https://www.jazzradio.fr | `http://jazzblues.ice.infomaniak.ch/jazzblues-high.mp3` |
| WWOZ New Orleans | New Orleans Public Radio | WWOZ 90.7 FM | https://www.wwoz.org | `https://wwoz-sc.streamguys1.com/wwoz-hi.mp3` |
| All Classical Portland | All Classical Radio (KQAC) | All Classical | https://www.allclassical.org | `https://allclassical.streamguys1.com/ac128kmp3` |
| Radio Swiss Classic | SRG SSR | Radio Swiss Classic | https://www.radioswissclassic.ch | `https://stream.srg-ssr.ch/m/rsc_de/mp3_128` |

**SomaFM®** is a trademark of SomaFM. Space Station Soma and Mission Control
are SomaFM channels. Listen on the official site too: https://somafm.com

**WWOZ** is the New Orleans jazz & heritage station. They live on listener
support: https://www.wwoz.org

**WALM Radio** programs Old Time Radio from public-domain golden-age shows:
https://walmradio.com

**The Jazz Groove** is an independent slow-jazz stream with no DJ chatter:
https://www.thejazzgroove.com

**Jazz Radio** (France) publishes a Blues channel via Infomaniak Icecast:
https://www.jazzradio.fr

**Liquid DnB** is a community liquid-funk stream served from Dribbcast /
Antares. It is not a SomaFM channel.

Playlist (`.pls`) files are fetched from the broadcaster and played by mpv,
including SomaFM's Icecast failover list.

## Radio Browser

`AGENTS.md` shows agents how to find streams through the community directory
at https://www.radio-browser.info. omaradio itself never calls it; the dial is
a local JSON file.

## Visual language — CrabMusic

The braille spectrum bars, peak-hold gravity, and frequency-band coloring
are adapted from **CrabMusic**, an ASCII music visualizer:

- Project: https://github.com/newjordan/crabmusic
- License: MIT
- Copyright (c) 2025 Frosty40

The MIT notice from CrabMusic is included in
[`third_party/CRABMUSIC_LICENSE`](third_party/CRABMUSIC_LICENSE). omaradio
reimplements a small slice of that visual language (Unicode braille 2×4
columns, attack/release smoothing, peak gravity) and drives it from a live
PipeWire sink-monitor FFT.

## Playback — mpv

Audio is **mpv** talking JSON IPC (`--input-ipc-server`). omaradio does not
link libmpv.

- Project: https://mpv.io
- Source: https://github.com/mpv-player/mpv
- License: GPL-2.0-or-later (mpv itself)

Install mpv from your OS. On Omarchy / Arch: `pacman -S mpv`.

## Capture — PipeWire

Spectrum, oscilloscope, and milkdrop read the default sink monitor via
`pw-record` (PipeWire). That is the speaker mix.

- https://pipewire.org

## Kitty graphics

The milkdrop blit uses the [Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/).
Kitty is Kovid Goyal's terminal. omaradio is not a Kitty product. Other
terminals get a cell-level plasma fallback.

## Milkdrop / Winamp

The fullscreen visualizer is an original software feedback renderer in the
spirit of MilkDrop (Ryan Geiss / Winamp). It is not MilkDrop, not projectM,
and not Nullsoft. No preset files are loaded.


## MilkDrop collection — projectM and preset authors

`M` uses **projectM** (https://github.com/projectM-visualizer/projectm), the
open-source MilkDrop engine, LGPL-2.1-or-later. omaradio does not link it:
`scripts/setup-milkdrop.sh` builds it into your `~/.local` and the app loads
it with `dlopen` at runtime when you ask for the collection. It stays a
separate, replaceable library; omaradio remains MIT.

The presets are projectM's **Cream of the Crop** pack
(https://github.com/projectM-visualizer/presets-cream-of-the-crop) and the
**MilkDrop texture pack**
(https://github.com/projectM-visualizer/presets-milkdrop-texture-pack). Each
`.milk` file is the work of its named author (Geiss, Flexi, martin, suksma,
Rovastar, Zylot, and hundreds more); the packs are redistributed by the
projectM project. They are not part of omaradio, are fetched by you, and the
preset's file name (the author's name) is shown while it plays.

MilkDrop itself is Ryan Geiss / Nullsoft; projectM is a reimplementation.

## Earth from space — NASA ISS

ISS mode defaults to NASA's official ISS live camera (no tracker HUD).
`c` cycles NASA HD truss cams, then Sen 4K (that one has overlays).

NASA imagery is generally public domain in the US. The optional third camera
is **Sen**'s 4K livestream (https://sen.com), which is Sen's copyright, shown
only when you cycle to it. omaradio is not a NASA or Sen product. We do not
rehost the streams; the ISS cameras are fetched from YouTube through yt-dlp on
your machine.

## Desktop — Omarchy

Built to live in a tiling terminal on **Omarchy**, an Arch-based desktop.

- https://omarchy.org

## Rust crates

Resolved versions live in `Cargo.lock`. License identifiers as published
on crates.io:

| Crate | Use | License |
|-------|-----|---------|
| [ratatui](https://ratatui.rs) | TUI widgets, layout, color | MIT |
| [crossterm](https://github.com/crossterm-rs/crossterm) | raw mode, keys, alternate screen | MIT |
| [serde](https://serde.rs) / [serde_json](https://github.com/serde-rs/json) | mpv IPC payloads | MIT OR Apache-2.0 |
| [anyhow](https://github.com/dtolnay/anyhow) | error reporting | MIT OR Apache-2.0 |
| [rustfft](https://github.com/ejmahler/RustFFT) | live spectrum | MIT OR Apache-2.0 |
| [base64](https://docs.rs/base64) | Kitty graphics payloads | MIT OR Apache-2.0 |

Unicode **Braille Patterns** (U+2800–U+28FF) are used for the spectrum and
oscilloscope. They are part of the Unicode Standard.

## Protocols and infrastructure

Stations typically speak HTTP(S) Icecast / SHOUTcast with ICY metadata.
SomaFM's public Icecast nodes and `.pls` playlists, Infomaniak Icecast
(Jazz Radio), StreamGuys (WWOZ), WALM Icecast, Dribbcast, and The Jazz
Groove's CDN are their infrastructure — not ours.

Stream discovery for the first dial used the community
[radio-browser.info](https://www.radio-browser.info) catalog to confirm
working URLs. The app does not call that API at runtime.

## NASA / Mission Control

SomaFM's Mission Control mixes NASA mission audio with ambient music.
NASA media is generally NASA-owned; SomaFM's program is SomaFM's. This
app only tunes SomaFM's published stream.

## What we are not claiming

- We do not rebroadcast, archive, or sell the audio.
- We do not use official SomaFM, WWOZ, Jazz Radio, WALM, Jazz Groove,
  Omarchy, or Kitty logos or brand assets.
- Station names in the dial are used to identify the stream you are about
  to hear.
- If you ship a fork, keep this file, `LICENSE`, and
  `third_party/CRABMUSIC_LICENSE`.
