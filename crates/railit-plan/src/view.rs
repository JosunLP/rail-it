//! Zero-Copy-Sicht auf ein Plan-Image.

use core::fmt;

use ed25519_dalek::{Signature, VerifyingKey};
use railit_types::bitset::{BitSet, WORD_BITS, words_for};
use railit_types::{PointId, PointPosition, RouteId, SignalId, TvdId};
use sha2::{Digest, Sha256};

use crate::error::PlanError;
use crate::format::{
    DIGEST_LEN, END_BOUNDARY, END_BUFFER_STOP, END_SIGNAL, FLAG_SIGNED, H_BODY_LEN, H_CONFLICT_OFF,
    H_FLAGS, H_NAMES_LEN, H_NAMES_OFF, H_POINT_TVD_OFF, H_POINTS, H_POOL_OFF, H_POOL_WORDS,
    H_ROUTE_OFF, H_ROUTES, H_SIGNALS, H_TVDS, H_VERSION, HEADER_LEN, MAGIC, NO_SIGNAL, ROUTE_LEN,
    SIGNATURE_LEN, VERSION, decode_point, read_u32, read_u64, read_word,
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

/// Geprüfte, unveränderliche Sicht auf ein Plan-Image.
///
/// Eine `PlanView` existiert nur, wenn [`PlanView::parse`] Struktur, Prüfsumme und alle
/// Verweise erfolgreich geprüft hat (REQ-PLN-002). Die Zugriffsmethoden geben trotzdem
/// `Option` zurück, damit der Kern nie in einen Panikpfad gerät; nach erfolgreicher Prüfung
/// liefern sie für gültige IDs immer `Some`.
#[derive(Copy, Clone)]
pub struct PlanView<'a> {
    bytes: &'a [u8],
    body_len: usize,
    points: usize,
    signals: usize,
    tvds: usize,
    routes: usize,
    route_off: usize,
    pool_off: usize,
    pool_words: usize,
    conflict_off: usize,
    conflict_words: usize,
    point_tvd_off: usize,
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
        let flags = h(H_FLAGS)?;
        let signed = flags & FLAG_SIGNED != 0;
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
        let conflict_words = words_for(routes);
        let names_count = add(add(add(add(1, points)?, signals)?, tvds)?, routes)?;

        let mut view = Self {
            bytes,
            body_len,
            points,
            signals,
            tvds,
            routes,
            route_off: to_usize(h(H_ROUTE_OFF)?)?,
            pool_off: to_usize(h(H_POOL_OFF)?)?,
            pool_words: to_usize(h(H_POOL_WORDS)?)?,
            conflict_off: to_usize(h(H_CONFLICT_OFF)?)?,
            conflict_words,
            point_tvd_off: to_usize(h(H_POINT_TVD_OFF)?)?,
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
            "Feindlichkeiten",
            view.conflict_off,
            mul(mul(routes, conflict_words)?, 8)?,
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
        view.check_points()?;
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

    fn check_points(&self) -> Result<(), PlanError> {
        for index in 0..self.points {
            let tvd = read_word(self.bytes, self.point_tvd_off, index);
            if !tvd.is_some_and(|t| to_usize(t).is_ok_and(|t| t < self.tvds)) {
                return Err(PlanError::InvalidReference {
                    what: "TVD einer Weiche",
                    index,
                });
            }
        }
        Ok(())
    }

    fn check_routes(&self) -> Result<(), PlanError> {
        let bad = |what| move |index| PlanError::InvalidReference { what, index };
        for index in 0..self.routes {
            let raw = self
                .raw_route(index)
                .ok_or(PlanError::SectionOutOfBounds("Fahrstraßen"))?;
            if to_usize(raw.start)? >= self.signals {
                return Err(bad("Startsignal")(index));
            }
            match raw.end_kind {
                END_SIGNAL if to_usize(raw.end_signal)? < self.signals => {}
                END_BOUNDARY | END_BUFFER_STOP if raw.end_signal == NO_SIGNAL => {}
                _ => return Err(bad("Ziel")(index)),
            }
            if add(raw.points_start, raw.points_len)? > self.pool_words {
                return Err(bad("Weichenliste")(index));
            }
            if add(raw.tvds_start, raw.tvds_len)? > self.pool_words {
                return Err(bad("TVD-Liste")(index));
            }
            if raw.tvds_len == 0 {
                return Err(PlanError::EmptyRoute(index));
            }
            let points =
                PointEntries::new(self.bytes, self.pool_off, raw.points_start, raw.points_len);
            for (i, (point, _)) in points.clone().enumerate() {
                if point.index().is_none_or(|p| p >= self.points) {
                    return Err(bad("Weiche")(index));
                }
                // Jede Weiche höchstens einmal je Fahrstraße (sonst widersprüchliche Lagen).
                if points
                    .clone()
                    .skip(i.saturating_add(1))
                    .any(|(other, _)| other == point)
                {
                    return Err(bad("doppelte Weiche")(index));
                }
            }
            let tvds = TvdEntries::new(self.bytes, self.pool_off, raw.tvds_start, raw.tvds_len);
            // Jede Weiche liegt in einem Abschnitt der Fahrstraße; sonst wäre sie ohne
            // Gleisfreimeldung verschlossen und könnte nie aufgelöst werden.
            for (point, _) in points.clone() {
                let point_tvd = read_word(
                    self.bytes,
                    self.point_tvd_off,
                    point.index().unwrap_or(usize::MAX),
                );
                if !point_tvd.is_some_and(|pt| tvds.clone().any(|t| t.raw() == pt)) {
                    return Err(bad("Weiche außerhalb der Abschnitte")(index));
                }
            }
            for (i, tvd) in tvds.clone().enumerate() {
                if tvd.index().is_none_or(|t| t >= self.tvds) {
                    return Err(bad("TVD")(index));
                }
                if tvds
                    .clone()
                    .skip(i.saturating_add(1))
                    .any(|other| other == tvd)
                {
                    return Err(bad("doppelter TVD")(index));
                }
            }
        }
        Ok(())
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
            // Keine Bits jenseits der letzten Fahrstraße.
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
        Some(RawRoute {
            start: w(0)?,
            end_kind: w(1)?,
            end_signal: w(2)?,
            points_start: usize::try_from(w(3)?).ok()?,
            points_len: usize::try_from(w(4)?).ok()?,
            tvds_start: usize::try_from(w(5)?).ok()?,
            tvds_len: usize::try_from(w(6)?).ok()?,
        })
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
        group_start: usize,
        index: Option<usize>,
        group_len: usize,
    ) -> Option<&'a str> {
        let index = index?;
        if index >= group_len {
            return None;
        }
        self.name(group_start.checked_add(index)?)
    }

    /// Name einer Weiche.
    pub fn point_name(&self, id: PointId) -> Option<&'a str> {
        self.name_at(1, id.index(), self.points)
    }

    /// Name eines Signals.
    pub fn signal_name(&self, id: SignalId) -> Option<&'a str> {
        self.name_at(self.points.checked_add(1)?, id.index(), self.signals)
    }

    /// Name eines Gleisfreimeldeabschnitts.
    pub fn tvd_name(&self, id: TvdId) -> Option<&'a str> {
        self.name_at(
            self.points.checked_add(1)?.checked_add(self.signals)?,
            id.index(),
            self.tvds,
        )
    }

    /// Name einer Fahrstraße.
    pub fn route_name(&self, id: RouteId) -> Option<&'a str> {
        let start = self
            .points
            .checked_add(1)?
            .checked_add(self.signals)?
            .checked_add(self.tvds)?;
        self.name_at(start, id.index(), self.routes)
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

    /// Gleisfreimeldeabschnitt, in dem eine Weiche liegt.
    pub fn point_tvd(&self, id: PointId) -> Option<TvdId> {
        let index = id.index()?;
        if index >= self.points {
            return None;
        }
        read_word(self.bytes, self.point_tvd_off, index).map(TvdId::new)
    }

    /// Fahrstraße `id`.
    pub fn route(&self, id: RouteId) -> Option<RouteView<'a>> {
        let raw = self.raw_route(id.index()?)?;
        let end = match raw.end_kind {
            END_SIGNAL => RouteEnd::Signal(SignalId::new(raw.end_signal)),
            END_BOUNDARY => RouteEnd::Boundary,
            END_BUFFER_STOP => RouteEnd::BufferStop,
            _ => return None,
        };
        Some(RouteView {
            plan: *self,
            id,
            start: SignalId::new(raw.start),
            end,
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

#[derive(Copy, Clone, Debug)]
struct RawRoute {
    start: u32,
    end_kind: u32,
    end_signal: u32,
    points_start: usize,
    points_len: usize,
    tvds_start: usize,
    tvds_len: usize,
}

/// Sicht auf eine Fahrstraße.
#[derive(Copy, Clone)]
pub struct RouteView<'a> {
    plan: PlanView<'a>,
    id: RouteId,
    start: SignalId,
    end: RouteEnd,
    raw: RawRoute,
}

impl fmt::Debug for RouteView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RouteView")
            .field("id", &self.id)
            .field("name", &self.name())
            .field("start", &self.start)
            .field("end", &self.end)
            .finish_non_exhaustive()
    }
}

impl<'a> RouteView<'a> {
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
        self.start
    }

    /// Ziel.
    pub const fn end(&self) -> RouteEnd {
        self.end
    }

    /// Erforderliche Weichenlagen.
    pub fn points(&self) -> PointEntries<'a> {
        PointEntries::new(
            self.plan.bytes,
            self.plan.pool_off,
            self.raw.points_start,
            self.raw.points_len,
        )
    }

    /// Gleisfreimeldeabschnitte in Fahrreihenfolge.
    pub fn tvds(&self) -> TvdEntries<'a> {
        TvdEntries::new(
            self.plan.bytes,
            self.plan.pool_off,
            self.raw.tvds_start,
            self.raw.tvds_len,
        )
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

/// Iterator über die Weichenlagen einer Fahrstraße.
#[derive(Clone, Debug)]
pub struct PointEntries<'a> {
    bytes: &'a [u8],
    base: usize,
    next: usize,
    end: usize,
}

impl<'a> PointEntries<'a> {
    fn new(bytes: &'a [u8], pool_off: usize, start: usize, len: usize) -> Self {
        Self {
            bytes,
            base: pool_off,
            next: start,
            end: start.saturating_add(len),
        }
    }
}

impl Iterator for PointEntries<'_> {
    type Item = (PointId, PointPosition);

    fn next(&mut self) -> Option<Self::Item> {
        if self.next >= self.end {
            return None;
        }
        let entry = read_word(self.bytes, self.base, self.next)?;
        self.next = self.next.saturating_add(1);
        Some(decode_point(entry))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.end.saturating_sub(self.next);
        (n, Some(n))
    }
}

impl ExactSizeIterator for PointEntries<'_> {}

/// Iterator über die Gleisfreimeldeabschnitte einer Fahrstraße.
#[derive(Clone, Debug)]
pub struct TvdEntries<'a> {
    bytes: &'a [u8],
    base: usize,
    next: usize,
    end: usize,
}

impl<'a> TvdEntries<'a> {
    fn new(bytes: &'a [u8], pool_off: usize, start: usize, len: usize) -> Self {
        Self {
            bytes,
            base: pool_off,
            next: start,
            end: start.saturating_add(len),
        }
    }
}

impl Iterator for TvdEntries<'_> {
    type Item = TvdId;

    fn next(&mut self) -> Option<TvdId> {
        if self.next >= self.end {
            return None;
        }
        let entry = read_word(self.bytes, self.base, self.next)?;
        self.next = self.next.saturating_add(1);
        Some(TvdId::new(entry))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.end.saturating_sub(self.next);
        (n, Some(n))
    }
}

impl ExactSizeIterator for TvdEntries<'_> {}
