#!/bin/sh
# Removes 3DCraft from ~/.local, or from PREFIX: ./uninstall.sh [PREFIX]
# It removes the files install.sh installed, as the list it installed
# names them (PREFIX/share/craft3d/files), or, when PREFIX has none, as
# the list of the 3DCraft tarball beside this script names them; only
# names that are 3DCraft's, then 3DCraft's own folders once they are
# empty. Documents, autosaves and every other file are left alone.
# Besides POSIX sh it needs readlink -f (GNU coreutils or BusyBox).
set -eu
# A CDPATH from the environment would make the cd below print and go elsewhere.
unset CDPATH
id=io.github.johancoppens.craft3d
usage="usage: ./uninstall.sh [PREFIX]   (PREFIX defaults to ~/.local)"

case "${1:-}" in
  -h | --help)
    echo "$usage"
    exit 0
    ;;
  -*)
    echo "$usage" >&2
    exit 2
    ;;
esac
if [ "$#" -gt 1 ]; then
  echo "$usage" >&2
  exit 2
fi
here="$(dirname "$(readlink -f "$0")")"
prefix="${1:-$HOME/.local}"

refuse() {
  printf '%s\n' "uninstall.sh: $1; nothing was removed." >&2
  exit 1
}
real() {
  (cd "$1" 2>/dev/null && pwd -P) || true
}
# The paths that are 3DCraft's; anything else in the list is skipped.
ours() {
  case "$1" in
    /* | *..*) return 1 ;;
    bin/craft3d | bin/craft3d-cli) return 0 ;;
    share/craft3d/* | share/doc/craft3d/*) return 0 ;;
    share/applications/"$id".desktop) return 0 ;;
    share/mime/packages/"$id".xml) return 0 ;;
    share/metainfo/"$id".metainfo.xml) return 0 ;;
    share/icons/hicolor/*/apps/"$id".png | share/icons/hicolor/*/apps/"$id".svg) return 0 ;;
    share/icons/hicolor/*/mimetypes/"$id"-jig.png | share/icons/hicolor/*/mimetypes/"$id"-jig.svg) return 0 ;;
    share/icons/hicolor/*/mimetypes/"$id"-asm.png | share/icons/hicolor/*/mimetypes/"$id"-asm.svg) return 0 ;;
  esac
  return 1
}

p="$(real "$prefix")"
[ -n "$p" ] || refuse "there is no folder $prefix"
if [ "$p" = "$(real "$here")" ]; then
  refuse "this script lies in PREFIX itself ($p), as when the tarball was unpacked straight into it: copy uninstall.sh out of PREFIX and run the copy, which removes what the list in PREFIX names"
fi
list="$prefix/share/craft3d/files"
if [ ! -f "$list" ]; then
  if [ -f "$here/bin/craft3d" ] && [ -f "$here/share/applications/$id.desktop" ] &&
    [ -f "$here/share/craft3d/files" ]; then
    list="$here/share/craft3d/files"
  else
    refuse "$prefix has no list of 3DCraft's files (share/craft3d/files), and there is no 3DCraft tarball beside this script to take one from: is 3DCraft installed there?"
  fi
fi

count=0
while IFS= read -r rel; do
  [ -n "$rel" ] || continue
  if ! ours "$rel"; then
    printf '%s\n' "uninstall.sh: skipped $rel: not a name of 3DCraft's" >&2
    continue
  fi
  if [ -f "$prefix/$rel" ] || [ -L "$prefix/$rel" ]; then
    rm -f -- "$prefix/$rel"
    count=$((count + 1))
  fi
  # 3DCraft's own folders, once the last of their files is gone.
  case "$rel" in
    share/craft3d/* | share/doc/craft3d/*)
      dir="$(dirname -- "$rel")"
      while [ "$dir" != share ] && [ "$dir" != share/doc ] && [ "$dir" != . ]; do
        rmdir -- "$prefix/$dir" 2>/dev/null || true
        dir="$(dirname -- "$dir")"
      done
      ;;
  esac
done <"$list"
[ "$count" -gt 0 ] || refuse "none of the files the list names is in $prefix"

if [ -d "$prefix/share/mime" ] && command -v update-mime-database >/dev/null 2>&1; then
  update-mime-database "$prefix/share/mime" >/dev/null 2>&1 || true
fi
if [ -d "$prefix/share/applications" ] && command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$prefix/share/applications" >/dev/null 2>&1 || true
fi
if [ -d "$prefix/share/icons/hicolor" ] && command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q -t "$prefix/share/icons/hicolor" >/dev/null 2>&1 || true
fi
printf '%s\n' "3DCraft removed from $prefix ($count files)"
