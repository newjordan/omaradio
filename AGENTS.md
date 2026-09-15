# omaradio for agents

omaradio is a terminal radio you can drive from outside. A running instance
listens on a Unix socket and answers newline-delimited JSON. The bundled
client, `omaradio ctl`, wraps it. Everything a human can do with keys, an
agent can do with one line, plus editing the dial.

## Install

    curl -fsSL https://raw.githubusercontent.com/newjordan/omaradio/main/install.sh | sh
    # or, for the MilkDrop collection too (builds projectM, ~5 min):
    curl -fsSL https://raw.githubusercontent.com/newjordan/omaradio/main/install.sh | sh -s -- --milkdrop

Runtime deps: `mpv` (required), PipeWire `pactl`/`pw-record` (live FFT),
`yt-dlp` + `ffmpeg` (ISS view). The radio must be running in a terminal for
`ctl` to have something to talk to:

    omaradio

## Talk to it

    omaradio ctl [--json] <command> [args]

| command | does |
|---|---|
| `status` | one JSON object with station, title, transport, volume, viz, tap health, dial size |
| `stations` | the dial: `n`, `id`, `name`, `url`, `mood`, `playing` |
| `tune <id\|n\|name\|url>` | tune by id (`swiss`), 1-based number, name fragment (`jazz groove`), or any http(s) stream |
| `play` `pause` `toggle` `stop` | transport |
| `next` `prev` | move along the dial and play |
| `volume <0-130\|+n\|-n>` | absolute or relative |
| `viz <bars\|wave\|milkdrop\|iss>` | pick a visualizer |
| `preset [next\|builtin\|collection\|<name>]` | milkdrop: next preset, built-in engine, projectM collection, or jump to a collection preset by name fragment |
| `add <id> <name> <url> [--source S] [--blurb B] [--homepage H] [--mood M] [--tune]` | add or replace a station in `stations.json` and reload |
| `remove <id>` | drop a station |
| `reload` | re-read `stations.json` after editing it by hand |
| `quit` | exit the app |

Replies are JSON. `ok: false` comes with an `error` string and exit code 1.
`--json` prints one compact line instead of pretty output.

Raw socket, if you would rather not shell out:

    $XDG_RUNTIME_DIR/omaradio.ctl.sock      (override: OMARADIO_CTL_SOCKET)
    → {"cmd":"tune","station":"swiss"}\n
    ← {"ok":true,"station":{"id":"swiss",...},"now_playing":"...",...}\n

One request per line, one reply per line, keep the connection open for more.
Request shapes: `{"cmd":"status"}`, `{"cmd":"volume","delta":-10}`,
`{"cmd":"volume","value":55}`, `{"cmd":"viz","kind":"milkdrop"}`,
`{"cmd":"preset","action":"collection"}`,
`{"cmd":"add","station":{"id":"kexp","name":"KEXP","url":"https://…","mood":"brass"},"tune":true}`.
Unknown `cmd` values are rejected with the list of valid ones.

## Status object

```json
{
  "ok": true, "version": "0.2.0", "protocol": 1,
  "station": {"id": "mission", "n": 4, "name": "Mission Control", "source": "SomaFM", "url": "…"},
  "selected": "mission",
  "now_playing": "Steve Roach - Endorphin Dreamtime",
  "paused": false, "idle": false, "volume": 70.0,
  "viz": "iss", "fullscreen": false,
  "milkdrop": {"engine": "builtin", "preset": "liquid tunnel", "collection": null, "collection_error": null},
  "tap": {"sink": "alsa_output…", "backend": "pacat", "rms": 0.044, "peak_bar": 20, "fft_ok": true, "error": ""},
  "iss": {"state": "live", "detail": "", "source": "ISS Earth viewing · NASA ISS live"},
  "stations": 9,
  "config": "/home/you/.config/omaradio/stations.json",
  "status": "Mission Control  ·  Steve Roach - Endorphin Dreamtime"
}
```

`tap.fft_ok` true with `rms` above ~0.01 means the visualizers are really
hearing the mix. `station` is `null` when nothing is tuned; a stream tuned by
url has `id: "url"` and `n: null`.

## Finding music for someone

The dial is a JSON file. Find a stream, add it, tune it, keep it if they like
it. Radio Browser (https://www.radio-browser.info, free, community-run) is the
usual directory; identify yourself with a User-Agent and be gentle with it.

    # what do they have on now?
    omaradio ctl status | jq '.station.name, .now_playing'

    # find calm classical streams, most-voted first
    curl -s -A 'omaradio-agent/1' \
      'https://de1.api.radio-browser.info/json/stations/search?tag=classical&order=votes&reverse=true&limit=10' \
      | jq -r '.[] | select(.lastcheckok==1) | "\(.votes)\t\(.name)\t\(.codec) \(.bitrate)\t\(.url_resolved)"'

    # try one without committing it to the dial
    omaradio ctl tune https://stream.srg-ssr.ch/m/rsc_de/mp3_128

    # they like it: put it on the dial and play it
    omaradio ctl add swiss "Radio Swiss Classic" https://stream.srg-ssr.ch/m/rsc_de/mp3_128 \
      --source "SRG SSR" --blurb "slow, peaceful classical, no talk" --mood classical --tune

Station `mood` steers the visualizer colours: `voice liquid space mission jazz
blues brass classical`. `id` is `[A-Za-z0-9_-]`. Prefer `url_resolved` from
Radio Browser and stations with `lastcheckok == 1`; `.pls`/`.m3u` playlist
urls are fine, mpv resolves them.

## The stations file

`~/.config/omaradio/stations.json` (or `$OMARADIO_CONFIG_DIR/stations.json`).
Absent → the built-in dial. Present → it *is* the dial. The first `add`
writes the built-ins plus yours, so nothing is lost. Hand edits need
`omaradio ctl reload` or a restart.

```json
{ "stations": [
  { "id": "swiss", "name": "Radio Swiss Classic", "url": "https://stream.srg-ssr.ch/m/rsc_de/mp3_128",
    "blurb": "slow, peaceful classical", "source": "SRG SSR",
    "homepage": "https://www.radioswissclassic.ch", "mood": "classical", "accent": [170, 200, 255] }
] }
```

Only `id`, `name`, `url` are required.

## Visuals

`viz` cycles bars → wave → milkdrop → ISS (NASA's live earth camera). Milkdrop
has eight built-in presets (`m`) and, when `scripts/setup-milkdrop.sh` has
run, the projectM "Cream of the Crop" collection of ~9,800 community-rated
MilkDrop presets (`M`, or `preset collection`). `status.milkdrop.collection`
tells you whether it is up and on which GPU; `collection_error` says why not.
Env: `OMARADIO_PROJECTM_LIB`, `OMARADIO_MILK_PRESETS`, `OMARADIO_EGL_DEVICE`.

## Etiquette

- Streams belong to their broadcasters. Do not proxy, record, or re-host.
- Radio Browser asks for a real User-Agent and no hammering.
- Do not run `omaradio --prove-tap` while someone is listening; it plays a
  test tone through their speakers (it refuses if it sees a running radio).
- Warnings from mpv/projectM go to `$XDG_RUNTIME_DIR/omaradio.log`.
