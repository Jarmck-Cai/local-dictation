//! Narrow, ephemeral F13–F24 state. No text, timestamps of typed keys or I/O.
pub(crate) const FIRST_HIGH: u16 = 0x7c;
pub(crate) const LAST_HIGH: u16 = 0x87;
const ALL: u16 = (1 << 12) - 1;
const STABLE_MS: u64 = 50;
const MAX_DEVICES: usize = 32;

pub(crate) fn high_index(vk: u16) -> Option<u8> {
    (FIRST_HIGH..=LAST_HIGH)
        .contains(&vk)
        .then(|| (vk - FIRST_HIGH) as u8)
}

pub(crate) fn configured_mask<'a>(bindings: impl IntoIterator<Item = &'a str>) -> u16 {
    let mut mask = 0;
    for binding in bindings {
        for token in binding.split('+').map(str::trim) {
            if let Some(number) = token.get(1..).filter(|_| token.starts_with(['f', 'F'])) {
                if let Ok(number @ 13..=24) = number.parse::<u16>() {
                    mask |= 1 << (number - 13);
                }
            }
        }
    }
    mask
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decision {
    Ready,
    Held,
    Unknown,
}

#[derive(Clone, Copy)]
struct Candidate {
    high: u16,
    generation: u64,
    epoch: u64,
    began_ms: u64,
}

pub(crate) struct State {
    devices: Vec<(usize, u16)>,
    known_up: u16,
    generation: u64,
    epoch: u64,
    available: bool,
    candidate: Option<Candidate>,
}

impl State {
    pub(crate) fn new() -> Self {
        Self {
            devices: Vec::new(),
            known_up: 0,
            generation: 0,
            epoch: 0,
            available: true,
            candidate: None,
        }
    }
    pub(crate) fn available(&self) -> bool {
        self.available
    }
    /// Revoke one synchronization attempt without losing observed physical
    /// holds or a prior up anchor. A scheduling delay is not lost raw data.
    pub(crate) fn revoke(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.candidate = None;
    }
    fn physical_down(&self) -> u16 {
        self.devices.iter().fold(0, |mask, (_, held)| mask | held)
    }
    pub(crate) fn baseline(&mut self, high: u16) {
        // A high state at startup/reset can never establish a released key.
        if self.available {
            self.known_up |= !high & ALL & !self.physical_down();
        }
    }
    pub(crate) fn reset(&mut self) {
        self.devices.clear();
        self.invalidate();
    }
    fn invalidate(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.known_up = 0;
        self.candidate = None;
    }
    pub(crate) fn fail(&mut self) {
        self.available = false;
        self.invalidate();
    }
    pub(crate) fn remove_device(&mut self, device: usize) {
        self.devices.retain(|(id, _)| *id != device);
        self.invalidate();
    }
    pub(crate) fn device_changed(&mut self) {
        self.invalidate();
    }
    /// Ordinary raw events invalidate a candidate without retaining their VK.
    pub(crate) fn raw(&mut self, device: usize, high_key: Option<u8>, down: bool) {
        self.generation = self.generation.wrapping_add(1);
        self.candidate = None;
        let Some(key @ 0..=11) = high_key else { return };
        if device == 0 {
            self.fail();
            return;
        }
        let bit = 1 << key;
        if down {
            if let Some((_, held)) = self.devices.iter_mut().find(|(id, _)| *id == device) {
                *held |= bit; // Repeated make events are idempotent.
            } else if self.devices.len() >= MAX_DEVICES || self.devices.try_reserve(1).is_err() {
                self.fail();
            } else {
                self.devices.push((device, bit));
            }
        } else {
            if let Some((_, held)) = self.devices.iter_mut().find(|(id, _)| *id == device) {
                *held &= !bit;
            }
            self.devices.retain(|(_, held)| *held != 0);
            // A break on one device says nothing about another device held
            // before startup/reset. Only an async-up baseline anchors trust.
        }
    }
    /// Called only after an explicit raw/device message drain barrier.
    pub(crate) fn query(&mut self, ordinary: bool, high: u16, bound: u16, now_ms: u64) -> Decision {
        if ordinary || self.physical_down() != 0 || high & bound != 0 {
            self.candidate = None;
            return Decision::Held;
        }
        if !self.available {
            self.candidate = None;
            return Decision::Unknown;
        }
        if high == 0 {
            self.candidate = None;
            self.baseline(0);
            return Decision::Ready;
        }
        if !self.available || high & !self.known_up != 0 {
            self.candidate = None;
            return Decision::Unknown;
        }
        if let Some(candidate) = self.candidate {
            if candidate.high == high
                && candidate.generation == self.generation
                && candidate.epoch == self.epoch
            {
                return if now_ms.saturating_sub(candidate.began_ms) >= STABLE_MS {
                    Decision::Ready
                } else {
                    Decision::Held
                };
            }
        }
        self.candidate = Some(Candidate {
            high,
            generation: self.generation,
            epoch: self.epoch,
            began_ms: now_ms,
        });
        Decision::Held
    }
}
