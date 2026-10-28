//! Shared capture contracts. Audio remains private and never enters IPC payloads.
use serde::{Deserialize, Serialize};

/// Shared capture queue budget. Every source must use try_send and stop on Full.
/// At two 10ms sources: about ten seconds, or roughly 6 MiB at 48k float mono+stereo.
pub const MEETING_CAPTURE_QUEUE_CAPACITY: usize = 2048;
pub const MEETING_CAPTURE_QUEUE_MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Microphone,
    Remote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AudioEncoding {
    Pcm,
    Float,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
    pub valid_bits: u16,
    pub channel_mask: u32,
    pub encoding: AudioEncoding,
}

#[derive(Clone, Debug)]
pub struct CapturePacket {
    pub source: Source,
    pub epoch: u32,
    pub sequence: u64,
    pub device_frame_position: u64,
    pub qpc_100ns: u64,
    pub frames: u32,
    pub flags: u32,
    pub format: AudioFormat,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug)]
pub enum CaptureEvent {
    Packet(CapturePacket),
    Heartbeat { source: Source, qpc_100ns: u64 },
    Error { source: Source, code: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub source: Source,
    pub is_default: bool,
}

#[derive(Clone, Debug)]
pub struct CaptureConfig {
    pub microphone_id: String,
    pub render_id: String,
    pub epoch: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum MeetingStatus {
    Recording,
    Paused,
    Recorded,
    Interrupted,
    Processing,
    Complete,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct ChunkMeta {
    pub file: String,
    pub source: Source,
    pub epoch: u32,
    pub format: AudioFormat,
    pub start_qpc_100ns: u64,
    pub end_qpc_100ns: u64,
    pub packets: u64,
    pub frames: u64,
    pub committed_bytes: u64,
    pub finalized: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct MeetingManifest {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    /// UTC seconds since Unix epoch. Retention uses creation time, never file mtime.
    pub created_at_unix_secs: u64,
    pub start_qpc_100ns: u64,
    pub end_qpc_100ns: Option<u64>,
    pub devices: Vec<DeviceInfo>,
    pub status: MeetingStatus,
    /// Static error code only; never audio, transcript, or device identifiers.
    pub error: Option<String>,
    /// Original capture failure survives processing status changes and retries.
    #[serde(default)]
    pub capture_error: Option<String>,
    /// None denotes legacy metadata; processing must preserve capture provenance.
    #[serde(default)]
    pub capture_complete: Option<bool>,
    pub chunks: Vec<ChunkMeta>,
    #[serde(default)]
    pub source_health: Vec<SourceHealth>,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct SourceHealth {
    pub source: Source,
    pub epoch: u32,
    pub first_heartbeat_qpc_100ns: u64,
    pub last_heartbeat_qpc_100ns: u64,
}
