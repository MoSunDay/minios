//! Clock-event-device abstraction (21 §4): the UP reference device is
//! the PIT in periodic mode at 100 Hz; one-shot programming waits for
//! the M8 LAPIC timer (the UP gate must not depend on QEMU APIC
//! configs). Expiry of software timers (hrtimer/wheel) is driven from
//! the periodic tick in time::tick.

/// Feature bits (Linux clock_event_device conventions).
pub const CLOCK_EVT_FEAT_PERIODIC: u32 = 1 << 0;
#[allow(dead_code)] // M8: LAPIC one-shot programming
pub const CLOCK_EVT_FEAT_ONESHOT: u32 = 1 << 1;

/// Static description of the active event device (PIT channel 0).
pub struct ClockEventDevice {
    pub name: &'static str,
    pub features: u32,
    pub rating: u32,
    /// Periodic interrupt frequency in Hz (programmed at init).
    pub hz: u32,
}

pub static PIT_EVENT: ClockEventDevice = ClockEventDevice {
    name: "pit",
    features: CLOCK_EVT_FEAT_PERIODIC,
    rating: 100,
    hz: 100,
};

/// The active event device (PIT is the only one in M1R/M3 UP).
pub fn active() -> &'static ClockEventDevice {
    &PIT_EVENT
}

/// Program PIT channel 0 in periodic mode at the device rate.
pub fn init_periodic() {
    super::legacy_init_pit();
}
