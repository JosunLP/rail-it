//! RaSTA über UDP mit einem oder mehreren Kanälen (REQ-COM-011).
//!
//! Mit einem Kanal gehen die PDUs der Sicherheitsschicht unverändert über einen Socket. Mit
//! mehreren Kanälen (je Kanal ein eigener Socket, in der Anlage ein eigenes Netz) verpackt die
//! Redundanzschicht ([`railit_rasta::redundancy`]) jede PDU in einen Rahmen, sendet ihn über
//! alle Kanäle und liefert beim Empfang jede PDU genau einmal in Reihenfolge aus.
//!
//! Die Redundanzschicht läuft je Paar von RaSTA-Kennungen (Absender, Empfänger), weil über
//! dieselben Sockets viele Verbindungen laufen (je Objektsteuerung eine).

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::time::Instant;

use railit_rasta::MAX_PDU;
use railit_rasta::redundancy::{MAX_CHANNELS, MAX_FRAME, RedConfig, Redundancy};
use serde::Serialize;

/// Zustand eines Kanals.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ChannelStatus {
    /// Kanal (0 …).
    pub channel: usize,
    /// Eigene Adresse.
    pub local: Option<String>,
    /// Empfangene Rahmen über alle Verbindungen.
    pub received: u64,
    /// Fehlende Rahmen im letzten Diagnosefenster (Summe über alle Verbindungen).
    pub missed: u64,
    /// Am Prüfstand gestört.
    pub down: bool,
}

/// UDP-Transport mit Redundanzschicht.
pub struct RedundantUdp {
    sockets: Vec<UdpSocket>,
    /// Gegenstellen je Kanal: fest (Stellwerk) oder je RaSTA-Kennung gelernt (Feldrechner).
    fixed: bool,
    peers: HashMap<u32, Vec<Option<SocketAddr>>>,
    layers: HashMap<(u32, u32), Redundancy>,
    cfg: RedConfig,
    clock: Instant,
    /// Gestörte Kanäle (Prüfstand): weder Senden noch Empfang.
    down: Vec<bool>,
}

fn bind_any(remote: SocketAddr) -> std::io::Result<UdpSocket> {
    let local: SocketAddr = if remote.is_ipv4() {
        ([0, 0, 0, 0], 0).into()
    } else {
        ([0u16; 8], 0).into()
    };
    let socket = UdpSocket::bind(local)?;
    socket.connect(remote)?;
    Ok(socket)
}

impl RedundantUdp {
    /// Stellwerksseite: je Kanal ein Socket zu `remotes[k]`.
    ///
    /// # Errors
    /// Keine oder zu viele Adressen, Socketfehler.
    pub fn connect(remotes: &[SocketAddr]) -> Result<Self, String> {
        Self::check(remotes.len())?;
        let sockets = remotes
            .iter()
            .map(|r| bind_any(*r).map_err(|e| format!("UDP {r}: {e}")))
            .collect::<Result<Vec<_>, _>>()?;
        Self::with(sockets, true)
    }

    /// Feldseite: je Kanal ein Socket an `listens[k]`; die Gegenstellen lernt sie aus den
    /// empfangenen PDUs.
    ///
    /// # Errors
    /// Keine oder zu viele Adressen, Socketfehler.
    pub fn bind(listens: &[SocketAddr]) -> Result<Self, String> {
        Self::check(listens.len())?;
        let sockets = listens
            .iter()
            .map(|l| UdpSocket::bind(l).map_err(|e| format!("UDP {l}: {e}")))
            .collect::<Result<Vec<_>, _>>()?;
        Self::with(sockets, false)
    }

    fn check(n: usize) -> Result<(), String> {
        if n == 0 || n > MAX_CHANNELS {
            return Err(format!("RaSTA: 1 bis {MAX_CHANNELS} Kanäle, nicht {n}"));
        }
        Ok(())
    }

    fn with(sockets: Vec<UdpSocket>, fixed: bool) -> Result<Self, String> {
        for s in &sockets {
            s.set_nonblocking(true).map_err(|e| format!("UDP: {e}"))?;
        }
        let n = sockets.len();
        Ok(Self {
            cfg: RedConfig {
                channels: sockets.len(),
                ..RedConfig::default()
            },
            sockets,
            fixed,
            peers: HashMap::new(),
            layers: HashMap::new(),
            clock: Instant::now(),
            down: vec![false; n],
        })
    }

    /// Zahl der Kanäle.
    pub fn channel_count(&self) -> usize {
        self.sockets.len()
    }

    /// Stört einen Kanal (Prüfstand): Was über ihn ginge, geht verloren.
    ///
    /// # Errors
    /// Unbekannter Kanal.
    pub fn set_down(&mut self, channel: usize, down: bool) -> Result<(), String> {
        let slot = self
            .down
            .get_mut(channel)
            .ok_or_else(|| format!("Kanal {channel} gibt es nicht"))?;
        *slot = down;
        Ok(())
    }

    fn is_down(&self, channel: usize) -> bool {
        self.down.get(channel).copied().unwrap_or(true)
    }

    /// Mit Redundanzschicht (mehr als ein Kanal)?
    pub fn redundant(&self) -> bool {
        self.sockets.len() > 1
    }

    /// Eigene Adressen je Kanal.
    pub fn local_addrs(&self) -> Vec<Option<SocketAddr>> {
        self.sockets.iter().map(|s| s.local_addr().ok()).collect()
    }

    fn ms(&self) -> u32 {
        // Die Redundanzschicht rechnet mit umlaufenden Millisekunden.
        u32::try_from(self.clock.elapsed().as_millis() % (1u128 << 32)).unwrap_or(0)
    }

    fn layer(&mut self, key: (u32, u32)) -> &mut Redundancy {
        let cfg = self.cfg;
        self.layers
            .entry(key)
            .or_insert_with(|| Redundancy::new(cfg))
    }

    /// Sendet eine PDU der Sicherheitsschicht über alle Kanäle. Verlust ist für RaSTA ein
    /// gewöhnlicher Fall; Sendefehler werden nicht gemeldet.
    pub fn send(&mut self, pdu: &[u8]) {
        let Some((receiver, sender)) = railit_rasta::addresses(pdu) else {
            return;
        };
        let mut frame = [0u8; MAX_FRAME];
        let bytes = if self.redundant() {
            let Some(len) = self.layer((sender, receiver)).wrap(pdu, &mut frame) else {
                return;
            };
            frame.get(..len).unwrap_or_default()
        } else {
            pdu
        };
        for (k, socket) in self.sockets.iter().enumerate() {
            if self.is_down(k) {
                continue;
            }
            if self.fixed {
                let _ = socket.send(bytes);
            } else if let Some(Some(to)) = self.peers.get(&receiver).and_then(|p| p.get(k)) {
                let _ = socket.send_to(bytes, to);
            }
        }
    }

    /// Holt alle eingegangenen Rahmen ab und liefert die PDUs (mit der Absenderadresse des
    /// Kanals, über den sie kamen) genau einmal in Reihenfolge aus.
    pub fn receive(&mut self, deliver: &mut dyn FnMut(SocketAddr, &[u8])) {
        let mut buf = [0u8; MAX_FRAME];
        let now = self.ms();
        for k in 0..self.sockets.len() {
            while let Some(Ok((n, from))) = self.sockets.get(k).map(|s| s.recv_from(&mut buf)) {
                let Some(frame) = buf.get(..n) else {
                    continue;
                };
                if self.is_down(k) {
                    continue;
                }
                self.accept(now, k, from, frame, deliver);
            }
        }
        if self.redundant() {
            for layer in self.layers.values_mut() {
                // Zurückgestellte PDUs nach Ablauf von t_seq; Absender ist hier unerheblich.
                let from = SocketAddr::from(([0, 0, 0, 0], 0));
                layer.tick(now, &mut |pdu| deliver(from, pdu));
            }
        }
    }

    fn accept(
        &mut self,
        now: u32,
        channel: usize,
        from: SocketAddr,
        frame: &[u8],
        deliver: &mut dyn FnMut(SocketAddr, &[u8]),
    ) {
        if !self.redundant() {
            if frame.len() <= MAX_PDU {
                self.learn(channel, from, frame);
                deliver(from, frame);
            }
            return;
        }
        let Some((_, pdu)) = railit_rasta::redundancy::parse(frame) else {
            return;
        };
        let Some((receiver, sender)) = railit_rasta::addresses(pdu) else {
            return;
        };
        self.learn(channel, from, pdu);
        self.layer((receiver, sender))
            .receive(now, channel, frame, &mut |p| deliver(from, p));
    }

    /// Merkt sich die Adresse des Absenders je Kanal (Feldseite).
    fn learn(&mut self, channel: usize, from: SocketAddr, pdu: &[u8]) {
        if self.fixed {
            return;
        }
        let Some((_, sender)) = railit_rasta::addresses(pdu) else {
            return;
        };
        let n = self.sockets.len();
        let slots = self.peers.entry(sender).or_insert_with(|| vec![None; n]);
        if let Some(slot) = slots.get_mut(channel) {
            *slot = Some(from);
        }
    }

    /// Zustand je Kanal.
    pub fn status(&self) -> Vec<ChannelStatus> {
        (0..self.sockets.len())
            .map(|k| {
                let (received, missed) = self
                    .layers
                    .values()
                    .filter_map(|l| l.diagnosis().get(k))
                    .fold((0u64, 0u64), |(r, m), d| {
                        (
                            r.saturating_add(d.received),
                            m.saturating_add(u64::from(d.missed_last_window)),
                        )
                    });
                ChannelStatus {
                    channel: k,
                    local: self
                        .sockets
                        .get(k)
                        .and_then(|s| s.local_addr().ok())
                        .map(|a| a.to_string()),
                    received,
                    missed,
                    down: self.is_down(k),
                }
            })
            .collect()
    }
}
