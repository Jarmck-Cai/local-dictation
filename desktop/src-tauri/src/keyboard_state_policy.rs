//! Narrow, ephemeral F13–F24 state. No text, timestamps of typed keys or I/O.
pub(crate) const FIRST_HIGH: u16 = 0x7c;
pub(crate) const LAST_HIGH: u16 = 0x87;
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

pub(crate) struct State {
    devices: Vec<(usize, u16)>,
    available: bool,
}

impl State {
    pub(crate) fn new() -> Self {
        Self {
            devices: Vec::new(),
            available: true,
        }
    }
    pub(crate) fn fail(&mut self) {
        self.available = false;
    }
    pub(crate) fn remove_device(&mut self, device: usize) {
        self.devices.retain(|(id, _)| *id != device);
    }
    /// Keep only observed extended-key holds, tracked per device.
    pub(crate) fn raw(&mut self, device: usize, high_key: Option<u8>, down: bool) {
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
        }
    }
    /// Called only after an explicit raw/device message drain barrier.
    pub(crate) fn query(&self, ordinary: bool, high: u16, bound: u16) -> Decision {
        if !self.available {
            Decision::Unknown
        } else if ordinary || !self.devices.is_empty() || high & bound != 0 {
            Decision::Held
        } else {
            // An unbound F13–F24 async bit alone does not modify the paste chord.
            Decision::Ready
        }
    }
}
