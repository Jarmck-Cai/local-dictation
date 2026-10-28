//! Shared capture backpressure: both event count and payload bytes are bounded.
use super::types::{CaptureEvent, MEETING_CAPTURE_QUEUE_CAPACITY, MEETING_CAPTURE_QUEUE_MAX_BYTES};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, RecvError, RecvTimeoutError, TryRecvError, TrySendError},
        Arc,
    },
    time::Duration,
};

struct Budget {
    bytes: AtomicUsize,
    connected: AtomicBool,
}
#[derive(Clone)]
pub struct CaptureSender {
    sender: mpsc::SyncSender<CaptureEvent>,
    budget: Arc<Budget>,
}
pub struct CaptureReceiver {
    receiver: mpsc::Receiver<CaptureEvent>,
    budget: Arc<Budget>,
}
pub fn capture_event_queue() -> (CaptureSender, CaptureReceiver) {
    let (sender, receiver) = mpsc::sync_channel(MEETING_CAPTURE_QUEUE_CAPACITY);
    let budget = Arc::new(Budget {
        bytes: AtomicUsize::new(0),
        connected: AtomicBool::new(true),
    });
    (
        CaptureSender {
            sender,
            budget: budget.clone(),
        },
        CaptureReceiver { receiver, budget },
    )
}
impl CaptureSender {
    pub fn try_send(&self, event: CaptureEvent) -> Result<(), TrySendError<CaptureEvent>> {
        if !self.budget.connected.load(Ordering::Acquire) {
            return Err(TrySendError::Disconnected(event));
        }
        let bytes = payload_bytes(&event);
        if self
            .budget
            .bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes)
                    .filter(|sum| *sum <= MEETING_CAPTURE_QUEUE_MAX_BYTES)
            })
            .is_err()
        {
            return Err(TrySendError::Full(event));
        }
        match self.sender.try_send(event) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.budget.bytes.fetch_sub(bytes, Ordering::AcqRel);
                Err(error)
            }
        }
    }
}
fn payload_bytes(event: &CaptureEvent) -> usize {
    match event {
        CaptureEvent::Packet(packet) => packet.data.len(),
        _ => 0,
    }
}
impl CaptureReceiver {
    fn release(&self, event: CaptureEvent) -> CaptureEvent {
        self.budget
            .bytes
            .fetch_sub(payload_bytes(&event), Ordering::AcqRel);
        event
    }
    pub fn recv(&self) -> Result<CaptureEvent, RecvError> {
        self.receiver.recv().map(|event| self.release(event))
    }
    pub fn recv_timeout(&self, timeout: Duration) -> Result<CaptureEvent, RecvTimeoutError> {
        self.receiver
            .recv_timeout(timeout)
            .map(|event| self.release(event))
    }
    pub fn try_recv(&self) -> Result<CaptureEvent, TryRecvError> {
        self.receiver.try_recv().map(|event| self.release(event))
    }
}
impl Drop for CaptureReceiver {
    fn drop(&mut self) {
        self.budget.connected.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::*;
    use super::*;
    fn packet(bytes: usize) -> CaptureEvent {
        CaptureEvent::Packet(CapturePacket {
            source: Source::Microphone,
            epoch: 0,
            sequence: 0,
            device_frame_position: 0,
            qpc_100ns: 0,
            frames: 1,
            flags: 0,
            format: AudioFormat {
                sample_rate: 48000,
                channels: 1,
                bits_per_sample: 16,
                valid_bits: 16,
                channel_mask: 4,
                encoding: AudioEncoding::Pcm,
            },
            data: vec![0; bytes],
        })
    }
    fn heartbeat() -> CaptureEvent {
        CaptureEvent::Heartbeat {
            source: Source::Remote,
            qpc_100ns: 0,
        }
    }
    #[test]
    fn byte_budget_is_full_at_sixteen_mib_and_each_receive_releases_it() {
        let (sender, receiver) = capture_event_queue();
        for _ in 0..4 {
            sender.try_send(packet(4 * 1024 * 1024)).unwrap();
        }
        assert!(matches!(
            sender.try_send(packet(1)),
            Err(TrySendError::Full(_))
        ));
        drop(receiver.recv().unwrap());
        sender.try_send(packet(4 * 1024 * 1024)).unwrap();
        drop(receiver.recv_timeout(Duration::from_secs(1)).unwrap());
        sender.try_send(packet(4 * 1024 * 1024)).unwrap();
        drop(receiver.try_recv().unwrap());
        sender.try_send(packet(4 * 1024 * 1024)).unwrap();
        assert!(matches!(
            sender.try_send(packet(1)),
            Err(TrySendError::Full(_))
        ));
    }
    #[test]
    fn count_failure_refunds_reserved_payload_bytes() {
        let (sender, receiver) = capture_event_queue();
        for _ in 0..2048 {
            sender.try_send(heartbeat()).unwrap();
        }
        assert!(matches!(
            sender.try_send(packet(4 * 1024 * 1024)),
            Err(TrySendError::Full(_))
        ));
        for _ in 0..2048 {
            receiver.recv().unwrap();
        }
        for _ in 0..4 {
            sender.try_send(packet(4 * 1024 * 1024)).unwrap();
        }
        assert!(matches!(
            sender.try_send(packet(1)),
            Err(TrySendError::Full(_))
        ));
    }
    #[test]
    fn receiver_drop_is_disconnected_even_with_exhausted_byte_budget() {
        let (sender, receiver) = capture_event_queue();
        for _ in 0..4 {
            sender.try_send(packet(4 * 1024 * 1024)).unwrap();
        }
        drop(receiver);
        assert!(matches!(
            sender.try_send(packet(1)),
            Err(TrySendError::Disconnected(_))
        ));
    }
    #[test]
    fn cloned_senders_reserve_one_shared_budget_atomically() {
        let (sender, receiver) = capture_event_queue();
        let barrier = Arc::new(std::sync::Barrier::new(6));
        let mut workers = vec![];
        for _ in 0..5 {
            let sender = sender.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                sender.try_send(packet(4 * 1024 * 1024)).is_ok()
            }));
        }
        barrier.wait();
        let successes = workers
            .into_iter()
            .map(|w| usize::from(w.join().unwrap()))
            .sum::<usize>();
        assert_eq!(successes, 4);
        drop(receiver);
    }
}
