//! Fahrplan im TOML-Format.
//!
//! ```toml
//! [timetable]
//! name = "Nordbahn Grundtakt"
//!
//! [[train]]
//! number = "RE1"
//! # Optional: Die Simulation setzt den Zug zur Zeit `at` vor dem Signal ein.
//! enter = { signal = "AA", at = "00:00:10", length_m = 100, speed_kmh = 80 }
//! stops = [
//!     { station = "Adorf", platform = "1", arr = "00:01:00", dep = "00:02:00" },
//!     { station = "Bstadt", platform = "1", arr = "00:08:00" },
//! ]
//! # Nach dem letzten Halt das Netz verlassen (Standard: dort stehen bleiben).
//! exit = false
//! ```
//!
//! Zeiten sind `HH:MM:SS` oder `HH:MM` ab Beginn der Simulation bzw. des Betriebs.

use std::collections::BTreeSet;

use railit_types::Tick;
use serde::Deserialize;

use crate::net::OpsNet;

/// Einsetzen eines Zuges in die Simulation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enter {
    /// Der Zug steht mit der Spitze vor diesem Signal.
    pub signal: String,
    /// Zeitpunkt.
    pub at: Tick,
    /// Zuglänge in Metern.
    pub length_m: u32,
    /// Geschwindigkeit in km/h.
    pub speed_kmh: u16,
}

/// Halt an einem Bahnsteiggleis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stop {
    /// Betriebsstelle.
    pub station: String,
    /// Bahnsteiggleis.
    pub platform: String,
    /// Planmäßige Ankunft.
    pub arr: Option<Tick>,
    /// Planmäßige Abfahrt (frühestens).
    pub dep: Option<Tick>,
    /// Mindesthaltezeit in Millisekunden (sonst die der Konfiguration).
    pub dwell_ms: Option<u64>,
}

/// Fahrplan eines Zuges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrainRun {
    /// Zugnummer.
    pub number: String,
    /// Einsetzen in die Simulation.
    pub enter: Option<Enter>,
    /// Halte in Fahrtrichtung.
    pub stops: Vec<Stop>,
    /// Nach dem letzten Halt das Netz verlassen.
    pub exit: bool,
}

/// Fahrplan.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Timetable {
    /// Name.
    pub name: String,
    /// Züge.
    pub trains: Vec<TrainRun>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    timetable: Option<Header>,
    #[serde(default, rename = "train")]
    trains: Vec<TrainFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrainFile {
    number: String,
    #[serde(default)]
    enter: Option<EnterFile>,
    #[serde(default)]
    stops: Vec<StopFile>,
    #[serde(default)]
    exit: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnterFile {
    signal: String,
    at: String,
    length_m: u32,
    speed_kmh: u16,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StopFile {
    station: String,
    platform: String,
    #[serde(default)]
    arr: Option<String>,
    #[serde(default)]
    dep: Option<String>,
    #[serde(default)]
    dwell_s: Option<u32>,
}

/// Liest eine Zeit `HH:MM:SS` oder `HH:MM`.
///
/// # Errors
/// Beschreibung, wenn die Zeit ungültig ist.
pub fn parse_time(text: &str) -> Result<Tick, String> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    let num = |s: &str, max: u64| -> Result<u64, String> {
        let v: u64 = s.parse().map_err(|_| format!("Zeit „{text}“ ungültig"))?;
        if v > max {
            return Err(format!("Zeit „{text}“ ungültig"));
        }
        Ok(v)
    };
    let (h, m, s) = match parts.as_slice() {
        [h, m] => (num(h, 9999)?, num(m, 59)?, 0),
        [h, m, s] => (num(h, 9999)?, num(m, 59)?, num(s, 59)?),
        _ => return Err(format!("Zeit „{text}“: erwartet HH:MM:SS")),
    };
    Ok(Tick::from_millis((h * 3600 + m * 60 + s) * 1000))
}

/// Gibt eine Zeit als `HH:MM:SS` aus.
pub fn format_time(t: Tick) -> String {
    let s = t.as_millis() / 1000;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

impl Timetable {
    /// Liest einen Fahrplan.
    ///
    /// # Errors
    /// Beschreibung, wenn die Datei kein gültiger Fahrplan ist.
    pub fn parse(source: &str) -> Result<Self, String> {
        let file: File = toml::from_str(source).map_err(|e| e.to_string())?;
        let opt = |t: Option<String>| t.as_deref().map(parse_time).transpose();
        let mut trains = Vec::with_capacity(file.trains.len());
        for t in file.trains {
            let enter = t
                .enter
                .map(|e| -> Result<Enter, String> {
                    Ok(Enter {
                        signal: e.signal,
                        at: parse_time(&e.at)?,
                        length_m: e.length_m,
                        speed_kmh: e.speed_kmh,
                    })
                })
                .transpose()?;
            let mut stops = Vec::with_capacity(t.stops.len());
            for s in t.stops {
                stops.push(Stop {
                    station: s.station,
                    platform: s.platform,
                    arr: opt(s.arr)?,
                    dep: opt(s.dep)?,
                    dwell_ms: s.dwell_s.map(|d| u64::from(d) * 1000),
                });
            }
            trains.push(TrainRun {
                number: t.number,
                enter,
                stops,
                exit: t.exit,
            });
        }
        Ok(Self {
            name: file.timetable.map(|h| h.name).unwrap_or_default(),
            trains,
        })
    }

    /// Prüft den Fahrplan gegen das Netz: Zugnummern eindeutig, Betriebsstellen,
    /// Bahnsteige und Signale vorhanden, Zeiten aufsteigend.
    ///
    /// # Errors
    /// Alle Abweichungen.
    pub fn validate(&self, net: &OpsNet) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        let mut numbers = BTreeSet::new();
        for t in &self.trains {
            let n = &t.number;
            if n.is_empty() {
                errors.push("Zug ohne Nummer".to_string());
            }
            if !numbers.insert(n.as_str()) {
                errors.push(format!("Zug {n}: Nummer doppelt"));
            }
            if let Some(e) = &t.enter
                && !net.has_signal(&e.signal)
            {
                errors.push(format!("Zug {n}: Signal „{}“ unbekannt", e.signal));
            }
            let mut last = t.enter.as_ref().map(|e| e.at);
            for s in &t.stops {
                if net.platform_signals(&s.station, &s.platform).is_none() {
                    errors.push(format!(
                        "Zug {n}: Bahnsteig „{}“ in „{}“ unbekannt",
                        s.platform, s.station
                    ));
                }
                for time in [s.arr, s.dep].into_iter().flatten() {
                    if last.is_some_and(|l| time < l) {
                        errors.push(format!(
                            "Zug {n}: Zeiten in {} nicht aufsteigend",
                            s.station
                        ));
                    }
                    last = Some(time);
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Fahrplan eines Zuges.
    pub fn train(&self, number: &str) -> Option<&TrainRun> {
        self.trains.iter().find(|t| t.number == number)
    }
}
