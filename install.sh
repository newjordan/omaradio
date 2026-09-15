#!/bin/sh
# omaradio one-shot installer.
#
#   curl -fsSL https://raw.githubusercontent.com/newjordan/omaradio/main/install.sh | sh
#   curl -fsSL …/install.sh | sh -s -- --milkdrop      # also the MilkDrop collection
#   ./install.sh                                         # from a checkout
#
# Installs to ~/.cargo/bin/omaradio. Needs Rust (offers rustup if missing),
# mpv, and for the live FFT PipeWire's pactl/pw-record; ISS view wants
# yt-dlp + ffmpeg. Never touches system packages without asking.
set -eu

REPO="${OMARADIO_REPO:-https://github.com/newjordan/omaradio.git}"
MILKDROP=0
YES=0
for a in "$@"; do
  case "$a" in
    --milkdrop) MILKDROP=1 ;;
    --yes|-y) YES=1 ;;
    -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
    *) printf 'unknown option %s\n' "$a" >&2; exit 2 ;;
  esac
done

say() { printf '\033[1;36m» %s\033[0m\n' "$*"; }
die() { printf '\033[1;31m✗ %s\033[0m\n' "$*" >&2; exit 1; }
ask() { # question -> 0 yes / 1 no
  [ "$YES" = 1 ] && return 0
  printf '%s [y/N] ' "$1"
  if [ -r /dev/tty ]; then read -r r </dev/tty; else r=n; fi
  case "$r" in y|Y|yes) return 0 ;; *) return 1 ;; esac
}

missing=""
for bin in mpv pactl pw-record yt-dlp ffmpeg; do
  command -v "$bin" >/dev/null 2>&1 || missing="$missing $bin"
done
if [ -n "$missing" ]; then
  say "missing:$missing"
  if command -v pacman >/dev/null 2>&1; then
    pk="mpv pipewire-pulse yt-dlp ffmpeg"
    ask "install with: sudo pacman -S --needed $pk ?" && sudo pacman -S --needed $pk
  elif command -v apt-get >/dev/null 2>&1; then
    pk="mpv pipewire-audio pulseaudio-utils yt-dlp ffmpeg"
    ask "install with: sudo apt-get install -y $pk ?" && sudo apt-get install -y $pk
  elif command -v dnf >/dev/null 2>&1; then
    pk="mpv pipewire-pulseaudio yt-dlp ffmpeg"
    ask "install with: sudo dnf install -y $pk ?" && sudo dnf install -y $pk
  else
    say "install these with your package manager: $missing"
  fi
fi
command -v mpv >/dev/null 2>&1 || die "mpv is required"

if ! command -v cargo >/dev/null 2>&1; then
  if [ -x "$HOME/.cargo/bin/cargo" ]; then
    PATH="$HOME/.cargo/bin:$PATH"
  elif ask "Rust is not installed. Install it with rustup (https://rustup.rs)?"; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
    PATH="$HOME/.cargo/bin:$PATH"
  else
    die "cargo is required (https://rustup.rs)"
  fi
fi

if [ -f Cargo.toml ] && grep -q '^name = "omaradio"' Cargo.toml; then
  SRC="$(pwd)"
else
  SRC="$(mktemp -d)/omaradio"
  say "cloning $REPO"
  git clone -q --depth 1 "$REPO" "$SRC"
fi
say "building omaradio (release)"
cargo install --path "$SRC" --locked --force --quiet
BIN="$HOME/.cargo/bin/omaradio"
[ -x "$BIN" ] || die "install failed"
say "installed $BIN ($("$BIN" --version))"

case ":$PATH:" in
  *":$HOME/.cargo/bin:"*) ;;
  *) say "add ~/.cargo/bin to your PATH (rustup usually does this on next login)" ;;
esac
# A stale copy elsewhere on PATH would shadow the fresh one.
found="$(command -v omaradio 2>/dev/null || true)"
if [ -n "$found" ] && [ "$found" != "$BIN" ]; then
  say "warning: $found comes first on PATH and is not the one just installed"
fi

if [ "$MILKDROP" = 1 ]; then
  sh "$SRC/scripts/setup-milkdrop.sh"
fi

say "run:   omaradio"
say "agent: omaradio ctl help   ·   docs: $SRC/AGENTS.md"
