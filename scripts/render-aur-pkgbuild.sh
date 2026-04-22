#!/usr/bin/env bash
# Render the AUR -bin PKGBUILD (and matching .SRCINFO) from templates.
#
# Usage:
#   render-aur-pkgbuild.sh <version> <sha256sums-file> [<out-dir>]
#
# When <out-dir> is given, writes BOTH `PKGBUILD` and `.SRCINFO` into
# that directory (cd in, commit, `git push` to AUR). When omitted,
# only the PKGBUILD is printed to stdout.
#
# Generating .SRCINFO ourselves (instead of relying on `makepkg
# --printsrcinfo`) means non-Arch maintainers don't need a container
# or local Arch install to publish a release.
#
# `<sha256sums-file>` is the SHA256SUMS asset attached to every
# GitHub Release by the release workflow. Download it with:
#   gh release download vX.Y.Z -R leboiko/markdown-reader -p SHA256SUMS

set -euo pipefail

if [[ $# -lt 2 || $# -gt 3 ]]; then
  echo "usage: $0 <version> <sha256sums-file> [<out-dir>]" >&2
  exit 2
fi

VERSION="$1"
CHECKSUMS="$2"
OUT_DIR="${3:-}"

if [[ ! -f $CHECKSUMS ]]; then
  echo "error: checksums file not found: $CHECKSUMS" >&2
  exit 1
fi

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" &>/dev/null && pwd)"
PKGBUILD_TMPL="$SCRIPT_DIR/../packaging/aur/PKGBUILD-bin.tmpl"
SRCINFO_TMPL="$SCRIPT_DIR/../packaging/aur/SRCINFO-bin.tmpl"

for tmpl in "$PKGBUILD_TMPL" "$SRCINFO_TMPL"; do
  if [[ ! -f $tmpl ]]; then
    echo "error: template not found: $tmpl" >&2
    exit 1
  fi
done

sha_for() {
  local needle="$1"
  awk -v n="$needle" '$2 ~ n { print $1; exit }' "$CHECKSUMS"
}

BASE_URL="https://github.com/leboiko/markdown-reader/releases/download/v${VERSION}"

URL_X86_64_LINUX_GNU="${BASE_URL}/markdown-reader-${VERSION}-x86_64-unknown-linux-gnu.tar.gz"
URL_AARCH64_LINUX_GNU="${BASE_URL}/markdown-reader-${VERSION}-aarch64-unknown-linux-gnu.tar.gz"

SHA_X86_64_LINUX_GNU="$(sha_for "x86_64-unknown-linux-gnu.tar.gz")"
SHA_AARCH64_LINUX_GNU="$(sha_for "aarch64-unknown-linux-gnu.tar.gz")"

for var in SHA_X86_64_LINUX_GNU SHA_AARCH64_LINUX_GNU; do
  if [[ -z ${!var} ]]; then
    echo "error: could not find checksum for $var in $CHECKSUMS" >&2
    exit 1
  fi
done

render() {
  sed \
    -e "s|{{VERSION}}|${VERSION}|g" \
    -e "s|{{URL_X86_64_LINUX_GNU}}|${URL_X86_64_LINUX_GNU}|g" \
    -e "s|{{URL_AARCH64_LINUX_GNU}}|${URL_AARCH64_LINUX_GNU}|g" \
    -e "s|{{SHA256_X86_64_LINUX_GNU}}|${SHA_X86_64_LINUX_GNU}|g" \
    -e "s|{{SHA256_AARCH64_LINUX_GNU}}|${SHA_AARCH64_LINUX_GNU}|g" \
    "$1"
}

if [[ -n $OUT_DIR ]]; then
  mkdir -p "$OUT_DIR"
  render "$PKGBUILD_TMPL" >"$OUT_DIR/PKGBUILD"
  render "$SRCINFO_TMPL" >"$OUT_DIR/.SRCINFO"
  echo "wrote $OUT_DIR/PKGBUILD and $OUT_DIR/.SRCINFO" >&2
else
  render "$PKGBUILD_TMPL"
fi
