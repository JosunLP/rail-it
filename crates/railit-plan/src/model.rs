//! Eigentümer-Modell eines Plans; Schreiben und Signieren von Plan-Images.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use ed25519_dalek::{Signer, SigningKey};
use railit_types::bitset::{BitSetMut, words_for};
use railit_types::{
    LxId, PointId, PointPosition, RouteId, SignalId, SignalKind, SignalSystem, Speed, TvdId,
};
use sha2::{Digest, Sha256};

use crate::error::PlanError;
use crate::format::{
    END_BOUNDARY, END_BUFFER_STOP, END_SIGNAL, FLAG_SIGNED, FLANK_POINT, FLANK_SIGNAL, H_BODY_LEN,
    H_CONFLICT_OFF, H_FLAGS, H_LX_TVD_OFF, H_LXS, H_NAMES_LEN, H_NAMES_OFF, H_POINT_TVD_OFF,
    H_POINTS, H_POOL_OFF, H_POOL_WORDS, H_ROUTE_OFF, H_ROUTES, H_SIGNAL_OFF, H_SIGNALS,
    H_TVD_FLAGS_OFF, H_TVDS, H_VERSION, HEADER_LEN, MAGIC, NONE, TVD_ENTRY, VERSION, encode_point,
};
use crate::view::{Flank, PlanView, Protection, RouteEnd};

/// Weiche im Plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PointDef {
    /// Fachlicher Name.
    pub name: String,
    /// Gleisfreimeldeabschnitt, in dem die Weiche liegt.
    pub tvd: TvdId,
}

/// Signal im Plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignalDef {
    /// Fachlicher Name.
    pub name: String,
    /// Signalart.
    pub kind: SignalKind,
    /// Signalsystem.
    pub system: SignalSystem,
    /// Bei Vorsignalen: das angekündigte Hauptsignal.
    pub announces: Option<SignalId>,
}

/// Gleisfreimeldeabschnitt im Plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TvdDef {
    /// Fachlicher Name.
    pub name: String,
    /// Einfahrabschnitt an einer Netzgrenze.
    pub entry: bool,
}

/// Bahnübergang im Plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LxDef {
    /// Fachlicher Name.
    pub name: String,
    /// Gleisfreimeldeabschnitt, in dem der Bahnübergang liegt.
    pub tvd: TvdId,
}

/// Fahrstraße im Plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteDef {
    /// Fachlicher Name, z. B. `"A-N1"`.
    pub name: String,
    /// Startsignal.
    pub start: SignalId,
    /// Ziel.
    pub end: RouteEnd,
    /// Zulässige Geschwindigkeit, falls beschränkt.
    pub speed: Option<Speed>,
    /// Erforderliche Weichenlagen im Fahrweg.
    pub points: Vec<(PointId, PointPosition)>,
    /// Gleisfreimeldeabschnitte in Fahrreihenfolge.
    pub tvds: Vec<TvdId>,
    /// Gleisfreimeldeabschnitte des Durchrutschwegs.
    pub overlap_tvds: Vec<TvdId>,
    /// Erforderliche Weichenlagen im Durchrutschweg.
    pub overlap_points: Vec<(PointId, PointPosition)>,
    /// Flankenschutzmaßnahmen.
    pub flank: Vec<Flank>,
    /// Bahnübergänge im Fahrweg und Durchrutschweg.
    pub level_crossings: Vec<LxId>,
    /// Feindliche Fahrstraßen.
    pub conflicts: Vec<RouteId>,
}

/// Vollständiger Plan im Speicher.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Name des Netzes.
    pub network: String,
    /// Weichen, Index = [`PointId`].
    pub points: Vec<PointDef>,
    /// Signale, Index = [`SignalId`].
    pub signals: Vec<SignalDef>,
    /// Gleisfreimeldeabschnitte, Index = [`TvdId`].
    pub tvds: Vec<TvdDef>,
    /// Bahnübergänge, Index = [`LxId`].
    pub level_crossings: Vec<LxDef>,
    /// Fahrstraßen, Index = [`RouteId`].
    pub routes: Vec<RouteDef>,
}

/// Puffer, in den das Image geschrieben wird.
struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    fn pos(&self) -> Result<u32, PlanError> {
        u32::try_from(self.buf.len()).map_err(|_| PlanError::TooLarge)
    }

    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn len(n: usize) -> Result<u32, PlanError> {
        u32::try_from(n).map_err(|_| PlanError::TooLarge)
    }

    fn patch(&mut self, word: usize, v: u32) -> Result<(), PlanError> {
        let off = word.checked_mul(4).ok_or(PlanError::TooLarge)?;
        let end = off.checked_add(4).ok_or(PlanError::TooLarge)?;
        self.buf
            .get_mut(off..end)
            .ok_or(PlanError::TooLarge)?
            .copy_from_slice(&v.to_le_bytes());
        Ok(())
    }
}

/// Pool mit Listen; liefert (Start, Anzahl) je Liste.
struct Pool {
    words: Vec<u32>,
}

impl Pool {
    fn list(
        &mut self,
        items: impl IntoIterator<Item = u32>,
        per_item: usize,
    ) -> Result<(u32, u32), PlanError> {
        let start = Writer::len(self.words.len())?;
        self.words.extend(items);
        let words = self
            .words
            .len()
            .checked_sub(usize::try_from(start).map_err(|_| PlanError::TooLarge)?)
            .ok_or(PlanError::TooLarge)?;
        let count = words.checked_div(per_item).ok_or(PlanError::TooLarge)?;
        Ok((start, Writer::len(count)?))
    }
}

fn encode_points(points: &[(PointId, PointPosition)]) -> Result<Vec<u32>, PlanError> {
    points
        .iter()
        .map(|(p, pos)| encode_point(*p, *pos).ok_or(PlanError::TooLarge))
        .collect()
}

impl Plan {
    /// Liest einen geprüften Plan zurück in das Eigentümer-Modell.
    pub fn from_view(view: &PlanView<'_>) -> Self {
        let ids = |n: usize| (0..n).filter_map(|i| u32::try_from(i).ok());
        Self {
            network: view.network_name().into(),
            points: ids(view.point_count())
                .map(PointId::new)
                .map(|id| PointDef {
                    name: view.point_name(id).unwrap_or_default().into(),
                    tvd: view.point_tvd(id).unwrap_or(TvdId::new(u32::MAX)),
                })
                .collect(),
            signals: ids(view.signal_count())
                .map(SignalId::new)
                .map(|id| SignalDef {
                    name: view.signal_name(id).unwrap_or_default().into(),
                    kind: view.signal_kind(id).unwrap_or(SignalKind::Main),
                    system: view.signal_system(id).unwrap_or_default(),
                    announces: view.signal_announces(id),
                })
                .collect(),
            tvds: ids(view.tvd_count())
                .map(TvdId::new)
                .map(|id| TvdDef {
                    name: view.tvd_name(id).unwrap_or_default().into(),
                    entry: view.tvd_is_entry(id),
                })
                .collect(),
            level_crossings: ids(view.lx_count())
                .map(LxId::new)
                .map(|id| LxDef {
                    name: view.lx_name(id).unwrap_or_default().into(),
                    tvd: view.lx_tvd(id).unwrap_or(TvdId::new(u32::MAX)),
                })
                .collect(),
            routes: view
                .routes()
                .map(|r| RouteDef {
                    name: r.name().into(),
                    start: r.start(),
                    end: r.end(),
                    speed: r.speed(),
                    points: r.points().collect(),
                    tvds: r.tvds().collect(),
                    overlap_tvds: r.overlap_tvds().collect(),
                    overlap_points: r.overlap_points().collect(),
                    flank: r.flank().collect(),
                    level_crossings: r.level_crossings().collect(),
                    conflicts: r.conflicts().collect(),
                })
                .collect(),
        }
    }

    /// Schreibt das Plan-Image ohne Signatur.
    ///
    /// Das Ergebnis wird vor der Rückgabe mit [`PlanView::parse`] vollständig geprüft; ein
    /// inkonsistenter Plan (z. B. ungültige Verweise, unsymmetrische Feindlichkeiten) wird
    /// abgewiesen.
    ///
    /// # Errors
    /// [`PlanError`], wenn der Plan ungültig ist oder die Formatgrenzen überschreitet.
    pub fn to_image(&self) -> Result<Vec<u8>, PlanError> {
        let mut image = self.body(false)?;
        let digest = Sha256::digest(&image);
        image.extend_from_slice(digest.as_slice());
        PlanView::parse(&image)?;
        Ok(image)
    }

    /// Schreibt das Plan-Image und signiert es mit `key` (REQ-PLN-004).
    ///
    /// # Errors
    /// Wie [`Self::to_image`].
    pub fn to_signed_image(&self, key: &SigningKey) -> Result<Vec<u8>, PlanError> {
        let mut image = self.body(true)?;
        let digest = Sha256::digest(&image);
        image.extend_from_slice(digest.as_slice());
        let signature = key.sign(digest.as_slice());
        image.extend_from_slice(&signature.to_bytes());
        PlanView::parse_signed(&image, &key.verifying_key())?;
        Ok(image)
    }

    #[allow(clippy::too_many_lines)]
    fn body(&self, signed: bool) -> Result<Vec<u8>, PlanError> {
        let mut w = Writer {
            buf: vec![0u8; HEADER_LEN],
        };

        // Pool: alle Listen aller Fahrstraßen.
        let mut pool = Pool { words: Vec::new() };
        let mut entries: Vec<[u32; 16]> = Vec::with_capacity(self.routes.len());
        for r in &self.routes {
            let (end_kind, end_signal) = match r.end {
                RouteEnd::Signal(s) => (END_SIGNAL, s.raw()),
                RouteEnd::Boundary => (END_BOUNDARY, NONE),
                RouteEnd::BufferStop => (END_BUFFER_STOP, NONE),
            };
            let points = pool.list(encode_points(&r.points)?, 1)?;
            let tvds = pool.list(r.tvds.iter().map(|t| t.raw()), 1)?;
            let overlap_tvds = pool.list(r.overlap_tvds.iter().map(|t| t.raw()), 1)?;
            let overlap_points = pool.list(encode_points(&r.overlap_points)?, 1)?;
            let mut flank_words = Vec::with_capacity(r.flank.len().saturating_mul(3));
            for f in &r.flank {
                let (kind, element) = match f.protection {
                    Protection::Point(p, pos) => (
                        FLANK_POINT,
                        encode_point(p, pos).ok_or(PlanError::TooLarge)?,
                    ),
                    Protection::Signal(s) => (FLANK_SIGNAL, s.raw()),
                };
                flank_words.extend([kind, element, f.protects.raw()]);
            }
            let flank = pool.list(flank_words, 3)?;
            let lxs = pool.list(r.level_crossings.iter().map(|l| l.raw()), 1)?;
            let speed = r.speed.map_or(0, |s| u32::from(s.as_kmh()));
            entries.push([
                r.start.raw(),
                end_kind,
                end_signal,
                speed,
                points.0,
                points.1,
                tvds.0,
                tvds.1,
                overlap_tvds.0,
                overlap_tvds.1,
                overlap_points.0,
                overlap_points.1,
                flank.0,
                flank.1,
                lxs.0,
                lxs.1,
            ]);
        }

        let route_off = w.pos()?;
        for entry in &entries {
            for v in entry {
                w.u32(*v);
            }
        }

        let pool_off = w.pos()?;
        for v in &pool.words {
            w.u32(*v);
        }

        let point_tvd_off = w.pos()?;
        for p in &self.points {
            w.u32(p.tvd.raw());
        }

        let signal_off = w.pos()?;
        for s in &self.signals {
            w.u32(s.kind.code());
            w.u32(s.system.code());
            w.u32(s.announces.map_or(NONE, railit_types::SignalId::raw));
        }

        let tvd_flags_off = w.pos()?;
        for t in &self.tvds {
            w.u32(if t.entry { TVD_ENTRY } else { 0 });
        }

        let lx_tvd_off = w.pos()?;
        for l in &self.level_crossings {
            w.u32(l.tvd.raw());
        }

        let conflict_off = w.pos()?;
        let row_words = words_for(self.routes.len());
        for (index, r) in self.routes.iter().enumerate() {
            let mut row = vec![0u64; row_words];
            let mut set = BitSetMut::new(&mut row);
            for c in &r.conflicts {
                let bit = c.index().ok_or(PlanError::TooLarge)?;
                set.insert(bit).map_err(|_| PlanError::InvalidReference {
                    what: "feindliche Fahrstraße",
                    index,
                })?;
            }
            for word in row {
                w.u64(word);
            }
        }

        let names_off = w.pos()?;
        let names: Vec<&str> = core::iter::once(self.network.as_str())
            .chain(self.points.iter().map(|p| p.name.as_str()))
            .chain(self.signals.iter().map(|s| s.name.as_str()))
            .chain(self.tvds.iter().map(|t| t.name.as_str()))
            .chain(self.routes.iter().map(|r| r.name.as_str()))
            .chain(self.level_crossings.iter().map(|l| l.name.as_str()))
            .collect();
        let mut data_off = 0u32;
        for name in &names {
            let len = Writer::len(name.len())?;
            w.u32(data_off);
            w.u32(len);
            data_off = data_off.checked_add(len).ok_or(PlanError::TooLarge)?;
        }
        for name in &names {
            w.buf.extend_from_slice(name.as_bytes());
        }
        while !w.buf.len().is_multiple_of(4) {
            w.buf.push(0);
        }
        let names_len = w.pos()?.checked_sub(names_off).ok_or(PlanError::TooLarge)?;
        let body_len = w.pos()?;

        let header = [
            (H_VERSION, VERSION),
            (H_BODY_LEN, body_len),
            (H_POINTS, Writer::len(self.points.len())?),
            (H_SIGNALS, Writer::len(self.signals.len())?),
            (H_TVDS, Writer::len(self.tvds.len())?),
            (H_ROUTES, Writer::len(self.routes.len())?),
            (H_LXS, Writer::len(self.level_crossings.len())?),
            (H_ROUTE_OFF, route_off),
            (H_POOL_OFF, pool_off),
            (H_POOL_WORDS, Writer::len(pool.words.len())?),
            (H_CONFLICT_OFF, conflict_off),
            (H_POINT_TVD_OFF, point_tvd_off),
            (H_SIGNAL_OFF, signal_off),
            (H_TVD_FLAGS_OFF, tvd_flags_off),
            (H_LX_TVD_OFF, lx_tvd_off),
            (H_NAMES_OFF, names_off),
            (H_NAMES_LEN, names_len),
            (H_FLAGS, if signed { FLAG_SIGNED } else { 0 }),
        ];
        w.buf
            .get_mut(..4)
            .ok_or(PlanError::TooLarge)?
            .copy_from_slice(&MAGIC);
        for (word, value) in header {
            w.patch(word, value)?;
        }
        Ok(w.buf)
    }
}
