#!/bin/sh
# rail-it – Installation, Aktualisierung und Deinstallation (Linux, macOS).
#
#   curl -fsSL https://josunlp.github.io/rail-it/install.sh | sh
#   curl -fsSL https://josunlp.github.io/rail-it/install.sh | sh -s -- --version 0.11.0
#   curl -fsSL https://josunlp.github.io/rail-it/install.sh | sh -s -- update
#   curl -fsSL https://josunlp.github.io/rail-it/install.sh | sh -s -- uninstall
#
# Nach der Installation genügen `railit self update` und `railit self uninstall`.
#
# Befehle:   install (Vorgabe), update, uninstall, help
# Optionen:  --version X.Y.Z   bestimmte Version statt der neuesten
#            --prefix DIR      Programme nach DIR/bin, Daten nach DIR/share/rail-it
#            --force           auch installieren, wenn die Version schon installiert ist
#            --no-modify-path  keinen Hinweis zum PATH ausgeben
# Umgebung:  RAILIT_PREFIX, RAILIT_VERSION, RAILIT_REPO (Vorgabe JosunLP/rail-it),
#            RAILIT_DOWNLOAD_URL (Basis statt https://github.com/$RAILIT_REPO/releases)
#
# Vorgabe für `--prefix`: ~/.local, als root /usr/local. Jede heruntergeladene Datei wird
# gegen SHA256SUMS des Releases geprüft. Das Manifest DIR/share/rail-it/manifest hält fest,
# welche Dateien installiert sind; update und uninstall richten sich danach.
set -eu

REPO=${RAILIT_REPO:-JosunLP/rail-it}
BASE=${RAILIT_DOWNLOAD_URL:-https://github.com/$REPO/releases}
BINARIES="railit railitd railit-field railit-node"

say() { printf 'rail-it: %s\n' "$*"; }
fail() { printf 'rail-it: Fehler: %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<'HILFE'
rail-it installieren, aktualisieren und entfernen (Linux, macOS)

  install.sh [install|update|uninstall] [--version X.Y.Z] [--prefix DIR] [--force]

  install    neueste (oder mit --version bestimmte) Version installieren (Vorgabe)
  update     auf die neueste Version aktualisieren
  uninstall  alle installierten Dateien entfernen
  --prefix   Programme nach DIR/bin, Daten nach DIR/share/rail-it
             (Vorgabe ~/.local, als root /usr/local)
  --force    auch installieren, wenn die Version schon installiert ist

Umgebung: RAILIT_PREFIX, RAILIT_VERSION, RAILIT_REPO, RAILIT_DOWNLOAD_URL
HILFE
}

default_prefix() {
  if [ "$(id -u 2>/dev/null || echo 1)" = "0" ]; then echo /usr/local; else echo "$HOME/.local"; fi
}

target() {
  os=$(uname -s)
  arch=$(uname -m)
  case "$arch" in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) fail "Prozessor $arch wird nicht unterstützt (x86_64, aarch64)" ;;
  esac
  case "$os" in
    Linux) echo "$arch-unknown-linux-musl" ;;
    Darwin) echo "$arch-apple-darwin" ;;
    *) fail "Betriebssystem $os wird nicht unterstützt (Windows: install.ps1)" ;;
  esac
}

# fetch URL ZIEL
fetch() {
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --proto '=https,http,file' --retry 3 -o "$2" "$1" || return 1
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$2" "$1" || return 1
  else
    fail "curl oder wget wird benötigt"
  fi
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d ' ' -f 1
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | cut -d ' ' -f 1
  else
    fail "sha256sum oder shasum wird zur Prüfung benötigt"
  fi
}

# Wert eines Schlüssels im Manifest.
manifest_get() {
  sed -n "s/^$1=//p" "$2" 2>/dev/null | head -n 1
}

# url VERSION DATEI
url() {
  if [ -n "$1" ]; then echo "$BASE/download/v$1/$2"; else echo "$BASE/latest/download/$2"; fi
}

install_release() {
  prefix=$1 version=$2 force=$3 hint=$4
  tgt=$(target)
  archive="railit-$tgt.tar.gz"
  bindir="$prefix/bin"
  data="$prefix/share/rail-it"
  manifest="$data/manifest"

  tmp=$(mktemp -d 2>/dev/null || mktemp -d -t railit)
  trap 'rm -rf "$tmp"' EXIT INT TERM

  say "lade ${version:-neueste Version} für $tgt"
  fetch "$(url "$version" "$archive")" "$tmp/$archive" ||
    fail "Download fehlgeschlagen: $(url "$version" "$archive")"
  fetch "$(url "$version" SHA256SUMS)" "$tmp/SHA256SUMS" ||
    fail "Prüfsummen fehlen: $(url "$version" SHA256SUMS)"
  expected=$(awk -v f="$archive" '$2 == f || $2 == "*" f { print $1 }' "$tmp/SHA256SUMS")
  [ -n "$expected" ] || fail "keine Prüfsumme für $archive"
  actual=$(sha256 "$tmp/$archive")
  [ "$expected" = "$actual" ] || fail "Prüfsumme von $archive stimmt nicht ($actual ≠ $expected)"

  tar -xzf "$tmp/$archive" -C "$tmp"
  stage="$tmp/railit-$tgt"
  [ -f "$stage/VERSION" ] || fail "Archiv ohne VERSION"
  new=$(cat "$stage/VERSION")
  old=$(manifest_get version "$manifest")
  if [ "$old" = "$new" ] && [ "$force" != 1 ]; then
    say "rail-it $new ist schon installiert ($bindir)"
    return 0
  fi

  mkdir -p "$bindir" "$data"
  files=""
  for bin in $BINARIES; do
    [ -f "$stage/$bin" ] || fail "Archiv ohne $bin"
    # Erst daneben ablegen, dann umbenennen: laufende Programme bleiben unberührt.
    cp "$stage/$bin" "$bindir/.$bin.new"
    chmod 755 "$bindir/.$bin.new"
    mv -f "$bindir/.$bin.new" "$bindir/$bin"
    files="$files $bindir/$bin"
  done
  rm -rf "$data/examples"
  cp -R "$stage/examples" "$data/examples"
  for doc in LICENSE README.md CHANGELOG.md VERSION; do
    [ -f "$stage/$doc" ] && cp "$stage/$doc" "$data/$doc"
  done
  if [ -f "$stage/install.sh" ]; then
    cp "$stage/install.sh" "$data/install.sh"
    chmod 755 "$data/install.sh"
  fi
  {
    echo "version=$new"
    echo "target=$tgt"
    echo "prefix=$prefix"
    echo "bindir=$bindir"
    echo "data=$data"
    echo "repo=$REPO"
    for f in $files; do echo "file=$f"; done
  } > "$manifest.new"
  mv -f "$manifest.new" "$manifest"

  if [ -n "$old" ]; then
    say "rail-it $old → $new aktualisiert ($bindir)"
  else
    say "rail-it $new installiert: $BINARIES nach $bindir"
    say "Beispielnetze: $data/examples"
  fi
  if [ "$hint" = 1 ]; then
    case ":$PATH:" in
      *":$bindir:"*) ;;
      *) say "Hinweis: $bindir steht nicht im PATH – z. B. in ~/.profile: export PATH=\"$bindir:\$PATH\"" ;;
    esac
  fi
}

uninstall() {
  prefix=$1
  data="$prefix/share/rail-it"
  manifest="$data/manifest"
  [ -f "$manifest" ] || fail "keine Installation unter $prefix gefunden (Manifest $manifest fehlt)"
  version=$(manifest_get version "$manifest")
  sed -n 's/^file=//p' "$manifest" | while IFS= read -r f; do
    [ -n "$f" ] && rm -f "$f"
  done
  rm -rf "$data"
  say "rail-it $version entfernt"
}

main() {
  command=install
  prefix=${RAILIT_PREFIX:-}
  version=${RAILIT_VERSION:-}
  force=0
  hint=1
  while [ $# -gt 0 ]; do
    case "$1" in
      install | update | uninstall) command=$1 ;;
      help | -h | --help) usage; exit 0 ;;
      --version) shift; version=${1:?--version braucht eine Version} ;;
      --version=*) version=${1#--version=} ;;
      --prefix) shift; prefix=${1:?--prefix braucht ein Verzeichnis} ;;
      --prefix=*) prefix=${1#--prefix=} ;;
      --force) force=1 ;;
      --no-modify-path) hint=0 ;;
      *) fail "unbekannte Angabe: $1 (help zeigt die Hilfe)" ;;
    esac
    shift
  done
  version=${version#v}
  if [ -z "$prefix" ]; then
    # Bei update und uninstall: die bestehende Installation finden.
    prefix=$(default_prefix)
    if [ "$command" != install ]; then
      for p in "$HOME/.local" /usr/local; do
        if [ -f "$p/share/rail-it/manifest" ]; then prefix=$p; break; fi
      done
    fi
  fi
  case "$command" in
    install) install_release "$prefix" "$version" "$force" "$hint" ;;
    update)
      [ -f "$prefix/share/rail-it/manifest" ] || fail "keine Installation unter $prefix – erst installieren"
      install_release "$prefix" "$version" "$force" 0
      ;;
    uninstall) uninstall "$prefix" ;;
  esac
}

main "$@"
