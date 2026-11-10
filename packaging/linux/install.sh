#!/bin/sh
# Installs 3DCraft from this unpacked tarball for the current user (into
# ~/.local), or into PREFIX when given: ./install.sh [PREFIX]
# Every file of the tarball's bin/ and share/ goes to the same place below
# PREFIX: the programs, the fonts they read at run time
# (share/craft3d/fonts, found from bin/), the MIME types, the metainfo,
# the icons and the desktop entry. Nothing outside PREFIX is touched;
# uninstall.sh beside it removes the same files again.
set -eu
here="$(dirname "$(readlink -f "$0")")"
prefix="${1:-$HOME/.local}"

(cd "$here" && find bin share -type f ! -path 'share/applications/*' | sort) |
  while IFS= read -r rel; do
    case "$rel" in
      bin/*) mode=755 ;;
      *) mode=644 ;;
    esac
    install -Dm"$mode" "$here/$rel" "$prefix/$rel"
  done

command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$prefix/share/icons/hicolor" || true

# The desktop entry goes last: GNOME Shell reads it the moment it appears
# and remembers a missing icon if the icons are not there yet.
(cd "$here" && find share/applications -type f | sort) |
  while IFS= read -r rel; do
    install -Dm644 "$here/$rel" "$prefix/$rel"
  done

# Refresh the caches when the tools are there; a desktop picks up the files
# at its next login otherwise.
command -v update-mime-database >/dev/null 2>&1 && update-mime-database "$prefix/share/mime" || true
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$prefix/share/applications" || true

echo "3DCraft installed into $prefix (make sure $prefix/bin is on PATH)"
