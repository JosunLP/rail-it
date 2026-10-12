//! Zero-Copy-Sicht auf ein Plan-Image.

use core::fmt;

use ed25519_dalek::{Signature, VerifyingKey};
use railit_types::bitset::{BitSet, WORD_BITS, words_for};
use railit_types::{
    BorderId, LxId, PointId, PointPosition, RouteId, SignalId, SignalKind, SignalSystem, Speed,
    TvdId,
};
use sha2::{Digest, Sha256};

use crate::error::PlanError;
use crate::format::{
    BORDER_INITIAL, BORDER_WORDS, DIGEST_LEN, END_BORDER, END_BOUNDARY, END_BUFFER_STOP,
    END_SIGNAL, FLAG_SIGNED, FLANK_POINT, FLANK_SIGNAL, FLANK_WORDS, H_BODY_LEN, H_BORDER_OFF,
    H_BORDERS, H_CONFLICT_OFF, H_FLAGS, H_LX_TVD_OFF, H_LXS, H_NAMES_LEN, H_NAMES_OFF,
    H_POINT_TVD_OFF, H_POINTS, H_POOL_OFF, H_POOL_WORDS, H_ROUTE_OFF, H_ROUTES, H_SIGNAL_OFF,
    H_SIGNALS, H_TVD_FLAGS_OFF, H_TVDS, H_VERSION, HEADER_LEN, MAGIC, NONE, ROUTE_LEN,
    SIGNAL_WORDS, SIGNATURE_LEN, TVD_ENTRY, VERSION, decode_point, r, read_u32, read_u64,
    read_word,
};

/// Ziel einer Fahrstraße.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum RouteEnd {
    /// Zielsignal.
    Signal(SignalId),
    /// Netzgrenze: Die Fahrt führt aus dem modellierten Netz hinaus (z. B. auf die Strecke).
    Boundary,
    /// Prellbock.
    BufferStop,
    /// Grenze zu einem benachbarten Stellbereich.
    Border(BorderId),
}

/// Flankenschutzmaßnahme.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Protection {
    /// Schutzweiche: Die Weiche muss in dieser Lage liegen und verschlossen sein.
    Point(PointId, PointPosition),
    /// Schutzsignal: Das Signal muss Halt zeigen und darf nicht auf Fahrt gestellt werden.
    Signal(SignalId),
}

/// Flankenschutz-Eintrag einer Fahrstraße.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Flank {
    /// Maßnahme.
    pub protection: Protection,
    /// Abschnitt der Fahrstraße, den die Maßnahme schützt. Sie wird mit ihm aufgelöst.
    pub protects: TvdId,
}

fn add(a: usize, b: usize) -> Result<usize, PlanError> {
    a.checked_add(b).ok_or(PlanError::TooLarge)
}

fn mul(a: usize, b: usize) -> Result<usize, PlanError> {
    a.checked_mul(b).ok_or(PlanError::TooLarge)
}

fn to_usize(v: u32) -> Result<usize, PlanError> {
    usize::try_from(v).map_err(|_| PlanError::TooLarge)
}

/// Bereich einer Liste im Pool.
#[derive(Copy, Clone, Debug, Default)]
struct Range {
    start: usize,
    len: usize,
}

#[derive(Copy, Clone, Debug)]
struct RawRoute {
    start: u32,
    end_kind: u32,
    end_signal: u32,
    speed: u32,
    points: Range,
    tvds: Range,
    overlap_tvds: Range,
    overlap_points: Range,
    flank: Range,
    lxs: Range,
}

/// Geprüfte, unveränderliche Sicht auf ein Plan-Image.
///
/// Eine `PlanView` existiert nur, wenn [`PlanView::parse`] Struktur, Prüfsumme und alle
/// Verweise erfolgreich geprüft hat (REQ-PLN-001, REQ-PLN-002). Die Zugriffsmethoden geben
/// trotzdem `Option` zurück, damit der Kern nie in einen Panikpfad gerät; nach erfolgreicher
/// Prüfung liefern sie für gültige IDs immer `Some`.
#[derive(Copy, Clone)]
pub struct PlanView<'a> {
    bytes: &'a [u8],
    body_len: usize,
    points: usize,
    signals: usize,
    tvds: usize,
    routes: usize,
    lxs: usize,
    borders: usize,
    border_off: usize,
    route_off: usize,
    pool_off: usize,
    pool_words: usize,
    conflict_off: usize,
    conflict_words: usize,
    point_tvd_off: usize,
    signal_off: usize,
    tvd_flags_off: usize,
    lx_tvd_off: usize,
    names_off: usize,
    names_data_off: usize,
    names_end: usize,
    names_count: usize,
    signed: bool,
}

impl fmt::Debug for PlanView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlanView")
            .field("network", &self.network_name())
            .field("points", &self.points)
            .field("signals", &self.signals)
            .field("tvds", &self.tvds)
            .field("routes", &self.routes)
            .field("level_crossings", &self.lxs)
            .field("signed", &self.signed)
            .finish_non_exhaustive()
    }
}

impl<'a> PlanView<'a> {
    /// Prüft ein Plan-Image vollständig und gibt eine Sicht darauf zurück.
    ///
    /// Geprüft werden Kennung, Version, Längen, Lage aller Abschnitte, die SHA-256-Prüfsumme,
    /// alle Verweise, die Feindlichkeitsmatrix und die Namen. Eine Signatur wird hier **nicht**
    /// geprüft, siehe [`Self::parse_signed`].
    ///
    /// # Errors
    /// [`PlanError`], wenn irgendeine Prüfung fehlschlägt.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, PlanError> {
        if bytes.len() < HEADER_LEN {
            return Err(PlanError::TooShort);
        }
        if bytes.get(..4) != Some(&MAGIC[..]) {
            return Err(PlanError::BadMagic);
        }
        let h = |i| read_word(bytes, 0, i).ok_or(PlanError::TooShort);
        let version = h(H_VERSION)?;
        if version != VERSION {
            return Err(PlanError::UnsupportedVersion(version));
        }

        let signed = h(H_FLAGS)? & FLAG_SIGNED != 0;
        let body_len = to_usize(h(H_BODY_LEN)?)?;
        let trailer = if signed {
            add(DIGEST_LEN, SIGNATURE_LEN)?
        } else {
            DIGEST_LEN
        };
        if body_len < HEADER_LEN || add(body_len, trailer)? != bytes.len() {
            return Err(PlanError::LengthMismatch);
        }

        let points = to_usize(h(H_POINTS)?)?;
        let signals = to_usize(h(H_SIGNALS)?)?;
        let tvds = to_usize(h(H_TVDS)?)?;
        let routes = to_usize(h(H_ROUTES)?)?;
        let lxs = to_usize(h(H_LXS)?)?;
        let borders = to_usize(h(H_BORDERS)?)?;
        let names_count = add(
            add(
                add(add(add(add(1, points)?, signals)?, tvds)?, routes)?,
                lxs,
            )?,
            borders,
        )?;

        let mut view = Self {
            bytes,
            body_len,
            points,
            signals,
            tvds,
            routes,
            lxs,
            borders,
            border_off: to_usize(h(H_BORDER_OFF)?)?,
            route_off: to_usize(h(H_ROUTE_OFF)?)?,
            pool_off: to_usize(h(H_POOL_OFF)?)?,
            pool_words: to_usize(h(H_POOL_WORDS)?)?,
            conflict_off: to_usize(h(H_CONFLICT_OFF)?)?,
            conflict_words: words_for(routes),
            point_tvd_off: to_usize(h(H_POINT_TVD_OFF)?)?,
            signal_off: to_usize(h(H_SIGNAL_OFF)?)?,
            tvd_flags_off: to_usize(h(H_TVD_FLAGS_OFF)?)?,
            lx_tvd_off: to_usize(h(H_LX_TVD_OFF)?)?,
            names_off: to_usize(h(H_NAMES_OFF)?)?,
            names_data_off: 0,
            names_end: 0,
            names_count,
            signed,
        };

        view.check_section("Fahrstraßen", view.route_off, mul(routes, ROUTE_LEN)?)?;
        view.check_section("Pool", view.pool_off, mul(view.pool_words, 4)?)?;
        view.check_section("Weichen-TVD", view.point_tvd_off, mul(points, 4)?)?;
        view.check_section(
            "Signale",
            view.signal_off,
            mul(mul(signals, SIGNAL_WORDS)?, 4)?,
        )?;
        view.check_section("TVD-Merkmale", view.tvd_flags_off, mul(tvds, 4)?)?;
        view.check_section("BÜ-TVD", view.lx_tvd_off, mul(lxs, 4)?)?;
        view.check_section(
            "Grenzen",
            view.border_off,
            mul(mul(borders, BORDER_WORDS)?, 4)?,
        )?;
        view.check_section(
            "Feindlichkeiten",
            view.conflict_off,
            mul(mul(routes, view.conflict_words)?, 8)?,
        )?;
        let names_len = to_usize(h(H_NAMES_LEN)?)?;
        view.check_section("Namen", view.names_off, names_len)?;
        let entries_len = mul(names_count, 8)?;
        if entries_len > names_len {
            return Err(PlanError::SectionOutOfBounds("Namen"));
        }
        view.names_data_off = add(view.names_off, entries_len)?;
        view.names_end = add(view.names_off, names_len)?;

        view.check_digest()?;
        view.check_tables()?;
        view.check_borders()?;
        view.check_routes()?;
        view.check_conflicts()?;
        view.check_names()?;
        Ok(view)
    }

    /// Wie [`Self::parse`], verlangt aber zusätzlich eine gültige Signatur von `key`
    /// (REQ-PLN-004).
    ///
    /// # Errors
    /// [`PlanError::SignatureMissing`] oder [`PlanError::SignatureInvalid`] sowie alle Fehler
    /// von [`Self::parse`].
    pub fn parse_signed(bytes: &'a [u8], key: &VerifyingKey) -> Result<Self, PlanError> {
        let view = Self::parse(bytes)?;
        view.verify_signature(key)?;
        Ok(view)
    }

    /// Prüft die Signatur gegen `key`.
    ///
    /// # Errors
    /// [`PlanError::SignatureMissing`], wenn das Image nicht signiert ist,
    /// [`PlanError::SignatureInvalid`], wenn die Signatur nicht passt.
    pub fn verify_signature(&self, key: &VerifyingKey) -> Result<(), PlanError> {
        if !self.signed {
            return Err(PlanError::SignatureMissing);
        }
        let sig_off = add(self.body_len, DIGEST_LEN)?;
        let raw = self
            .bytes
            .get(sig_off..add(sig_off, SIGNATURE_LEN)?)
            .ok_or(PlanError::LengthMismatch)?;
        let raw: [u8; SIGNATURE_LEN] = raw.try_into().map_err(|_| PlanError::LengthMismatch)?;
        let signature = Signature::from_bytes(&raw);
        key.verify_strict(&self.digest(), &signature)
            .map_err(|_| PlanError::SignatureInvalid)
    }

    fn check_section(&self, name: &'static str, off: usize, len: usize) -> Result<(), PlanError> {
        if off < HEADER_LEN || add(off, len)? > self.body_len {
            return Err(PlanError::SectionOutOfBounds(name));
        }
        Ok(())
    }

    fn check_digest(&self) -> Result<(), PlanError> {
        let body = self
            .bytes
            .get(..self.body_len)
            .ok_or(PlanError::LengthMismatch)?;
        let stored = self
            .bytes
            .get(self.body_len..add(self.body_len, DIGEST_LEN)?)
            .ok_or(PlanError::LengthMismatch)?;
        if Sha256::digest(body).as_slice() != stored {
            return Err(PlanError::DigestMismatch);
        }
        Ok(())
    }

    fn valid_tvd(&self, raw: u32) -> bool {
        to_usize(raw).is_ok_and(|t| t < self.tvds)
    }

    fn check_tables(&self) -> Result<(), PlanError> {
        for index in 0..self.points {
            let tvd = read_word(self.bytes, self.point_tvd_off, index);
            if !tvd.is_some_and(|t| self.valid_tvd(t)) {
                return Err(PlanError::InvalidReference {
                    what: "TVD einer Weiche",
                    index,
                });
            }
        }
        for index in 0..self.lxs {
            let tvd = read_word(self.bytes, self.lx_tvd_off, index);
            if !tvd.is_some_and(|t| self.valid_tvd(t)) {
                return Err(PlanError::InvalidReference {
                    what: "TVD eines Bahnübergangs",
                    index,
                });
            }
        }
        for index in 0..self.tvds {
            let flags = read_word(self.bytes, self.tvd_flags_off, index);
            if flags.is_none_or(|f| f & !TVD_ENTRY != 0) {
                return Err(PlanError::InvalidReference {
                    what: "TVD-Merkmal",
                    index,
                });
            }
        }
        for index in 0..self.signals {
            let id = SignalId::from_index(index).ok_or(PlanError::TooLarge)?;
            let bad = PlanError::InvalidReference {
                what: "Signaleintrag",
                index,
            };
            let kind = self.signal_kind(id).ok_or(bad)?;
            self.signal_system(id).ok_or(bad)?;
            let announces = self.signal_word(id, 2).ok_or(bad)?;
            if announces != NONE {
                let target = SignalId::new(announces);
                let target_delimits = self
                    .signal_kind(target)
                    .is_some_and(SignalKind::delimits_routes);
                if kind != SignalKind::Distant || !target_delimits {
                    return Err(PlanError::InvalidReference {
                        what: "angekündigtes Signal",
                        index,
                    });
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn check_routes(&self) -> Result<(), PlanError> {
        let bad = |what| move |index| PlanError::InvalidReference { what, index };
        for index in 0..self.routes {
            let raw = self
                .raw_route(index)
                .ok_or(PlanError::SectionOutOfBounds("Fahrstraßen"))?;
            let delimits = |s: u32| {
                self.signal_kind(SignalId::new(s))
                    .is_some_and(SignalKind::delimits_routes)
            };
            if !delimits(raw.start) {
                return Err(bad("Startsignal")(index));
            }
            match raw.end_kind {
                END_SIGNAL if delimits(raw.end_signal) => {}
                END_BOUNDARY | END_BUFFER_STOP if raw.end_signal == NONE => {}
                END_BORDER if to_usize(raw.end_signal).is_ok_and(|b| b < self.borders) => {}
                _ => return Err(bad("Ziel")(index)),
            }
            if u16::try_from(raw.speed).is_err() {
                return Err(bad("Geschwindigkeit")(index));
            }
            for (range, words) in [
                (raw.points, 1),
                (raw.tvds, 1),
                (raw.overlap_tvds, 1),
                (raw.overlap_points, 1),
                (raw.flank, FLANK_WORDS),
                (raw.lxs, 1),
            ] {
                if add(range.start, mul(range.len, words)?)? > self.pool_words {
                    return Err(bad("Liste außerhalb des Pools")(index));
                }
            }
            if raw.tvds.len == 0 {
                return Err(PlanError::EmptyRoute(index));
            }
            if raw.end_kind != END_SIGNAL && raw.overlap_tvds.len != 0 {
                return Err(bad("Durchrutschweg ohne Zielsignal")(index));
            }

            let view = RouteView {
                plan: *self,
                id: RouteId::from_index(index).ok_or(PlanError::TooLarge)?,
                raw,
            };
            let path_tvd = |t: TvdId| view.tvds().any(|x| x == t);
            let overlap_tvd = |t: TvdId| view.overlap_tvds().any(|x| x == t);

            // Abschnitte: gültig, eindeutig, Fahrweg und Durchrutschweg disjunkt.
            for (i, t) in view.tvds().enumerate() {
                if !self.valid_tvd(t.raw()) || view.tvds().skip(i.saturating_add(1)).any(|x| x == t)
                {
                    return Err(bad("TVD")(index));
                }
            }
            for (i, t) in view.overlap_tvds().enumerate() {
                if !self.valid_tvd(t.raw())
                    || path_tvd(t)
                    || view
                        .overlap_tvds()
                        .skip(i.saturating_add(1))
                        .any(|x| x == t)
                {
                    return Err(bad("TVD des Durchrutschwegs")(index));
                }
            }

            // Weichen: gültig, eindeutig über Fahrweg und Durchrutschweg, im eigenen Abschnitt.
            let all_points = || view.points().chain(view.overlap_points());
            for (i, (p, _)) in all_points().enumerate() {
                if p.index().is_none_or(|x| x >= self.points)
                    || all_points().skip(i.saturating_add(1)).any(|(o, _)| o == p)
                {
                    return Err(bad("doppelte oder ungültige Weiche")(index));
                }
            }
            for (p, _) in view.points() {
                if !self.point_tvd(p).is_some_and(path_tvd) {
                    return Err(bad("Weiche außerhalb der Abschnitte")(index));
                }
            }
            for (p, _) in view.overlap_points() {
                if !self.point_tvd(p).is_some_and(overlap_tvd) {
                    return Err(bad("Weiche außerhalb des Durchrutschwegs")(index));
                }
            }

            // Flankenschutz.
            let mut flank_count = 0usize;
            for f in view.flank() {
                flank_count = flank_count.saturating_add(1);
                if !(path_tvd(f.protects) || overlap_tvd(f.protects)) {
                    return Err(bad("Flankenschutz schützt fremden Abschnitt")(index));
                }
                match f.protection {
                    Protection::Point(p, _) => {
                        if p.index().is_none_or(|x| x >= self.points)
                            || all_points().any(|(o, _)| o == p)
                        {
                            return Err(bad("Schutzweiche")(index));
                        }
                    }
                    Protection::Signal(s) => {
                        if s.index().is_none_or(|x| x >= self.signals) || s.raw() == raw.start {
                            return Err(bad("Schutzsignal")(index));
                        }
                    }
                }
            }
            if flank_count != raw.flank.len {
                return Err(bad("Flankenschutz-Eintrag")(index));
            }

            // Bahnübergänge.
            for (i, lx) in view.level_crossings().enumerate() {
                let tvd = self.lx_tvd(lx);
                if !tvd.is_some_and(|t| path_tvd(t) || overlap_tvd(t))
                    || view
                        .level_crossings()
                        .skip(i.saturating_add(1))
                        .any(|o| o == lx)
                {
                    return Err(bad("Bahnübergang")(index));
                }
            }
        }
        Ok(())
    }

    fn check_borders(&self) -> Result<(), PlanError> {
        for index in 0..self.borders {
            let bad = PlanError::InvalidReference {
                what: "Grenze",
                index,
            };
            let id = BorderId::from_index(index).ok_or(PlanError::TooLarge)?;
            let flags = self.border_word(id, 0).ok_or(bad)?;
            if flags & !BORDER_INITIAL != 0 {
                return Err(bad);
            }
            let entry = self.border_entry_signal(id).ok_or(bad)?;
            if !self
                .signal_kind(entry)
                .is_some_and(SignalKind::delimits_routes)
            {
                return Err(bad);
            }
            for range in [self.border_range(id, 2), self.border_range(id, 4)] {
                let range = range.ok_or(bad)?;
                if add(range.start, range.len)? > self.pool_words {
                    return Err(bad);
                }
            }
            let tvds = || self.border_approach_tvds(id);
            if tvds().len() == 0 {
                return Err(bad);
            }
            for (i, t) in tvds().enumerate() {
                if !self.valid_tvd(t.raw()) || tvds().skip(i.saturating_add(1)).any(|x| x == t) {
                    return Err(bad);
                }
            }
            let points = || self.border_approach_points(id);
            for (i, (p, _)) in points().enumerate() {
                if p.index().is_none_or(|x| x >= self.points)
                    || points().skip(i.saturating_add(1)).any(|(o, _)| o == p)
                    || !self.point_tvd(p).is_some_and(|t| tvds().any(|x| x == t))
                {
                    return Err(bad);
                }
            }
        }
        Ok(())
    }

    fn border_word(&self, id: BorderId, word: usize) -> Option<u32> {
        let index = id.index()?;
        if index >= self.borders {
            return None;
        }
        read_word(
            self.bytes,
            self.border_off,
            index.checked_mul(BORDER_WORDS)?.checked_add(word)?,
        )
    }

    fn border_range(&self, id: BorderId, word: usize) -> Option<Range> {
        Some(Range {
            start: usize::try_from(self.border_word(id, word)?).ok()?,
            len: usize::try_from(self.border_word(id, word.checked_add(1)?)?).ok()?,
        })
    }

    fn pool_words_of(&self, range: Option<Range>) -> Words<'a> {
        let range = range.unwrap_or_default();
        Words {
            bytes: self.bytes,
            base: self.pool_off,
            next: range.start,
            end: range.start.saturating_add(range.len),
        }
    }

    /// Anzahl Grenzen zu Nachbarbereichen.
    pub const fn border_count(&self) -> usize {
        self.borders
    }

    /// Name einer Grenze (z. B. `">Strecke"`).
    pub fn border_name(&self, id: BorderId) -> Option<&'a str> {
        self.name_at(self.group_start(5), id.index(), self.borders)
    }

    /// Grenze mit dem Namen `name`.
    pub fn find_border(&self, name: &str) -> Option<BorderId> {
        Self::find(name, self.borders, |i| {
            BorderId::from_index(i).and_then(|id| self.border_name(id))
        })
        .and_then(BorderId::from_index)
    }

    /// Übernimmt dieser Bereich beim Anlauf die Erlaubnis für die Grenze?
    pub fn border_initial(&self, id: BorderId) -> bool {
        self.border_word(id, 0)
            .is_some_and(|f| f & BORDER_INITIAL != 0)
    }

    /// Einfahrsignal hinter der Grenze: Hier halten Züge, die aus dem Nachbarbereich kommen.
    pub fn border_entry_signal(&self, id: BorderId) -> Option<SignalId> {
        self.border_word(id, 1).map(SignalId::new)
    }

    /// Abschnitte des Annäherungsabschnitts: von der Grenze bis zum Einfahrsignal und
    /// dessen Durchrutschweg.
    pub fn border_approach_tvds(&self, id: BorderId) -> TvdEntries<'a> {
        TvdEntries(self.pool_words_of(self.border_range(id, 2)))
    }

    /// Weichenlagen im Annäherungsabschnitt.
    pub fn border_approach_points(&self, id: BorderId) -> PointEntries<'a> {
        PointEntries(self.pool_words_of(self.border_range(id, 4)))
    }

    fn conflict_word(&self, route: usize, word: usize) -> Option<u64> {
        let row = route.checked_mul(self.conflict_words)?.checked_add(word)?;
        read_u64(
            self.bytes,
            self.conflict_off.checked_add(row.checked_mul(8)?)?,
        )
    }

    fn conflict_bit(&self, a: usize, b: usize) -> Option<bool> {
        let word = self.conflict_word(a, b.checked_div(WORD_BITS)?)?;
        let bit = u32::try_from(b.checked_rem(WORD_BITS)?).ok()?;
        Some(word.checked_shr(bit)? & 1 == 1)
    }

    fn check_conflicts(&self) -> Result<(), PlanError> {
        for a in 0..self.routes {
            let used_in_last = self.routes.checked_rem(WORD_BITS).unwrap_or(0);
            if used_in_last != 0 {
                let last = self.conflict_words.saturating_sub(1);
                let word = self
                    .conflict_word(a, last)
                    .ok_or(PlanError::ConflictMatrix(a))?;
                let shift = u32::try_from(used_in_last).map_err(|_| PlanError::TooLarge)?;
                if word.checked_shr(shift).unwrap_or(0) != 0 {
                    return Err(PlanError::ConflictMatrix(a));
                }
            }
            if self.conflict_bit(a, a) != Some(false) {
                return Err(PlanError::ConflictMatrix(a));
            }
            for b in a.saturating_add(1)..self.routes {
                let ab = self
                    .conflict_bit(a, b)
                    .ok_or(PlanError::ConflictMatrix(a))?;
                let ba = self
                    .conflict_bit(b, a)
                    .ok_or(PlanError::ConflictMatrix(b))?;
                if ab != ba {
                    return Err(PlanError::ConflictMatrix(a));
                }
            }
        }
        Ok(())
    }

    fn check_names(&self) -> Result<(), PlanError> {
        for i in 0..self.names_count {
            if self.name(i).is_none() {
                return Err(PlanError::InvalidName(i));
            }
        }
        Ok(())
    }

    fn name(&self, index: usize) -> Option<&'a str> {
        let entry = self.names_off.checked_add(index.checked_mul(8)?)?;
        let off = usize::try_from(read_u32(self.bytes, entry)?).ok()?;
        let len = usize::try_from(read_u32(self.bytes, entry.checked_add(4)?)?).ok()?;
        let start = self.names_data_off.checked_add(off)?;
        let end = start.checked_add(len)?;
        if end > self.names_end {
            return None;
        }
        core::str::from_utf8(self.bytes.get(start..end)?).ok()
    }

    fn raw_route(&self, index: usize) -> Option<RawRoute> {
        if index >= self.routes {
            return None;
        }
        let base = self.route_off.checked_add(index.checked_mul(ROUTE_LEN)?)?;
        let w = |i| read_word(self.bytes, base, i);
        let range = |i: usize| -> Option<Range> {
            Some(Range {
                start: usize::try_from(w(i)?).ok()?,
                len: usize::try_from(w(i.checked_add(1)?)?).ok()?,
            })
        };
        Some(RawRoute {
            start: w(r::START)?,
            end_kind: w(r::END_KIND)?,
            end_signal: w(r::END_SIGNAL)?,
            speed: w(r::SPEED)?,
            points: range(r::POINTS)?,
            tvds: range(r::TVDS)?,
            overlap_tvds: range(r::OVERLAP_TVDS)?,
            overlap_points: range(r::OVERLAP_POINTS)?,
            flank: range(r::FLANK)?,
            lxs: range(r::LXS)?,
        })
    }

    fn signal_word(&self, id: SignalId, word: usize) -> Option<u32> {
        let index = id.index()?;
        if index >= self.signals {
            return None;
        }
        read_word(
            self.bytes,
            self.signal_off,
            index.checked_mul(SIGNAL_WORDS)?.checked_add(word)?,
        )
    }

    /// SHA-256-Prüfsumme des Plans; identifiziert den Plan eindeutig.
    pub fn digest(&self) -> [u8; DIGEST_LEN] {
        let mut out = [0u8; DIGEST_LEN];
        if let Some(stored) = self
            .bytes
            .get(self.body_len..self.body_len.saturating_add(DIGEST_LEN))
        {
            out.copy_from_slice(stored);
        }
        out
    }

    /// Ist das Image signiert?
    pub const fn is_signed(&self) -> bool {
        self.signed
    }

    /// Das gesamte Image.
    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Anzahl Weichen.
    pub const fn point_count(&self) -> usize {
        self.points
    }

    /// Anzahl Signale.
    pub const fn signal_count(&self) -> usize {
        self.signals
    }

    /// Anzahl Gleisfreimeldeabschnitte.
    pub const fn tvd_count(&self) -> usize {
        self.tvds
    }

    /// Anzahl Fahrstraßen.
    pub const fn route_count(&self) -> usize {
        self.routes
    }

    /// Anzahl Bahnübergänge.
    pub const fn lx_count(&self) -> usize {
        self.lxs
    }

    /// Anzahl `u64`-Wörter einer Bitmenge über alle Fahrstraßen.
    pub const fn route_set_words(&self) -> usize {
        self.conflict_words
    }

    /// Name des Netzes.
    pub fn network_name(&self) -> &'a str {
        self.name(0).unwrap_or("")
    }

    fn name_at(
        &self,
        group_start: Option<usize>,
        index: Option<usize>,
        group_len: usize,
    ) -> Option<&'a str> {
        let index = index?;
        if index >= group_len {
            return None;
        }
        self.name(group_start?.checked_add(index)?)
    }

    fn group_start(&self, group: usize) -> Option<usize> {
        [self.points, self.signals, self.tvds, self.routes, self.lxs]
            .iter()
            .take(group)
            .try_fold(1usize, |acc, n| acc.checked_add(*n))
    }

    /// Name einer Weiche.
    pub fn point_name(&self, id: PointId) -> Option<&'a str> {
        self.name_at(self.group_start(0), id.index(), self.points)
    }

    /// Name eines Signals.
    pub fn signal_name(&self, id: SignalId) -> Option<&'a str> {
        self.name_at(self.group_start(1), id.index(), self.signals)
    }

    /// Name eines Gleisfreimeldeabschnitts.
    pub fn tvd_name(&self, id: TvdId) -> Option<&'a str> {
        self.name_at(self.group_start(2), id.index(), self.tvds)
    }

    /// Name einer Fahrstraße.
    pub fn route_name(&self, id: RouteId) -> Option<&'a str> {
        self.name_at(self.group_start(3), id.index(), self.routes)
    }

    /// Name eines Bahnübergangs.
    pub fn lx_name(&self, id: LxId) -> Option<&'a str> {
        self.name_at(self.group_start(4), id.index(), self.lxs)
    }

    fn find(name: &str, count: usize, get: impl Fn(usize) -> Option<&'a str>) -> Option<usize> {
        (0..count).find(|&i| get(i) == Some(name))
    }

    /// Weiche mit dem Namen `name`.
    pub fn find_point(&self, name: &str) -> Option<PointId> {
        Self::find(name, self.points, |i| {
            PointId::from_index(i).and_then(|id| self.point_name(id))
        })
        .and_then(PointId::from_index)
    }

    /// Signal mit dem Namen `name`.
    pub fn find_signal(&self, name: &str) -> Option<SignalId> {
        Self::find(name, self.signals, |i| {
            SignalId::from_index(i).and_then(|id| self.signal_name(id))
        })
        .and_then(SignalId::from_index)
    }

    /// Gleisfreimeldeabschnitt mit dem Namen `name`.
    pub fn find_tvd(&self, name: &str) -> Option<TvdId> {
        Self::find(name, self.tvds, |i| {
            TvdId::from_index(i).and_then(|id| self.tvd_name(id))
        })
        .and_then(TvdId::from_index)
    }

    /// Fahrstraße mit dem Namen `name`.
    pub fn find_route(&self, name: &str) -> Option<RouteId> {
        Self::find(name, self.routes, |i| {
            RouteId::from_index(i).and_then(|id| self.route_name(id))
        })
        .and_then(RouteId::from_index)
    }

    /// Bahnübergang mit dem Namen `name`.
    pub fn find_lx(&self, name: &str) -> Option<LxId> {
        Self::find(name, self.lxs, |i| {
            LxId::from_index(i).and_then(|id| self.lx_name(id))
        })
        .and_then(LxId::from_index)
    }

    /// Gleisfreimeldeabschnitt, in dem eine Weiche liegt.
    pub fn point_tvd(&self, id: PointId) -> Option<TvdId> {
        let index = id.index()?;
        if index >= self.points {
            return None;
        }
        read_word(self.bytes, self.point_tvd_off, index).map(TvdId::new)
    }

    /// Gleisfreimeldeabschnitt, in dem ein Bahnübergang liegt.
    pub fn lx_tvd(&self, id: LxId) -> Option<TvdId> {
        let index = id.index()?;
        if index >= self.lxs {
            return None;
        }
        read_word(self.bytes, self.lx_tvd_off, index).map(TvdId::new)
    }

    /// Art eines Signals.
    pub fn signal_kind(&self, id: SignalId) -> Option<SignalKind> {
        SignalKind::from_code(self.signal_word(id, 0)?)
    }

    /// Signalsystem eines Signals.
    pub fn signal_system(&self, id: SignalId) -> Option<SignalSystem> {
        SignalSystem::from_code(self.signal_word(id, 1)?)
    }

    /// Hauptsignal, das ein Vorsignal ankündigt.
    pub fn signal_announces(&self, id: SignalId) -> Option<SignalId> {
        match self.signal_word(id, 2)? {
            NONE => None,
            s => Some(SignalId::new(s)),
        }
    }

    /// Ist der Abschnitt ein Einfahrabschnitt an einer Netzgrenze? Dort dürfen Züge ohne
    /// Fahrstraße erscheinen.
    pub fn tvd_is_entry(&self, id: TvdId) -> bool {
        id.index()
            .filter(|i| *i < self.tvds)
            .and_then(|i| read_word(self.bytes, self.tvd_flags_off, i))
            .is_some_and(|f| f & TVD_ENTRY != 0)
    }

    /// Fahrstraße `id`.
    pub fn route(&self, id: RouteId) -> Option<RouteView<'a>> {
        let raw = self.raw_route(id.index()?)?;
        Some(RouteView {
            plan: *self,
            id,
            raw,
        })
    }

    /// Alle Fahrstraßen.
    pub fn routes(&self) -> impl Iterator<Item = RouteView<'a>> + '_ {
        (0..self.routes)
            .filter_map(RouteId::from_index)
            .filter_map(|id| self.route(id))
    }
}

/// Sicht auf eine Fahrstraße.
#[derive(Copy, Clone)]
pub struct RouteView<'a> {
    plan: PlanView<'a>,
    id: RouteId,
    raw: RawRoute,
}

impl fmt::Debug for RouteView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RouteView")
            .field("id", &self.id)
            .field("name", &self.name())
            .field("start", &self.start())
            .field("end", &self.end())
            .finish_non_exhaustive()
    }
}

impl<'a> RouteView<'a> {
    fn words(&self, range: Range) -> Words<'a> {
        Words {
            bytes: self.plan.bytes,
            base: self.plan.pool_off,
            next: range.start,
            end: range.start.saturating_add(range.len),
        }
    }

    /// ID der Fahrstraße.
    pub const fn id(&self) -> RouteId {
        self.id
    }

    /// Name der Fahrstraße.
    pub fn name(&self) -> &'a str {
        self.plan.route_name(self.id).unwrap_or("")
    }

    /// Startsignal.
    pub const fn start(&self) -> SignalId {
        SignalId::new(self.raw.start)
    }

    /// Ziel.
    pub const fn end(&self) -> RouteEnd {
        match self.raw.end_kind {
            END_SIGNAL => RouteEnd::Signal(SignalId::new(self.raw.end_signal)),
            END_BOUNDARY => RouteEnd::Boundary,
            END_BORDER => RouteEnd::Border(BorderId::new(self.raw.end_signal)),
            _ => RouteEnd::BufferStop,
        }
    }

    /// Zulässige Geschwindigkeit, falls die Fahrstraße beschränkt ist (z. B. über einen
    /// abzweigenden Weichenstrang).
    pub fn speed(&self) -> Option<Speed> {
        u16::try_from(self.raw.speed)
            .ok()
            .filter(|v| *v != 0)
            .map(Speed::from_kmh)
    }

    /// Erforderliche Weichenlagen im Fahrweg.
    pub fn points(&self) -> PointEntries<'a> {
        PointEntries(self.words(self.raw.points))
    }

    /// Gleisfreimeldeabschnitte des Fahrwegs in Fahrreihenfolge.
    pub fn tvds(&self) -> TvdEntries<'a> {
        TvdEntries(self.words(self.raw.tvds))
    }

    /// Gleisfreimeldeabschnitte des Durchrutschwegs in Fahrreihenfolge.
    pub fn overlap_tvds(&self) -> TvdEntries<'a> {
        TvdEntries(self.words(self.raw.overlap_tvds))
    }

    /// Erforderliche Weichenlagen im Durchrutschweg.
    pub fn overlap_points(&self) -> PointEntries<'a> {
        PointEntries(self.words(self.raw.overlap_points))
    }

    /// Flankenschutzmaßnahmen.
    pub fn flank(&self) -> FlankEntries<'a> {
        let words = Words {
            bytes: self.plan.bytes,
            base: self.plan.pool_off,
            next: self.raw.flank.start,
            end: self
                .raw
                .flank
                .start
                .saturating_add(self.raw.flank.len.saturating_mul(FLANK_WORDS)),
        };
        FlankEntries(words)
    }

    /// Bahnübergänge im Fahrweg und Durchrutschweg.
    pub fn level_crossings(&self) -> LxEntries<'a> {
        LxEntries(self.words(self.raw.lxs))
    }

    /// Ist diese Fahrstraße mit `other` feindlich?
    pub fn conflicts_with(&self, other: RouteId) -> bool {
        // Nicht lesbar heißt: feindlich (sichere Seite).
        match (self.id.index(), other.index()) {
            (Some(a), Some(b)) if b < self.plan.routes => {
                self.plan.conflict_bit(a, b).unwrap_or(true)
            }
            _ => true,
        }
    }

    /// Ist diese Fahrstraße mit mindestens einer Fahrstraße aus `active` feindlich?
    pub fn conflicts_with_any(&self, active: &BitSet<'_>) -> bool {
        let Some(a) = self.id.index() else {
            return true;
        };
        active.words().iter().enumerate().any(|(w, bits)| {
            *bits != 0
                && self
                    .plan
                    .conflict_word(a, w)
                    .is_none_or(|row| row & bits != 0)
        })
    }

    /// Alle mit dieser feindlichen Fahrstraßen.
    pub fn conflicts(&self) -> impl Iterator<Item = RouteId> + '_ {
        (0..self.plan.routes)
            .filter_map(RouteId::from_index)
            .filter(|other| *other != self.id && self.conflicts_with(*other))
    }
}

/// Wörter einer Liste im Pool.
#[derive(Clone, Debug)]
struct Words<'a> {
    bytes: &'a [u8],
    base: usize,
    next: usize,
    end: usize,
}

impl Iterator for Words<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        if self.next >= self.end {
            return None;
        }
        let word = read_word(self.bytes, self.base, self.next)?;
        self.next = self.next.saturating_add(1);
        Some(word)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.end.saturating_sub(self.next);
        (n, Some(n))
    }
}

/// Iterator über Weichenlagen.
#[derive(Clone, Debug)]
pub struct PointEntries<'a>(Words<'a>);

impl Iterator for PointEntries<'_> {
    type Item = (PointId, PointPosition);

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(decode_point)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl ExactSizeIterator for PointEntries<'_> {}

/// Iterator über Gleisfreimeldeabschnitte.
#[derive(Clone, Debug)]
pub struct TvdEntries<'a>(Words<'a>);

impl Iterator for TvdEntries<'_> {
    type Item = TvdId;

    fn next(&mut self) -> Option<TvdId> {
        self.0.next().map(TvdId::new)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl ExactSizeIterator for TvdEntries<'_> {}

/// Iterator über Bahnübergänge.
#[derive(Clone, Debug)]
pub struct LxEntries<'a>(Words<'a>);

impl Iterator for LxEntries<'_> {
    type Item = LxId;

    fn next(&mut self) -> Option<LxId> {
        self.0.next().map(LxId::new)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl ExactSizeIterator for LxEntries<'_> {}

/// Iterator über Flankenschutzmaßnahmen. Ungültige Einträge beenden die Iteration; sie
/// kommen in einem geprüften Plan nicht vor.
#[derive(Clone, Debug)]
pub struct FlankEntries<'a>(Words<'a>);

impl Iterator for FlankEntries<'_> {
    type Item = Flank;

    fn next(&mut self) -> Option<Flank> {
        let kind = self.0.next()?;
        let element = self.0.next()?;
        let protects = TvdId::new(self.0.next()?);
        let protection = match kind {
            FLANK_POINT => {
                let (p, pos) = decode_point(element);
                Protection::Point(p, pos)
            }
            FLANK_SIGNAL => Protection::Signal(SignalId::new(element)),
            _ => return None,
        };
        Some(Flank {
            protection,
            protects,
        })
    }
}
