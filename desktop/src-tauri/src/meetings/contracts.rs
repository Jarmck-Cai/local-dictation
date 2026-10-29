//! Small UI contracts; raw packets, paths and audio never enter status events.
use super::types::{MeetingStatus, Source};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct MeetingSummary {
    pub id: String,
    /// Increasing within one application run; clients ignore older snapshots.
    #[serde(default)]
    pub revision: u64,
    pub title: String,
    pub created_at_ms: u64,
    pub duration_ms: u64,
    pub status: MeetingStatus,
    pub error: Option<String>,
    pub progress_percent: Option<u8>,
    pub microphone_name: Option<String>,
    pub playback_name: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct MeetingSegment {
    pub id: u32,
    pub source: Source,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct MeetingTranscript {
    pub id: String,
    pub complete: bool,
    pub segments: Vec<MeetingSegment>,
    pub warnings: Vec<String>,
}
