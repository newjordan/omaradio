<p align="center">
  <img src="art/shots/iss.png" alt="omaradio on Mission Control with the live NASA ISS earth view" width="960">
</p>

<p align="center">
  <strong>Night-dial terminal radio.</strong> Small TUI. mpv under the hood.<br>
    CrabMusic braille bars, a live oscilloscope, a Winamp-style milkdrop,<br>
    plus the live ISS earth view. Built for Omarchy.
</p>

<p align="center">
  <code>cargo install --path . --force && omaradio</code>
</p>

---

<p align="center">
  <img src="art/shots/bars.png" alt="CrabMusic braille spectrum bars driven by the live mix" width="400">
  <img src="art/shots/wave.png" alt="oscilloscope" width="400">
</p>

<p align="center">
  <img src="art/shots/milkdrop-kaleido.png" alt="milkdrop preset: neon kaleido" width="400">
  <img src="art/shots/milkdrop-rain.png" alt="milkdrop preset: drift rain" width="400">
</p>

<p align="center">
  <img src="art/shots/cinema.png" alt="cinema mode: full-window milkdrop" width="820">
</p>

Shots are stock Kitty on Omarchy, 1080p, Berkeley Mono. The bars, wave and
milkdrop are reading the real PipeWire sink monitor, not a mood oscillator.

Needs **mpv** on `PATH`. Kitty graphics for milkdrop / ISS (cell
fallback otherwise). PipeWire `pw-record` for the live FFT. ISS live also
wants **yt-dlp** + **ffmpeg**.

```bash
# Omarchy / Arch
sudo pacman -S mpv
git clone https://github.com/newjordan/omaradio.git
cd omaradio
cargo install --path . --force
omaradio
```

Or from the crate without installing:

```bash
cargo run --release
```

## Dial

Default tune is Mission Control, with the ISS earth view up.

| # | Station | Broadcaster | Why it's there |
|---|---------|-------------|----------------|
| 1 | Old Time Radio | [WALM Radio](https://walmradio.com) | Golden-age drama and mystery |
| 2 | Liquid DnB | Liquid DnB | Rolling liquid funk |
| 3 | Space Station | [SomaFM](https://somafm.com/spacestation/) | Deep-space ambient |
| 4 | Mission Control | [SomaFM](https://somafm.com/missioncontrol/) | NASA comms over pads |
| 5 | Jazz Groove | [The Jazz Groove](https://www.thejazzgroove.com) | After-midnight slow jazz, no chatter |
| 6 | Midnight Blues | [Jazz Radio](https://www.jazzradio.fr) | Slow blues after hours |
| 7 | WWOZ New Orleans | [WWOZ 90.7 FM](https://www.wwoz.org) | Live from the Quarter |

Streams belong to the broadcasters. This is an unofficial tuner. Full credits
live in **[ATTRIBUTION.md](ATTRIBUTION.md)** and on the in-app `?` card.

## Keys

| Key | Action |
|-----|--------|
| `↑↓` / `jk` | Select |
| `Enter` / `l` | Play |
| `space` | Pause |
| `+/-` `←→` | Volume |
| `[]` `n` `p` | Previous / next and play |
| `1-7` | Jump |
| `v` | Cycle viz: bars → wave → milkdrop → ISS |
| `f` | Full-window viz (milkdrop / ISS) |
| `m` | Next milkdrop preset (8 presets; they also rotate on the beat) |
| `s` | Stop |
| `?` / `h` | Credits |
| `q` | Quit |

## How it works

- **Playback** is [mpv](https://mpv.io) over JSON IPC — ICY titles, Soma `.pls`
  failover, volume. We spawn mpv; we do not link it.
- **Bars / wave** are a real FFT + oscilloscope of the PipeWire default-sink
  monitor, drawn with [CrabMusic](https://github.com/newjordan/crabmusic)'s
  braille columns + peak gravity, MIT © 2025 Frosty40.
- **Milkdrop** is a software feedback visualizer (eight presets, bass onset
  detection, auto-rotation) blitted with the
  [Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/).
- **ISS** is NASA's public HD earth-view livestream (audio stays on the radio).
- **Desktop** target is [Omarchy](https://omarchy.org).

## License

omaradio is [MIT](LICENSE) © 2026 Frosty40.

CrabMusic's MIT notice is preserved in
[`third_party/CRABMUSIC_LICENSE`](third_party/CRABMUSIC_LICENSE). mpv remains
GPL-2.0-or-later as a separate program you install yourself.

Not affiliated with SomaFM, WWOZ, WALM, The Jazz Groove, Jazz Radio, Omarchy,
mpv, or NASA. SomaFM® is a trademark of SomaFM. ISS imagery remains NASA's
public stream — we only display it.
