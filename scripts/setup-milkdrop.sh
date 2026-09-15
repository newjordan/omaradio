#!/bin/sh
# Bring up the MilkDrop collection for omaradio's `M` key:
#   1. libprojectM 4.1.7 built from source into ~/.local/lib
#   2. the "Cream of the Crop" preset pack (9,700+ community-rated presets)
#   3. the MilkDrop texture pack many presets reference
# Nothing is installed system-wide. Re-run with --force to rebuild/refetch.
set -eu

PM_TAG="${OMARADIO_PROJECTM_TAG:-v4.1.7}"
PREFIX="${OMARADIO_PREFIX:-$HOME/.local}"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}/omaradio"
FORCE=0
[ "${1:-}" = "--force" ] && FORCE=1

say() { printf '\033[1;36m» %s\033[0m\n' "$*"; }
die() { printf '\033[1;31m✗ %s\033[0m\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "need $1 ($2)"; }

need git "git"
need cmake "cmake"
need c++ "a C++ compiler (gcc or clang)"
[ -e /usr/lib/libEGL.so.1 ] || [ -e /usr/lib64/libEGL.so.1 ] || [ -e /usr/lib/x86_64-linux-gnu/libEGL.so.1 ] \
  || say "warning: libEGL.so.1 not found; projectM needs EGL + a GPU driver at runtime"

if [ "$FORCE" = 0 ] && [ -e "$PREFIX/lib/libprojectM-4.so" ]; then
  say "libprojectM already at $PREFIX/lib (use --force to rebuild)"
else
  WORK="$(mktemp -d)"
  trap 'rm -rf "$WORK"' EXIT
  say "cloning projectM $PM_TAG"
  git clone -q --depth 1 --branch "$PM_TAG" --recurse-submodules --shallow-submodules \
    https://github.com/projectM-visualizer/projectm.git "$WORK/src"
  say "building (this takes a few minutes)"
  cmake -S "$WORK/src" -B "$WORK/build" -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$PREFIX" -DENABLE_PLAYLIST=OFF -DENABLE_SDL_UI=OFF \
    -DBUILD_TESTING=OFF -DBUILD_SHARED_LIBS=ON >"$WORK/cmake.log" 2>&1 \
    || { tail -20 "$WORK/cmake.log"; die "cmake configure failed"; }
  cmake --build "$WORK/build" -j"$(nproc 2>/dev/null || echo 4)" >"$WORK/build.log" 2>&1 \
    || { tail -30 "$WORK/build.log"; die "build failed"; }
  cmake --install "$WORK/build" >/dev/null
  say "installed $PREFIX/lib/libprojectM-4.so"
fi

mkdir -p "$DATA"
fetch() { # name repo
  if [ "$FORCE" = 0 ] && [ -d "$DATA/$1" ]; then
    say "$1 already at $DATA/$1"
  else
    rm -rf "$DATA/$1"
    say "fetching $1"
    git clone -q --depth 1 "$2" "$DATA/$1"
    rm -rf "$DATA/$1/.git"
  fi
}
fetch cream-of-the-crop https://github.com/projectM-visualizer/presets-cream-of-the-crop.git
fetch textures https://github.com/projectM-visualizer/presets-milkdrop-texture-pack.git

N="$(find "$DATA/cream-of-the-crop" -name '*.milk' | wc -l | tr -d ' ')"
say "done: $N presets. In omaradio press M to cycle them, m for the built-ins."
say "override paths with OMARADIO_PROJECTM_LIB / OMARADIO_MILK_PRESETS if you keep them elsewhere."
