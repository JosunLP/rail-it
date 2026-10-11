//! Einheiten und Zeit.

use railit_types::{Duration, Length, Speed, Tick, TrackId};

#[test]
fn length_arithmetic_is_checked() {
    let a = Length::from_m(12).unwrap();
    assert_eq!(a.as_mm(), 12_000);
    assert_eq!(
        a.checked_add(Length::from_mm(345)),
        Some(Length::from_mm(12_345))
    );
    assert_eq!(
        Length::from_mm(u32::MAX).checked_add(Length::from_mm(1)),
        None
    );
    assert_eq!(Length::ZERO.checked_sub(Length::from_mm(1)), None);
    assert_eq!(Length::from_m(u32::MAX), None);
}

#[test]
fn display_formats() {
    assert_eq!(Length::from_mm(12_345).to_string(), "12.345 m");
    assert_eq!(Length::from_mm(7_000).to_string(), "7 m");
    assert_eq!(Length::from_mm(5).to_string(), "0.005 m");
    assert_eq!(Speed::from_kmh(80).to_string(), "80 km/h");
    assert_eq!(TrackId::new(3).to_string(), "Track#3");
}

#[test]
fn tick_since_saturates_to_zero() {
    let t1 = Tick::from_millis(1_000);
    let t2 = t1.checked_add(Duration::from_secs(2).unwrap()).unwrap();
    assert_eq!(t2.saturating_since(t1), Duration::from_millis(2_000));
    assert_eq!(t1.saturating_since(t2), Duration::ZERO);
    assert_eq!(
        Tick::from_millis(u64::MAX).checked_add(Duration::from_millis(1)),
        None
    );
}

#[test]
fn id_index_roundtrip() {
    let id = TrackId::from_index(42).unwrap();
    assert_eq!(id.index(), Some(42));
    assert_eq!(id.raw(), 42);
}
