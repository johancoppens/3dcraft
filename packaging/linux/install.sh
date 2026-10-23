#!/bin/sh
# Installs 3DCraft from this unpacked tarball for the current user (into
# ~/.local), or into PREFIX when given: ./install.sh [PREFIX]
# Nothing outside PREFIX is touched; uninstall.sh beside it removes the
# same files again.
set -eu
here="$(dirname "$(readlink -f "$0")")"
prefix="${1:-$HOME/.local}"
id=io.github.johancoppens.craft3d

install -Dm755 "$here/bin/craft3d" "$prefix/bin/craft3d"
install -Dm755 "$here/bin/craft3d-cli" "$prefix/bin/craft3d-cli"
install -Dm644 "$here/share/mime/packages/$id.xml" "$prefix/share/mime/packages/$id.xml"
install -Dm644 "$here/share/metainfo/$id.metainfo.xml" "$prefix/share/metainfo/$id.metainfo.xml"
for icon in "$here"/share/icons/hicolor/*/apps/$id.*; do
  rel="${icon#"$here"/}"
  install -Dm644 "$icon" "$prefix/$rel"
done

command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$prefix/share/icons/hicolor" || true

# The desktop entry goes last: GNOME Shell reads it the moment it appears
# and remembers a missing icon if the icons are not there yet.
install -Dm644 "$here/share/applications/$id.desktop" "$prefix/share/applications/$id.desktop"

# Refresh the caches when the tools are there; a desktop picks up the files
# at its next login otherwise.
command -v update-mime-database >/dev/null 2>&1 && update-mime-database "$prefix/share/mime" || true
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$prefix/share/applications" || true

echo "3DCraft installed into $prefix (make sure $prefix/bin is on PATH)"
