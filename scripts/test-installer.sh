#!/bin/sh
# Prüft install.sh gegen ein lokales Release: installieren, erneut installieren,
# aktualisieren, `railit self`, entfernen.
#
#   sh scripts/test-installer.sh dist/railit-<Ziel>.tar.gz VERSION
#
# Das Archiv wird wie ein GitHub-Release unter http://127.0.0.1:<Port>/ angeboten
# (download/vVERSION/… und latest/download/…), der Installer mit RAILIT_DOWNLOAD_URL
# darauf gelenkt. Ohne Archiv baut das Skript eines mit Platzhalterprogrammen.
set -eu

archive=${1:-}
version=${2:-0.0.0-test}
root=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
server=""
cleanup() {
  if [ -n "$server" ]; then kill "$server" 2>/dev/null || true; fi
  rm -rf "$work"
}
trap cleanup EXIT INT TERM

# Ziel wie im Installer.
arch=$(uname -m)
case "$arch" in x86_64 | amd64) arch=x86_64 ;; aarch64 | arm64) arch=aarch64 ;; esac
case "$(uname -s)" in Darwin) target="$arch-apple-darwin" ;; *) target="$arch-unknown-linux-musl" ;; esac
name="railit-$target.tar.gz"

if [ -z "$archive" ]; then
  stage="$work/stage/railit-$target"
  mkdir -p "$stage/examples"
  for bin in railit railitd railit-field railit-node; do
    printf '#!/bin/sh\necho "%s %s"\n' "$bin" "$version" > "$stage/$bin"
    chmod 755 "$stage/$bin"
  done
  cp "$root/LICENSE" "$root/README.md" "$root/scripts/install.sh" "$stage/"
  cp "$root"/examples/*.toml "$stage/examples/"
  echo "$version" > "$stage/VERSION"
  (cd "$work/stage" && tar -czf "$work/$name" "railit-$target")
  archive="$work/$name"
  real=0
else
  real=1
fi

for dir in "download/v$version" latest/download; do
  mkdir -p "$work/rel/$dir"
  cp "$archive" "$work/rel/$dir/$name"
  (cd "$work/rel/$dir" && { sha256sum "$name" 2>/dev/null || shasum -a 256 "$name"; } > SHA256SUMS)
done
port=$((20000 + $$ % 20000))
(cd "$work/rel" && exec python3 -m http.server "$port" --bind 127.0.0.1 >/dev/null 2>&1) &
server=$!
i=0
until curl -fs "http://127.0.0.1:$port/latest/download/SHA256SUMS" >/dev/null 2>&1; do
  i=$((i + 1)); [ $i -lt 50 ] || { echo "HTTP-Server startet nicht" >&2; exit 1; }
  sleep 0.1
done
export RAILIT_DOWNLOAD_URL="http://127.0.0.1:$port"
prefix="$work/prefix"
install="$root/scripts/install.sh"

check() { if ! "$@"; then echo "FEHLGESCHLAGEN: $*" >&2; exit 1; fi; }
out() { "$@" 2>&1 || { echo "FEHLGESCHLAGEN: $*" >&2; exit 1; }; }

out sh "$install" --prefix "$prefix" --no-modify-path | tee "$work/log"
check grep -q "$version installiert" "$work/log"
for bin in railit railitd railit-field railit-node; do check test -x "$prefix/bin/$bin"; done
check test -f "$prefix/share/rail-it/manifest"
check test -f "$prefix/share/rail-it/examples/beispielstadt.toml"
check grep -q "^version=$version$" "$prefix/share/rail-it/manifest"

out sh "$install" --prefix "$prefix" | tee "$work/log"
check grep -q "schon installiert" "$work/log"
out sh "$install" update --prefix "$prefix" --version "$version" --force | tee "$work/log"
check grep -q "aktualisiert" "$work/log"

if [ "$real" = 1 ]; then
  out "$prefix/bin/railit" --version
  out "$prefix/bin/railit" self info | tee "$work/log"
  check grep -q "Installiert über den Installer" "$work/log"
  out "$prefix/bin/railit" self update
  out "$prefix/bin/railit" net validate "$prefix/share/rail-it/examples/beispielstadt.toml"
  out "$prefix/bin/railit" self uninstall
else
  out sh "$install" uninstall --prefix "$prefix"
fi
for bin in railit railitd railit-field railit-node; do check test ! -e "$prefix/bin/$bin"; done
check test ! -e "$prefix/share/rail-it"

# Verfälschte Prüfsumme wird abgewiesen.
sed 's/^./0/' "$work/rel/latest/download/SHA256SUMS" > "$work/sums" && mv "$work/sums" "$work/rel/latest/download/SHA256SUMS"
if sh "$install" --prefix "$prefix" >/dev/null 2>&1; then
  echo "FEHLGESCHLAGEN: verfälschte Prüfsumme angenommen" >&2
  exit 1
fi
check test ! -e "$prefix/bin/railit"
echo "Installer: alle Prüfungen bestanden ($target, $version)"
