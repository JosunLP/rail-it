#!/bin/sh
# Packt die Release-Binärdateien eines Ziels: dist/railit-<Ziel>.tar.gz bzw. .zip.
#
#   sh scripts/package.sh VERSION ZIEL [tar.gz|zip] [BINÄRVERZEICHNIS]
#
# Das Archiv enthält das Verzeichnis railit-<Ziel>/ mit railit, railitd, railit-field und
# railit-node, den Beispielnetzen, Lizenz, README, CHANGELOG und VERSION. Die Namen der
# Archive tragen keine Version, damit .../releases/latest/download/<Archiv> immer das
# neueste Release liefert; die Version steht in VERSION.
set -eu

version=${1:?Version fehlt}
target=${2:?Ziel fehlt}
archive=${3:-tar.gz}
bindir=${4:-target/$target/release}

ext=""
case "$target" in *windows*) ext=".exe" ;; esac

name="railit-$target"
stage="dist/$name"
rm -rf "$stage"
mkdir -p "$stage/examples"
for bin in railit railitd railit-field railit-node; do
  cp "$bindir/$bin$ext" "$stage/"
done
cp LICENSE README.md "$stage/"
cp scripts/install.sh scripts/install.ps1 "$stage/"
[ -f CHANGELOG.md ] && cp CHANGELOG.md "$stage/"
cp examples/*.toml examples/*.ppxml "$stage/examples/"
printf '%s\n' "$version" > "$stage/VERSION"

cd dist
case "$archive" in
  tar.gz) tar -czf "$name.tar.gz" "$name" ;;
  zip)
    if command -v 7z >/dev/null 2>&1; then
      7z a -tzip "$name.zip" "$name" >/dev/null
    else
      zip -qr "$name.zip" "$name"
    fi
    ;;
  *) echo "unbekanntes Archivformat: $archive" >&2; exit 2 ;;
esac
echo "dist/$name.$archive"
