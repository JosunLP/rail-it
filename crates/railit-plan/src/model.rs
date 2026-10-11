//! Eigentümer-Modell eines Plans; Schreiben und Signieren von Plan-Images.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use ed25519_dalek::{Signer, SigningKey};
use railit_types::bitset::{BitSetMut, words_for};
use railit_types::{PointId, PointPosition, RouteId, SignalId, TvdId};
use sha2::{Digest, Sha256};

use crate::error::PlanError;
use crate::format::{
    END_BOUNDARY, END_BUFFER_STOP, END_SIGNAL, FLAG_SIGNED, H_BODY_LEN, H_CONFLICT_OFF, H_FLAGS,
    H_NAMES_LEN, H_NAMES_OFF, H_POINT_TVD_OFF, H_POINTS, H_POOL_OFF, H_POOL_WORDS, H_ROUTE_OFF,
    H_ROUTES, H_SIGNALS, H_TVDS, H_VERSION, HEADER_LEN, MAGIC, NO_SIGNAL, VERSION, encode_point,
};
use crate::view::{PlanView, RouteEnd};

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
}

/// Gleisfreimeldeabschnitt im Plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TvdDef {
    /// Fachlicher Name.
    pub name: String,
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
    /// Erforderliche Weichenlagen.
    pub points: Vec<(PointId, PointPosition)>,
    /// Gleisfreimeldeabschnitte in Fahrreihenfolge.
    pub tvds: Vec<TvdId>,
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
                .map(|i| SignalDef {
                    name: view
                        .signal_name(SignalId::new(i))
                        .unwrap_or_default()
                        .into(),
                })
                .collect(),
            tvds: ids(view.tvd_count())
                .map(|i| TvdDef {
                    name: view.tvd_name(TvdId::new(i)).unwrap_or_default().into(),
                })
                .collect(),
            routes: view
                .routes()
                .map(|r| RouteDef {
                    name: r.name().into(),
                    start: r.start(),
                    end: r.end(),
                    points: r.points().collect(),
                    tvds: r.tvds().collect(),
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

    fn body(&self, signed: bool) -> Result<Vec<u8>, PlanError> {
        let mut w = Writer {
            buf: vec![0u8; HEADER_LEN],
        };

        // Pool vorbereiten: Weichen und TVDs je Fahrstraße.
        let mut pool: Vec<u32> = Vec::new();
        let mut ranges = Vec::with_capacity(self.routes.len());
        for r in &self.routes {
            let points_start = Writer::len(pool.len())?;
            for (point, pos) in &r.points {
                pool.push(encode_point(*point, *pos).ok_or(PlanError::TooLarge)?);
            }
            let tvds_start = Writer::len(pool.len())?;
            pool.extend(r.tvds.iter().map(|t| t.raw()));
            ranges.push((
                points_start,
                Writer::len(r.points.len())?,
                tvds_start,
                Writer::len(r.tvds.len())?,
            ));
        }

        let route_off = w.pos()?;
        for (r, (ps, pl, ts, tl)) in self.routes.iter().zip(&ranges) {
            let (kind, end_signal) = match r.end {
                RouteEnd::Signal(s) => (END_SIGNAL, s.raw()),
                RouteEnd::Boundary => (END_BOUNDARY, NO_SIGNAL),
                RouteEnd::BufferStop => (END_BUFFER_STOP, NO_SIGNAL),
            };
            for v in [r.start.raw(), kind, end_signal, *ps, *pl, *ts, *tl, 0] {
                w.u32(v);
            }
        }

        let pool_off = w.pos()?;
        for v in &pool {
            w.u32(*v);
        }

        let point_tvd_off = w.pos()?;
        for p in &self.points {
            w.u32(p.tvd.raw());
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
            (H_ROUTE_OFF, route_off),
            (H_POOL_OFF, pool_off),
            (H_POOL_WORDS, Writer::len(pool.len())?),
            (H_CONFLICT_OFF, conflict_off),
            (H_POINT_TVD_OFF, point_tvd_off),
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
