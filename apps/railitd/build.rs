//! Build von railitd:
//!
//! 1. Übersetzt den gRPC-Vertrag `proto/railit/v1/railit.proto` (ohne `protoc`, mit protox).
//! 2. Bettet den Bedienplatz (`apps/railitd/ui`, gebaut mit `bun run build`) ein.
//!
//! Der gebaute Bedienplatz wird nicht eingecheckt; die CI und der Release-Workflow bauen ihn
//! vor railitd. Quelle ist `ui/dist` oder das Verzeichnis in `RAILIT_UI_DIST`. Fehlt er, bettet
//! der Build eine Hinweisseite ein und warnt – mit `RAILIT_REQUIRE_UI=1` (CI, Release) bricht
//! er stattdessen ab, damit nie ein railitd ohne Bedienplatz ausgeliefert wird.

use std::path::{Path, PathBuf};

const FILES: [&str; 3] = ["index.html", "app.js", "style.css"];

const PLACEHOLDER: &str = "<!doctype html>\n<html lang=\"de\"><head><meta charset=\"utf-8\">\
<title>rail-it</title><link rel=\"stylesheet\" href=\"style.css\"></head><body>\
<h1>Bedienplatz nicht gebaut</h1><p>Dieser railitd wurde ohne Bedienplatz gebaut. \
Bauen mit <code>cd apps/railitd/ui &amp;&amp; bun install &amp;&amp; bun run build</code>, \
danach railitd neu bauen. Die API unter <code>/v1/</code> steht vollständig zur Verfügung.</p>\
</body></html>\n";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    grpc()?;
    ui();
    Ok(())
}

/// gRPC-Vertrag.
fn grpc() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=../../proto");
    let fds = protox::compile(["railit/v1/railit.proto"], ["../../proto"])?;
    tonic_prost_build::configure()
        .build_client(true)
        // Clients wählen ihren Transport selbst (Tests, CLI).
        .build_transport(false)
        .build_server(true)
        .compile_fds(fds)?;
    Ok(())
}

/// Bedienplatz: gebaut aus `ui/dist` (bzw. `RAILIT_UI_DIST`), sonst Hinweisseite.
fn ui() {
    println!("cargo:rerun-if-env-changed=RAILIT_UI_DIST");
    println!("cargo:rerun-if-env-changed=RAILIT_REQUIRE_UI");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let source = std::env::var_os("RAILIT_UI_DIST")
        .map_or_else(|| manifest.join("ui").join("dist"), PathBuf::from);
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default()).join("ui");
    if let Err(e) = std::fs::create_dir_all(&out) {
        panic!("{}: {e}", out.display());
    }
    for file in FILES {
        println!("cargo:rerun-if-changed={}", source.join(file).display());
    }
    let built = FILES.iter().all(|f| source.join(f).is_file());
    if built {
        for file in FILES {
            copy(&source.join(file), &out.join(file));
        }
        return;
    }
    assert!(
        !std::env::var("RAILIT_REQUIRE_UI").is_ok_and(|v| v == "1"),
        "Bedienplatz fehlt in {} (RAILIT_REQUIRE_UI=1): erst `bun run build` in apps/railitd/ui",
        source.display()
    );
    println!(
        "cargo:warning=Bedienplatz nicht gebaut ({}): railitd zeigt unter /ui/ eine Hinweisseite. \
         Bauen mit `cd apps/railitd/ui && bun install && bun run build`.",
        source.display()
    );
    write(&out.join("index.html"), PLACEHOLDER);
    write(&out.join("app.js"), "// Bedienplatz nicht gebaut\n");
    write(
        &out.join("style.css"),
        "body { font-family: sans-serif; margin: 2rem; }\n",
    );
}

fn copy(from: &Path, to: &Path) {
    if let Err(e) = std::fs::copy(from, to) {
        panic!("{} → {}: {e}", from.display(), to.display());
    }
}

fn write(path: &Path, text: &str) {
    if let Err(e) = std::fs::write(path, text) {
        panic!("{}: {e}", path.display());
    }
}
