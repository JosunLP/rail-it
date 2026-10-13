//! Erzeugt große Netze für Leistungsmessung und Lasttests (Konzept, Abschnitt 15).
//!
//! [`corridor`] reiht Bahnhöfe nach dem Vorbild der Beispielstadt aneinander: je Bahnhof zwei
//! Weichen und zwei Bahnsteiggleise, Einfahr- und Ausfahrsignale in beiden Richtungen, ein
//! Streckengleis zum nächsten Bahnhof; jeder `lx_every`-te Abschnitt hat einen Bahnübergang.
//!
//! ```text
//!  GW ═ L0 ═ W0a ╤═ S0_1 ═╤ W0b ═ L1 ═ W1a ╤ … ╤ W(n-1)b ═ Ln ═ GE
//!                ╘═ S0_2 ═╛
//! ```

use std::fmt::Write as _;

/// Netz im rail-it-TOML mit `stations` Bahnhöfen.
pub fn corridor(stations: usize, lx_every: usize) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# Erzeugt mit railit net generate: Korridor mit {stations} Bahnhöfen.\n\n[network]\nname = \"Korridor {stations}\"\n"
    );
    let _ = writeln!(
        out,
        "[[terminal]]\nname = \"GW\"\nkind = \"boundary\"\n\n[[terminal]]\nname = \"GE\"\nkind = \"boundary\"\n"
    );
    let link = |out: &mut String, from: &str, to: &str| {
        let _ = writeln!(out, "[[link]]\nfrom = \"{from}\"\nto = \"{to}\"\n");
    };
    let signal = |out: &mut String, name: &str, track: &str, at: u32, dir: &str, system: &str| {
        let kind = if name.starts_with('A') || name.starts_with('F') {
            "combined"
        } else {
            "main"
        };
        let _ = writeln!(
            out,
            "[[signal]]\nname = \"{name}\"\ntrack = \"{track}\"\nposition_m = {at}\ndirection = \"{dir}\"\nkind = \"{kind}\"\nsystem = \"{system}\"\n"
        );
    };
    let tvd = |out: &mut String, name: &str, element: &str| {
        let _ = writeln!(
            out,
            "[[tvd]]\nname = \"{name}\"\nelements = [\"{element}\"]\n"
        );
    };
    for k in 0..=stations {
        let l = format!("L{k}");
        let _ = writeln!(out, "[[track]]\nname = \"{l}\"\nlength_m = 2000\n");
        tvd(&mut out, &format!("T{l}"), &l);
        let from = if k == 0 {
            "GW.end".to_string()
        } else {
            format!("W{}b.tip", k - 1)
        };
        link(&mut out, &from, &format!("{l}.a"));
        if k > 0 {
            // Einfahrt in den vorigen Bahnhof von Osten.
            signal(&mut out, &format!("F{}", k - 1), &l, 0, "b_to_a", "hv");
        }
        if lx_every > 0 && k > 0 && k % lx_every == 0 {
            let _ = writeln!(
                out,
                "[[level_crossing]]\nname = \"BUE{k}\"\ntrack = \"{l}\"\nposition_m = 1000\n"
            );
        }
        if k == stations {
            link(&mut out, &format!("{l}.b"), "GE.end");
            continue;
        }
        signal(&mut out, &format!("A{k}"), &l, 2000, "a_to_b", "ks");
        for side in ["a", "b"] {
            let w = format!("W{k}{side}");
            let _ = writeln!(
                out,
                "[[point]]\nname = \"{w}\"\ndiverging = \"right\"\ndiverging_speed_kmh = 40\n"
            );
            tvd(&mut out, &format!("T{w}"), &w);
        }
        link(&mut out, &format!("{l}.b"), &format!("W{k}a.tip"));
        for (track, leg) in [("1", "left"), ("2", "right")] {
            let s = format!("S{k}_{track}");
            let _ = writeln!(out, "[[track]]\nname = \"{s}\"\nlength_m = 600\n");
            tvd(&mut out, &format!("T{s}"), &s);
            link(&mut out, &format!("W{k}a.{leg}"), &format!("{s}.a"));
            link(&mut out, &format!("{s}.b"), &format!("W{k}b.{leg}"));
            signal(&mut out, &format!("N{k}_{track}"), &s, 600, "a_to_b", "ks");
            signal(&mut out, &format!("P{k}_{track}"), &s, 0, "b_to_a", "hv");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_corridor_is_a_valid_network() {
        let source = corridor(3, 2);
        let net = crate::toml_format::parse(&source)
            .unwrap_or_else(|e| panic!("{e}\n{source}"))
            .topology;
        assert_eq!(net.name(), "Korridor 3");
    }
}
