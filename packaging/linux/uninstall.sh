#!/bin/sh
# Removes what install.sh beside it put into ~/.local, or into PREFIX:
# ./uninstall.sh [PREFIX]. Every file of this tarball's bin/ and share/ is
# removed from there, and 3DCraft's own folder of fonts once it is empty.
# Documents and autosaves are left alone.
set -eu
here="$(dirname "$(readlink -f "$0")")"
prefix="${1:-$HOME/.local}"

(cd "$here" && find bin share -type f) |
  while IFS= read -r rel; do
    rm -f "$prefix/$rel"
  done
(cd "$here" && find share/craft3d -depth -type d) |
  while IFS= read -r rel; do
    rmdir "$prefix/$rel" 2>/dev/null || true
  done

command -v update-mime-database >/dev/null 2>&1 && update-mime-database "$prefix/share/mime" || true
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$prefix/share/applications" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$prefix/share/icons/hicolor" || true
echo "3DCraft removed from $prefix"
