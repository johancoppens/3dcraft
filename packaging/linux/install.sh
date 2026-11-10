#!/bin/sh
# Installs 3DCraft from this unpacked tarball for the current user (into
# ~/.local), or into PREFIX: ./install.sh [PREFIX]
# It installs the files share/craft3d/files names, each to the same path
# below PREFIX: the programs, the fonts they read at run time
# (share/craft3d/fonts, found from bin/), the MIME types, the metainfo,
# the icons, the licences (share/doc/craft3d), that list, and the desktop
# entry last. Only names that are 3DCraft's are installed, and nothing
# outside PREFIX is touched; uninstall.sh removes the same files again.
# Besides POSIX sh it needs readlink -f and install -D (GNU coreutils or
# BusyBox).
set -eu
# A CDPATH from the environment would make the cd below print and go elsewhere.
unset CDPATH
id=io.github.johancoppens.craft3d
usage="usage: ./install.sh [PREFIX]   (PREFIX defaults to ~/.local)"

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
list="$here/share/craft3d/files"

refuse() {
  printf '%s\n' "install.sh: $1; nothing was installed." >&2
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
put() {
  case "$1" in
    bin/*) mode=755 ;;
    *) mode=644 ;;
  esac
  install -D -m "$mode" -- "$here/$1" "$prefix/$1"
}

if [ ! -d "$here/bin" ] || [ ! -d "$here/share" ] || [ ! -f "$here/bin/craft3d" ] ||
  [ ! -f "$here/share/applications/$id.desktop" ] || [ ! -f "$list" ]; then
  refuse "$here is not an unpacked 3DCraft tarball (its bin/craft3d, desktop entry or share/craft3d/files is missing): run the install.sh that came in the tarball, in the folder it was unpacked into"
fi
p="$(real "$prefix")"
if [ -n "$p" ] && [ "$p" = "$(real "$here")" ]; then
  refuse "the tarball is unpacked into PREFIX itself ($p): unpack it into a folder of its own and run its install.sh there"
fi

count=0
while IFS= read -r rel; do
  [ -n "$rel" ] || continue
  if ! ours "$rel"; then
    printf '%s\n' "install.sh: skipped $rel: not a name of 3DCraft's" >&2
    continue
  fi
  case "$rel" in
    share/applications/*) continue ;;
  esac
  put "$rel"
  count=$((count + 1))
done <"$list"
[ "$count" -gt 0 ] || refuse "share/craft3d/files names nothing of 3DCraft's"

if [ -d "$prefix/share/icons/hicolor" ] && command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q -t "$prefix/share/icons/hicolor" >/dev/null 2>&1 || true
fi

# The desktop entry goes last: GNOME Shell reads it the moment it appears
# and remembers a missing icon if the icons are not there yet.
while IFS= read -r rel; do
  case "$rel" in
    share/applications/*)
      if ours "$rel"; then
        put "$rel"
        count=$((count + 1))
      fi
      ;;
  esac
done <"$list"

# Refresh the caches when the tools are there; a desktop picks up the files
# at its next login otherwise.
if [ -d "$prefix/share/mime" ] && command -v update-mime-database >/dev/null 2>&1; then
  update-mime-database "$prefix/share/mime" >/dev/null 2>&1 || true
fi
if [ -d "$prefix/share/applications" ] && command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$prefix/share/applications" >/dev/null 2>&1 || true
fi

printf '%s\n' "3DCraft installed into $prefix ($count files; make sure $prefix/bin is on PATH)"
