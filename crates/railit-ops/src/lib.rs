//! Betriebsführung von rail-it: Zugnummernmeldung, Fahrplan, Zuglenkung und Disposition.
//!
//! Die Betriebsführung liegt **über** den Stellwerkskernen und ist nicht sicherheitsrelevant:
//! Sie stellt Anforderungen (Fahrstraße einstellen, Erlaubnis anfordern) wie ein Bediener.
//! Jede Anforderung prüft der Kern; eine falsche Entscheidung der Zuglenkung kann höchstens
//! den Betrieb verzögern, nie einen Zug gefährden.
//!
//! - [`OpsNet`]: Fahrstraßengraph über alle Stellbereiche; Grenzen führen zum Einfahrsignal
//!   des Nachbarn.
//! - [`Timetable`]: Fahrplan (TOML) mit Halten an Bahnsteiggleisen und Einsetzzeiten für die
//!   Simulation.
//! - [`Ops`]: Zugnummernmeldung (Nummern wandern mit den Fahrstraßen von Signal zu Signal),
//!   Zuglenkung (nächste Fahrstraße auf dem kürzesten Weg zum nächsten Halt, Erlaubnisse
//!   entlang des Weges), Selbstblock und Disposition (Zurückhalten, Gleiswechsel, Wartezeit).

mod net;
mod ops;
mod timetable;

pub use net::{OpsNet, OpsRoute, PlatformInfo, RouteNext, StationInfo};
pub use ops::{Ops, OpsActions, OpsConfig, OpsEvent, OpsSnapshot, OpsView, Spawn, TrainSnapshot};
pub use timetable::{Enter, Stop, Timetable, TrainRun, format_time, parse_time};
