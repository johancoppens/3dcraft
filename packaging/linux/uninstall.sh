#!/bin/sh
# Removes what install.sh put into ~/.local, or into PREFIX: ./uninstall.sh [PREFIX]
# Documents and autosaves are left alone.
set -eu
prefix="${1:-$HOME/.local}"
id=io.github.johancoppens.craft3d

rm -f "$prefix/bin/craft3d" "$prefix/bin/craft3d-cli" \
  "$prefix/share/applications/$id.desktop" \
  "$prefix/share/mime/packages/$id.xml" \
  "$prefix/share/metainfo/$id.metainfo.xml" \
  "$prefix"/share/icons/hicolor/*/apps/$id.*

command -v update-mime-database >/dev/null 2>&1 && update-mime-database "$prefix/share/mime" || true
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$prefix/share/applications" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$prefix/share/icons/hicolor" || true
echo "3DCraft removed from $prefix"
