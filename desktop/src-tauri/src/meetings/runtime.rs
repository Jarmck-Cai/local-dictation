//! Meeting actors and exports, independent of Tauri and ordinary dictation output.
use super::{
    alignment::AlignedMeeting,
    capture,
    contracts::{MeetingSummary, MeetingTranscript},
    queue::{capture_event_queue, CaptureReceiver, CaptureSender},
    store::{MeetingStore, MeetingWriter},
    types::*,
};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, RecvTimeoutError, TryRecvError},
        Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

type RuntimeResult<T> = Result<T, String>;
pub type ProcessingJob = dyn Fn(&MeetingStore, &str, &AtomicBool, &dyn Fn(u8)) -> RuntimeResult<MeetingTranscript>
    + Send
    + Sync;
pub type RecordingLease = dyn Fn() -> RuntimeResult<Box<dyn Send>> + Send + Sync;
pub type DeviceProvider = dyn Fn() -> RuntimeResult<Vec<DeviceInfo>> + Send + Sync;

/// A committed two-source capture session. Stop must join both producers.
pub trait CaptureSession: Send {
    fn start_qpc_100ns(&self) -> u64;
    fn stop(self: Box<Self>) -> RuntimeResult<u64>;
}
/// Injection seam: synthetic sessions use the same queue, actor and disk writer.
pub trait CaptureFactory: Send + Sync {
    fn start(
        &self,
        config: CaptureConfig,
        sender: CaptureSender,
        stop: Arc<AtomicBool>,
    ) -> RuntimeResult<Box<dyn CaptureSession>>;
}
pub struct NativeCaptureFactory;
impl CaptureSession for capture::CaptureHandle {
    fn start_qpc_100ns(&self) -> u64 {
        self.start_qpc_100ns()
    }
    fn stop(self: Box<Self>) -> RuntimeResult<u64> {
        (*self).stop()
    }
}
impl CaptureFactory for NativeCaptureFactory {
    fn start(
        &self,
        config: CaptureConfig,
        sender: CaptureSender,
        stop: Arc<AtomicBool>,
    ) -> RuntimeResult<Box<dyn CaptureSession>> {
        capture::start_capture(config, sender, stop)
            .map(|session| Box::new(session) as Box<dyn CaptureSession>)
    }
}
/// Application glue supplies model serialization, record exclusion and events.
pub struct RuntimeServices {
    pub devices: Arc<DeviceProvider>,
    pub capture: Arc<dyn CaptureFactory>,
    pub acquire_recording: Arc<RecordingLease>,
    pub process: Arc<ProcessingJob>,
    pub emit: Arc<dyn Fn(MeetingSummary) + Send + Sync>,
}

#[derive(Default)]
struct Completion {
    result: Mutex<Option<RuntimeResult<MeetingSummary>>>,
    ready: Condvar,
}
impl Completion {
    fn finish(&self, result: RuntimeResult<MeetingSummary>) {
        if let Ok(mut slot) = self.result.lock() {
            *slot = Some(result);
            self.ready.notify_all();
        }
    }
    fn wait(&self) -> RuntimeResult<MeetingSummary> {
        let slot = self.result.lock().map_err(|_| "meeting_lock")?;
        let (slot, _) = self
            .ready
            .wait_timeout_while(slot, Duration::from_secs(35), |slot| slot.is_none())
            .map_err(|_| "meeting_lock")?;
        slot.clone()
            .unwrap_or_else(|| Err("meeting_stop_timeout".into()))
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Activity {
    Starting,
    Recording,
    Processing,
}
struct Active {
    id: String,
    generation: u64,
    kind: Activity,
    request: Arc<AtomicBool>,
    completion: Arc<Completion>,
}
#[derive(Default)]
struct State {
    active: Option<Active>,
    current: Option<MeetingSummary>,
    generation: u64,
    revision: u64,
    snapshots: HashMap<String, MeetingSummary>,
    deleted: HashSet<String>,
}
struct Inner {
    store: MeetingStore,
    root: PathBuf,
    services: RuntimeServices,
    state: Mutex<State>,
    closing: AtomicBool,
    #[cfg(test)]
    observer: Mutex<Option<CaptureObserver>>,
}
#[cfg(test)]
type CaptureObserver =
    Arc<dyn Fn(Source, u64, u32, usize, u64, super::store::WriterStats) + Send + Sync>;
pub struct MeetingRuntime {
    inner: Arc<Inner>,
}
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn unix_secs() -> RuntimeResult<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_secs())
        .map_err(|_| "meeting_clock".into())
}
fn summary(manifest: &MeetingManifest, progress: Option<u8>) -> MeetingSummary {
    let observed_end = manifest.end_qpc_100ns.unwrap_or_else(|| {
        manifest
            .chunks
            .iter()
            .map(|chunk| chunk.end_qpc_100ns)
            .chain(
                manifest
                    .source_health
                    .iter()
                    .map(|health| health.last_heartbeat_qpc_100ns),
            )
            .max()
            .unwrap_or(manifest.start_qpc_100ns)
    });
    MeetingSummary {
        id: manifest.id.clone(),
        revision: 0,
        title: manifest.title.clone(),
        created_at_ms: manifest.created_at_unix_secs.saturating_mul(1000),
        duration_ms: observed_end.saturating_sub(manifest.start_qpc_100ns) / 10_000,
        status: manifest.status,
        error: manifest.error.clone(),
        capture_error: manifest.capture_error.clone(),
        progress_percent: progress,
        microphone_name: manifest
            .devices
            .iter()
            .find(|device| device.source == Source::Microphone)
            .map(|device| device.name.clone()),
        playback_name: manifest
            .devices
            .iter()
            .find(|device| device.source == Source::Remote)
            .map(|device| device.name.clone()),
    }
}
fn disk_summary(manifest: &MeetingManifest, owned: bool) -> MeetingSummary {
    let mut value = summary(manifest, None);
    if !owned {
        // A checkpoint discovered from disk cannot revive a job that this
        // runtime does not own. Keep raw repair on explicit transcribe/export.
        match value.status {
            MeetingStatus::Recording | MeetingStatus::Paused => {
                value.status = MeetingStatus::Interrupted;
            }
            MeetingStatus::Processing => {
                value.status = MeetingStatus::Failed;
                value.error = Some("meeting_processing_interrupted".into());
            }
            _ => (),
        }
    }
    value
}
fn static_error(error: &str) -> String {
    if error.starts_with("meeting_")
        && error.len() <= 96
        && error
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        error.into()
    } else {
        "meeting_internal".into()
    }
}
fn emit(inner: &Inner, value: MeetingSummary) {
    // A disappearing window/event sink cannot kill the recording writer.
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        (inner.services.emit)(value)
    }));
}
fn remember(state: &mut State, mut value: MeetingSummary) -> MeetingSummary {
    state.revision = state.revision.saturating_add(1);
    value.revision = state.revision;
    state.snapshots.insert(value.id.clone(), value.clone());
    value
}
fn publish(
    inner: &Inner,
    generation: u64,
    value: MeetingSummary,
    done: bool,
) -> Option<MeetingSummary> {
    let accepted = if let Ok(mut state) = inner.state.lock() {
        if state
            .active
            .as_ref()
            .is_some_and(|active| active.generation == generation)
        {
            let value = remember(&mut state, value);
            state.current = Some(value.clone());
            if done {
                state.active = None;
            }
            Some(value)
        } else {
            None
        }
    } else {
        None
    };
    if let Some(value) = &accepted {
        emit(inner, value.clone());
    }
    accepted
}
fn publish_progress(inner: &Inner, generation: u64, percent: u8) {
    // Read, advance and commit in one critical section. Delivery is allowed to
    // block or arrive out of order; the assigned revision never changes.
    let value = inner.state.lock().ok().and_then(|mut state| {
        if state
            .active
            .as_ref()
            .is_some_and(|active| active.generation == generation)
        {
            state.current.clone().map(|mut value| {
                value.progress_percent =
                    Some(percent.min(100).max(value.progress_percent.unwrap_or(0)));
                let value = remember(&mut state, value);
                state.current = Some(value.clone());
                value
            })
        } else {
            None
        }
    });
    if let Some(value) = value {
        emit(inner, value);
    }
}
fn finish_runtime(
    inner: &Inner,
    generation: u64,
    result: RuntimeResult<MeetingSummary>,
    failed_status: MeetingStatus,
    completion: &Completion,
) {
    let value = match &result {
        Ok(value) => Some(value.clone()),
        Err(error) => inner
            .state
            .lock()
            .ok()
            .and_then(|state| state.current.clone())
            .map(|mut value| {
                value.status = failed_status;
                value.error = Some(static_error(error));
                if failed_status == MeetingStatus::Interrupted && value.capture_error.is_none() {
                    value.capture_error = Some(static_error(error));
                }
                value.progress_percent = None;
                value
            }),
    };
    let result = if let Some(value) = value {
        let committed = publish(inner, generation, value, true);
        match (result, committed) {
            (Ok(_), Some(value)) => Ok(value),
            (result, _) => result,
        }
    } else if let Ok(mut state) = inner.state.lock() {
        if state
            .active
            .as_ref()
            .is_some_and(|active| active.generation == generation)
        {
            state.active = None;
        }
        result
    } else {
        result
    };
    completion.finish(result);
}

fn needs_raw_recovery(manifest: &MeetingManifest) -> bool {
    matches!(
        manifest.status,
        MeetingStatus::Recording | MeetingStatus::Paused
    ) || manifest.chunks.iter().any(|chunk| !chunk.finalized)
}

/// Only an unfinished raw writer warrants a scan/truncation. A stale offline
/// job has healthy raw and must retain its original capture provenance.
fn prepare_stored(
    store: &MeetingStore,
    manifest: MeetingManifest,
) -> RuntimeResult<MeetingManifest> {
    if needs_raw_recovery(&manifest) {
        let recovered = store.recover(&manifest.id).map_err(str::to_owned)?;
        if matches!(
            manifest.status,
            MeetingStatus::Interrupted | MeetingStatus::Failed
        ) && manifest.error.is_some()
        {
            let writer = store.edit(&manifest.id).map_err(str::to_owned)?;
            return writer
                .finish(
                    MeetingStatus::Interrupted,
                    recovered.end_qpc_100ns,
                    manifest.error,
                )
                .map_err(str::to_owned);
        }
        return Ok(recovered);
    }
    if manifest.status == MeetingStatus::Processing {
        return store
            .edit(&manifest.id)
            .map_err(str::to_owned)?
            .finish(
                MeetingStatus::Failed,
                manifest.end_qpc_100ns,
                Some("meeting_processing_interrupted".into()),
            )
            .map_err(str::to_owned);
    }
    Ok(manifest)
}

fn prepare_with_cause(
    store: &MeetingStore,
    manifest: MeetingManifest,
    cause: Option<String>,
) -> RuntimeResult<MeetingManifest> {
    let manifest = if let Some(cause) = cause.filter(|_| {
        manifest.capture_error.as_deref().is_none_or(|old| {
            matches!(
                old,
                "meeting_capture_recovered" | "meeting_recovered" | "meeting_recording_interrupted"
            )
        })
    }) {
        store
            .persist_capture_error(&manifest.id, cause)
            .map_err(str::to_owned)?
    } else {
        manifest
    };
    prepare_stored(store, manifest)
}

impl MeetingRuntime {
    /// Recover only unfinished sessions, then enforce creation-time retention.
    pub fn new(
        store: MeetingStore,
        meeting_root: PathBuf,
        services: RuntimeServices,
    ) -> RuntimeResult<Self> {
        let root = fs::canonicalize(meeting_root).map_err(|_| "meeting_path")?;
        validate_existing(&root)?;
        for manifest in store.list().map_err(str::to_owned)? {
            match prepare_stored(&store, manifest) {
                Ok(_) => (),
                Err(error) if error == "meeting_active" => (),
                Err(error) => return Err(error),
            }
        }
        store.prune(unix_secs()?).map_err(str::to_owned)?;
        Ok(Self {
            inner: Arc::new(Inner {
                store,
                root,
                services,
                state: Mutex::new(State::default()),
                closing: AtomicBool::new(false),
                #[cfg(test)]
                observer: Mutex::new(None),
            }),
        })
    }
    /// Narrow test adapter: no observer or per-packet timer in application builds.
    #[cfg(test)]
    pub fn observe_capture(&self, observer: CaptureObserver) {
        *self.inner.observer.lock().unwrap() = Some(observer);
    }
    pub fn list(&self) -> RuntimeResult<Vec<MeetingSummary>> {
        self.list_with(|| self.inner.store.list().map_err(str::to_owned))
    }
    fn list_with(
        &self,
        read: impl FnOnce() -> RuntimeResult<Vec<MeetingManifest>>,
    ) -> RuntimeResult<Vec<MeetingSummary>> {
        let observed_revision = self
            .inner
            .state
            .lock()
            .map_err(|_| "meeting_lock")?
            .revision;
        let manifests = read()?;
        let mut state = self.inner.state.lock().map_err(|_| "meeting_lock")?;
        let disk_ids: HashSet<_> = manifests
            .iter()
            .map(|manifest| manifest.id.as_str())
            .collect();
        let active_id = state.active.as_ref().map(|active| active.id.clone());
        // Daily maintenance can prune from another store instance. Keep only
        // disk rows and snapshots published after this scan began (or active).
        state.snapshots.retain(|id, value| {
            disk_ids.contains(id.as_str())
                || value.revision > observed_revision
                || active_id.as_deref() == Some(id)
        });
        if state
            .current
            .as_ref()
            .is_some_and(|value| !state.snapshots.contains_key(&value.id))
        {
            state.current = None;
        }
        for manifest in &manifests {
            if let Some(previous) = state.snapshots.get(&manifest.id).cloned() {
                let unavailable = manifest.schema_version == 0;
                let was_unavailable = previous.created_at_ms == 0
                    && previous
                        .error
                        .as_deref()
                        .is_some_and(super::store::is_manifest_restore_error);
                if (unavailable || was_unavailable)
                    && previous.revision <= observed_revision
                    && active_id.as_deref() != Some(manifest.id.as_str())
                    && (unavailable != was_unavailable
                        || previous.error != manifest.error
                        || previous.status != manifest.status)
                {
                    let mut value = disk_summary(manifest, false);
                    if unavailable || (was_unavailable && value.capture_error.is_none()) {
                        value.capture_error = previous.capture_error;
                    }
                    let value = remember(&mut state, value);
                    if state
                        .current
                        .as_ref()
                        .is_some_and(|current| current.id == manifest.id)
                    {
                        state.current = Some(value);
                    }
                }
            }
            if !state.deleted.contains(&manifest.id) && !state.snapshots.contains_key(&manifest.id)
            {
                remember(
                    &mut state,
                    disk_summary(manifest, active_id.as_deref() == Some(manifest.id.as_str())),
                );
            }
        }
        if let Some(active) = state.current.clone().filter(|value| {
            active_id.as_deref() == Some(value.id.as_str()) && value.revision < state.revision
        }) {
            // History repair advances the global revision. Publish the still
            // current activity last so list consumers cannot interpret that
            // newer historical terminal row as an authoritative idle state.
            state.current = Some(remember(&mut state, active));
        }
        // Retained snapshots win over disk: a concurrent stop/retry/create must
        // retain its revision. Tombstones exclude a late read after deletion.
        let mut rows: Vec<_> = state.snapshots.values().cloned().collect();
        rows.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms).then(a.id.cmp(&b.id)));
        Ok(rows)
    }
    pub fn current(&self) -> RuntimeResult<Option<MeetingSummary>> {
        Ok(self
            .inner
            .state
            .lock()
            .map_err(|_| "meeting_lock")?
            .current
            .clone())
    }
    fn stored_summary(&self, id: &str) -> RuntimeResult<MeetingSummary> {
        let manifest = self.inner.store.load(id).map_err(str::to_owned)?;
        let mut state = self.inner.state.lock().map_err(|_| "meeting_lock")?;
        if state.deleted.contains(id) {
            return Err("meeting_not_found".into());
        }
        Ok(state
            .snapshots
            .get(id)
            .cloned()
            .unwrap_or_else(|| remember(&mut state, summary(&manifest, None))))
    }
    fn reserve(
        &self,
        id: &str,
        kind: Activity,
    ) -> RuntimeResult<(u64, Arc<AtomicBool>, Arc<Completion>)> {
        let mut state = self.inner.state.lock().map_err(|_| "meeting_lock")?;
        if self.inner.closing.load(Ordering::Acquire) {
            return Err("meeting_shutdown".into());
        }
        if state.active.is_some() {
            return Err("meeting_busy".into());
        }
        state.generation = state.generation.wrapping_add(1);
        let generation = state.generation;
        let request = Arc::new(AtomicBool::new(false));
        let completion = Arc::new(Completion::default());
        state.active = Some(Active {
            id: id.into(),
            generation,
            kind,
            request: request.clone(),
            completion: completion.clone(),
        });
        Ok((generation, request, completion))
    }
    fn abandon(&self, generation: u64) {
        if let Ok(mut state) = self.inner.state.lock() {
            if state
                .active
                .as_ref()
                .is_some_and(|active| active.generation == generation)
            {
                state.active = None;
            }
        }
    }
    /// Start the selected endpoints. No implicit default fallback is permitted.
    pub fn start(
        &self,
        title: &str,
        microphone_id: &str,
        render_id: &str,
    ) -> RuntimeResult<MeetingSummary> {
        if title.len() > 4096 {
            return Err("meeting_id".into());
        }
        let id = format!(
            "meeting-{}-{}-{}",
            unix_secs()?,
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let (generation, request, completion) = self.reserve(&id, Activity::Starting)?;
        let result: RuntimeResult<MeetingSummary> = (|| {
            let available = (self.inner.services.devices)()?;
            let mut devices = vec![];
            for (selected, source) in [
                (microphone_id, Source::Microphone),
                (render_id, Source::Remote),
            ] {
                devices.push(
                    available
                        .iter()
                        .find(|device| device.id == selected && device.source == source)
                        .cloned()
                        .ok_or("meeting_device_unavailable")?,
                );
            }
            let lease = (self.inner.services.acquire_recording)()?;
            let (sender, receiver) = capture_event_queue();
            let native_stop = Arc::new(AtomicBool::new(false));
            let session = self.inner.services.capture.start(
                CaptureConfig {
                    microphone_id: microphone_id.into(),
                    render_id: render_id.into(),
                    epoch: 0,
                },
                sender,
                native_stop.clone(),
            )?;
            let start = session.start_qpc_100ns();
            if start == 0 {
                drop(receiver);
                let _ = session.stop();
                return Err("meeting_capture_interval_invalid".into());
            }
            let writer = match self.inner.store.create(&id, title, start, devices) {
                Ok(writer) => writer,
                Err(error) => {
                    drop(receiver);
                    let _ = session.stop();
                    return Err(error.into());
                }
            };
            let value = summary(writer.manifest(), None);
            {
                let mut state = self.inner.state.lock().map_err(|_| "meeting_lock")?;
                let active = state.active.as_mut().ok_or("meeting_busy")?;
                active.kind = Activity::Recording;
            }
            if self.inner.closing.load(Ordering::Acquire) {
                request.store(true, Ordering::Release);
            }
            // Publish the initial state before an immediately-failing source
            // can finish on the actor; event order must never regress to Recording.
            let value = publish(&self.inner, generation, value, false).ok_or("meeting_busy")?;
            let inner = self.inner.clone();
            let done = completion.clone();
            let spawn = thread::Builder::new()
                .name("meeting-writer".into())
                .spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        record_actor(
                            &inner,
                            generation,
                            writer,
                            session,
                            receiver,
                            native_stop,
                            request,
                            lease,
                        )
                    }))
                    .unwrap_or_else(|_| Err("meeting_recording_panicked".into()));
                    let result = match result {
                        Ok(value) => Ok(value),
                        Err(error) => inner
                            .store
                            .recover_capture_error(&id, Some(static_error(&error)))
                            .map(|manifest| summary(&manifest, None))
                            .map_err(|_| error),
                    };
                    finish_runtime(
                        &inner,
                        generation,
                        result,
                        MeetingStatus::Interrupted,
                        &done,
                    );
                });
            if spawn.is_err() {
                return Err("meeting_thread_spawn".into());
            }
            Ok(value)
        })();
        if let Err(error) = &result {
            completion.finish(Err(error.clone()));
            self.abandon(generation);
        }
        result
    }
    /// Wait for capture stop and drain while the writer keeps receiving packets.
    pub fn stop(&self, id: &str) -> RuntimeResult<MeetingSummary> {
        let completion = {
            let state = self.inner.state.lock().map_err(|_| "meeting_lock")?;
            match &state.active {
                Some(active) if active.id == id && active.kind == Activity::Recording => {
                    active.request.store(true, Ordering::Release);
                    Some(active.completion.clone())
                }
                Some(_) => return Err("meeting_busy".into()),
                None => None,
            }
        };
        match completion {
            Some(completion) => completion.wait(),
            None => self.stored_summary(id),
        }
    }
    /// Launch a bounded offline processing job and return without running ASR here.
    pub fn transcribe(&self, id: &str) -> RuntimeResult<MeetingSummary> {
        {
            let state = self.inner.state.lock().map_err(|_| "meeting_lock")?;
            if let Some(active) = &state.active {
                if active.id == id && active.kind == Activity::Processing {
                    return state.current.clone().ok_or_else(|| "meeting_busy".into());
                }
                return Err("meeting_busy".into());
            }
        }
        let known_cause = self
            .inner
            .state
            .lock()
            .map_err(|_| "meeting_lock")?
            .snapshots
            .get(id)
            .and_then(|value| value.capture_error.clone());
        let (generation, cancel, completion) = self.reserve(id, Activity::Processing)?;
        let manifest = match self
            .inner
            .store
            .load(id)
            .map_err(str::to_owned)
            .and_then(|manifest| prepare_with_cause(&self.inner.store, manifest, known_cause))
        {
            Ok(manifest) => manifest,
            Err(error) => {
                completion.finish(Err(error.clone()));
                self.abandon(generation);
                return Err(error);
            }
        };
        let mut value = summary(&manifest, Some(0));
        value.status = MeetingStatus::Processing;
        value.error = None;
        let value = publish(&self.inner, generation, value, false).ok_or("meeting_busy")?;
        let inner = self.inner.clone();
        let id = id.to_owned();
        let spawn = thread::Builder::new()
            .name("meeting-processing".into())
            .spawn(move || {
                let progress = |percent: u8| publish_progress(&inner, generation, percent);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    (inner.services.process)(&inner.store, &id, &cancel, &progress)
                }))
                .unwrap_or_else(|_| Err("meeting_processing_panicked".into()));
                let final_result = finish_processing(&inner.store, &id, result, &cancel);
                finish_runtime(
                    &inner,
                    generation,
                    final_result,
                    MeetingStatus::Failed,
                    &completion,
                );
            });
        if spawn.is_err() {
            self.abandon(generation);
            return Err("meeting_thread_spawn".into());
        }
        Ok(value)
    }
    pub fn cancel_transcription(&self, id: &str) -> RuntimeResult<MeetingSummary> {
        {
            let state = self.inner.state.lock().map_err(|_| "meeting_lock")?;
            if let Some(active) = &state.active {
                if active.id != id || active.kind != Activity::Processing {
                    return Err("meeting_busy".into());
                }
                active.request.store(true, Ordering::Release);
                return state.current.clone().ok_or_else(|| "meeting_busy".into());
            }
        }
        self.stored_summary(id)
    }
    pub fn transcript(&self, id: &str) -> RuntimeResult<Option<MeetingTranscript>> {
        self.inner
            .store
            .read_transcript(id)
            .map_err(str::to_owned)?
            .map(|bytes| {
                let transcript: MeetingTranscript =
                    serde_json::from_slice(&bytes).map_err(|_| "meeting_transcript")?;
                if transcript.id != id {
                    return Err("meeting_transcript".into());
                }
                Ok(transcript)
            })
            .transpose()
    }
    pub fn delete(&self, id: &str) -> RuntimeResult<()> {
        let mut state = self.inner.state.lock().map_err(|_| "meeting_lock")?;
        if state.active.as_ref().is_some_and(|active| active.id == id) {
            return Err("meeting_active".into());
        }
        self.inner.store.delete(id).map_err(str::to_owned)?;
        if state.current.as_ref().is_some_and(|value| value.id == id) {
            state.current = None;
        }
        state.snapshots.remove(id);
        state.deleted.insert(id.into());
        Ok(())
    }
    /// Export original aligned audio or the saved transcript using a safe replacement.
    pub fn export(
        &self,
        id: &str,
        destination: &Path,
        format: &str,
        language: &str,
    ) -> RuntimeResult<()> {
        if !matches!(format, "audio" | "text") || !matches!(language, "zh" | "en") {
            return Err("meeting_export_format".into());
        }
        let lease = {
            let state = self.inner.state.lock().map_err(|_| "meeting_lock")?;
            if state.active.as_ref().is_some_and(|active| active.id == id) {
                return Err("meeting_active".into());
            }
            prepare_with_cause(
                &self.inner.store,
                self.inner.store.load(id).map_err(str::to_owned)?,
                state
                    .snapshots
                    .get(id)
                    .and_then(|value| value.capture_error.clone()),
            )?;
            self.inner.store.edit(id).map_err(str::to_owned)?
        };
        let (target, temporary, file) = export_destination(&self.inner.root, destination)?;
        let result = (|| {
            if format == "text" {
                let transcript = self.transcript(id)?.ok_or("meeting_transcript_missing")?;
                let mut output = BufWriter::new(file);
                write_text(&mut output, &transcript, language)?;
                output.flush().map_err(|_| "meeting_export_io")?;
                output
                    .get_ref()
                    .sync_all()
                    .map_err(|_| "meeting_export_io")?;
            } else {
                let mut aligned = AlignedMeeting::open(self.inner.store.clone(), id)?;
                let spec = hound::WavSpec {
                    channels: 2,
                    sample_rate: 16000,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                };
                let mut output = hound::WavWriter::new(BufWriter::new(file), spec)
                    .map_err(|_| "meeting_export_io")?;
                while let Some(frame) = aligned.next_frame()? {
                    for index in 0..frame.valid_samples as usize {
                        for sample in [frame.microphone[index], frame.remote[index]] {
                            let pcm = (sample.clamp(-1.0, 1.0) * 32768.0)
                                .round()
                                .clamp(-32768.0, 32767.0)
                                as i16;
                            output.write_sample(pcm).map_err(|_| "meeting_export_io")?;
                        }
                    }
                }
                output.finalize().map_err(|_| "meeting_export_io")?;
                OpenOptions::new()
                    .write(true)
                    .open(&temporary)
                    .and_then(|file| file.sync_all())
                    .map_err(|_| "meeting_export_io")?;
            }
            validate_export_target(&self.inner.root, &target)?;
            atomic_replace(&temporary, &target)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        drop(lease);
        result
    }
    /// Stop recording or cooperatively cancel processing before application exit.
    pub fn shutdown(&self) -> RuntimeResult<()> {
        self.inner.closing.store(true, Ordering::Release);
        let completion = {
            let state = self.inner.state.lock().map_err(|_| "meeting_lock")?;
            state.active.as_ref().map(|active| {
                active.request.store(true, Ordering::Release);
                active.completion.clone()
            })
        };
        if let Some(completion) = completion {
            completion.wait()?;
        }
        Ok(())
    }
}
impl Drop for MeetingRuntime {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[allow(clippy::too_many_arguments)]
fn record_actor(
    inner: &Inner,
    generation: u64,
    mut writer: MeetingWriter,
    session: Box<dyn CaptureSession>,
    receiver: CaptureReceiver,
    native_stop: Arc<AtomicBool>,
    request: Arc<AtomicBool>,
    lease: Box<dyn Send>,
) -> RuntimeResult<MeetingSummary> {
    let mut session = Some(session);
    let mut lease = Some(lease);
    let mut stopping = None;
    let mut terminal = None;
    let mut error: Option<String> = None;
    let mut last_event = Instant::now();
    loop {
        if stopping.is_none()
            && (request.load(Ordering::Acquire)
                || native_stop.load(Ordering::Acquire)
                || error.is_some())
        {
            let capture = session.take().ok_or("meeting_internal")?;
            let lease = lease.take();
            let (sender, stopped) = mpsc::channel();
            // Native stop joins producers. Receiving continues here throughout
            // their bounded drain, so even a nearly-full queue can make progress.
            thread::Builder::new()
                .name("meeting-capture-stop".into())
                .spawn(move || {
                    let result =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| capture.stop()))
                            .unwrap_or_else(|_| Err("meeting_capture_thread_panicked".into()));
                    drop(lease);
                    let _ = sender.send(result);
                })
                .map_err(|_| "meeting_thread_spawn")?;
            stopping = Some(stopped);
        }
        if terminal.is_none() {
            if let Some(stopped) = &stopping {
                match stopped.try_recv() {
                    Ok(result) => terminal = Some(result),
                    Err(TryRecvError::Disconnected) => {
                        terminal = Some(Err("meeting_capture_thread_panicked".into()))
                    }
                    Err(TryRecvError::Empty) => (),
                }
            }
        }
        let event = if terminal.is_some() {
            match receiver.try_recv() {
                Ok(event) => Some(event),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        } else {
            match receiver.recv_timeout(Duration::from_millis(25)) {
                Ok(event) => Some(event),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => {
                    if let Some(stopped) = &stopping {
                        // Joined producers can close the event channel before
                        // their stop thread delivers the result. Drain is done;
                        // wait for that result instead of inventing a failure.
                        match stopped.recv_timeout(Duration::from_millis(25)) {
                            Ok(result) => terminal = Some(result),
                            Err(RecvTimeoutError::Disconnected) => {
                                terminal = Some(Err("meeting_capture_thread_panicked".into()))
                            }
                            Err(RecvTimeoutError::Timeout) => (),
                        }
                    } else {
                        error.get_or_insert_with(|| "meeting_capture_consumer_gone".into());
                    }
                    None
                }
            }
        };
        if let Some(event) = event {
            let result = match event {
                CaptureEvent::Packet(packet) => {
                    #[cfg(test)]
                    let observation = (
                        packet.source,
                        packet.qpc_100ns,
                        packet.frames,
                        packet.data.len(),
                        Instant::now(),
                    );
                    let result = writer.append(packet);
                    #[cfg(test)]
                    if result.is_ok() {
                        if let Some(observer) = inner.observer.lock().ok().and_then(|o| o.clone()) {
                            observer(
                                observation.0,
                                observation.1,
                                observation.2,
                                observation.3,
                                observation.4.elapsed().as_micros() as u64,
                                writer.stats(),
                            );
                        }
                    }
                    result
                }
                CaptureEvent::Heartbeat { source, qpc_100ns } => {
                    writer.heartbeat(source, 0, qpc_100ns)
                }
                CaptureEvent::Error { code, .. } => {
                    error.get_or_insert_with(|| static_error(&code));
                    Ok(())
                }
            };
            if let Err(code) = result {
                error.get_or_insert_with(|| code.into());
            }
        }
        if let Err(code) = writer.checkpoint_if_due() {
            error.get_or_insert_with(|| code.into());
        }
        if last_event.elapsed() >= Duration::from_secs(1) {
            publish(inner, generation, summary(writer.manifest(), None), false);
            last_event = Instant::now();
        }
    }
    let end = match terminal.ok_or("meeting_internal")? {
        Ok(end) => Some(end),
        Err(code) => {
            error.get_or_insert_with(|| static_error(&code));
            None
        }
    };
    let end = end.or_else(|| {
        writer
            .manifest()
            .chunks
            .iter()
            .map(|chunk| chunk.end_qpc_100ns)
            .max()
    });
    let status = if error.is_some() {
        MeetingStatus::Interrupted
    } else {
        MeetingStatus::Recorded
    };
    // Publish known provenance before finish: even permanent disk failure must
    // leave the first fault visible for this application run.
    if let Some(cause) = &error {
        let mut value = summary(writer.manifest(), None);
        value.capture_error.get_or_insert_with(|| cause.clone());
        publish(inner, generation, value, false);
    }
    let manifest = writer
        .finish(status, end, error.clone())
        .map_err(|finish_error| error.unwrap_or_else(|| finish_error.into()))?;
    // finish releases the writer lease before recovery. Failed writes leave
    // their chunks unfinished, allowing only valid packet prefixes to survive.
    let manifest = prepare_stored(&inner.store, manifest)?;
    Ok(summary(&manifest, None))
}

fn finish_processing(
    store: &MeetingStore,
    id: &str,
    result: RuntimeResult<MeetingTranscript>,
    cancel: &AtomicBool,
) -> RuntimeResult<MeetingSummary> {
    let mut manifest = store.load(id).map_err(str::to_owned)?;
    let expected = match &result {
        Ok(transcript) if transcript.id == id => {
            if transcript.complete && !cancel.load(Ordering::Acquire) {
                MeetingStatus::Complete
            } else {
                MeetingStatus::Cancelled
            }
        }
        _ => MeetingStatus::Failed,
    };
    // The injected processor owns its lease and persists partial results. This
    // fallback also finalizes callbacks that panic before publishing a terminal state.
    if manifest.status != expected {
        let mut writer = store.edit(id).map_err(str::to_owned)?;
        let (status, error) = match result {
            Ok(transcript) if transcript.id == id => {
                writer
                    .write_transcript(
                        &serde_json::to_vec(&transcript).map_err(|_| "meeting_transcript")?,
                    )
                    .map_err(str::to_owned)?;
                (
                    if transcript.complete && !cancel.load(Ordering::Acquire) {
                        MeetingStatus::Complete
                    } else {
                        MeetingStatus::Cancelled
                    },
                    None,
                )
            }
            Ok(_) => (MeetingStatus::Failed, Some("meeting_transcript".into())),
            Err(code) => (MeetingStatus::Failed, Some(static_error(&code))),
        };
        let end = writer.manifest().end_qpc_100ns;
        manifest = writer.finish(status, end, error).map_err(str::to_owned)?;
    }
    Ok(summary(
        &manifest,
        if manifest.status == MeetingStatus::Complete {
            Some(100)
        } else {
            None
        },
    ))
}

fn time_label(ms: u64) -> String {
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}
fn write_text(
    output: &mut impl Write,
    transcript: &MeetingTranscript,
    language: &str,
) -> RuntimeResult<()> {
    if transcript
        .warnings
        .iter()
        .any(|code| code == "meeting_attribution_experimental")
    {
        writeln!(
            output,
            "{}",
            if language == "zh" {
                "Beta 预览：以下标签表示录音来源，不是说话人身份；外放可能产生重复文字或来源误判。"
            } else {
                "Beta preview: labels identify audio sources, not speaker identities; speaker playback may cause duplicate text or incorrect attribution."
            }
        )
        .map_err(|_| "meeting_export_io")?;
    }
    if transcript
        .warnings
        .iter()
        .any(|code| code == "meeting_alignment_incomplete_recording")
    {
        writeln!(
            output,
            "{}",
            if language == "zh" {
                "录音有中断或缺失，以下仅包含已保存的录音内容。"
            } else {
                "Recording was interrupted or incomplete; only saved audio is represented."
            }
        )
        .map_err(|_| "meeting_export_io")?;
    }
    if !transcript.complete {
        writeln!(
            output,
            "{}",
            if language == "zh" {
                "转录未完成，以下仅为已有结果。"
            } else {
                "Incomplete transcript; only available results follow."
            }
        )
        .map_err(|_| "meeting_export_io")?;
    }
    for segment in &transcript.segments {
        if segment.end_ms < segment.start_ms {
            return Err("meeting_transcript".into());
        }
        let label = match (segment.source, language) {
            (Source::Microphone, "zh") => "麦克风",
            (Source::Remote, "zh") => "电脑声音",
            (Source::Microphone, _) => "Microphone",
            (Source::Remote, _) => "Computer audio",
        };
        writeln!(
            output,
            "[{} --> {}] {}{}{}",
            time_label(segment.start_ms),
            time_label(segment.end_ms),
            label,
            if language == "zh" { "：" } else { ": " },
            segment.text
        )
        .map_err(|_| "meeting_export_io")?;
    }
    Ok(())
}
fn is_link(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}
fn validate_existing(path: &Path) -> RuntimeResult<()> {
    let mut part = PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return Err("meeting_path".into());
        }
        part.push(component);
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        match fs::symlink_metadata(&part) {
            Ok(metadata) if is_link(&metadata) => return Err("meeting_link".into()),
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err("meeting_export_io".into()),
        }
    }
    Ok(())
}
fn reject_hardlink(file: &File) -> RuntimeResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if file.metadata().map_err(|_| "meeting_export_io")?.nlink() != 1 {
            return Err("meeting_link".into());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        #[repr(C)]
        struct FileInfo {
            attributes: u32,
            times: [u32; 6],
            volume: u32,
            size_high: u32,
            size_low: u32,
            links: u32,
            index_high: u32,
            index_low: u32,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetFileInformationByHandle(
                handle: *mut std::ffi::c_void,
                info: *mut FileInfo,
            ) -> i32;
        }
        let mut info: FileInfo = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err("meeting_export_io".into());
        }
        if info.links != 1 {
            return Err("meeting_link".into());
        }
    }
    Ok(())
}
fn validate_export_target(root: &Path, target: &Path) -> RuntimeResult<()> {
    if !target.is_absolute() || target.file_name().is_none() {
        return Err("meeting_path".into());
    }
    // An ordinary save-dialog filename must never name an NTFS alternate data
    // stream, device alias, or a Windows path normalized to another filename.
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("meeting_path")?;
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if name.contains([':', '\0'])
        || name.ends_with(['.', ' '])
        || [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ]
        .contains(&stem.as_str())
    {
        return Err("meeting_path".into());
    }
    validate_existing(target)?;
    let parent = target.parent().ok_or("meeting_path")?;
    let parent = fs::canonicalize(parent).map_err(|_| "meeting_path")?;
    if parent.starts_with(root) {
        return Err("meeting_path".into());
    }
    if let Ok(metadata) = fs::symlink_metadata(target) {
        if !metadata.is_file() {
            return Err("meeting_path".into());
        }
        reject_hardlink(&File::open(target).map_err(|_| "meeting_export_io")?)?;
        let resolved = fs::canonicalize(target).map_err(|_| "meeting_path")?;
        if resolved.starts_with(root) {
            return Err("meeting_path".into());
        }
    }
    Ok(())
}
fn export_destination(root: &Path, destination: &Path) -> RuntimeResult<(PathBuf, PathBuf, File)> {
    validate_export_target(root, destination)?;
    let parent = fs::canonicalize(destination.parent().ok_or("meeting_path")?)
        .map_err(|_| "meeting_path")?;
    let target = parent.join(destination.file_name().ok_or("meeting_path")?);
    let temporary = parent.join(format!(
        ".qingshu-export-{}-{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| "meeting_export_io")?;
    Ok((target, temporary, file))
}
#[cfg(windows)]
fn atomic_replace(from: &Path, to: &Path) -> RuntimeResult<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0x1 | 0x8) } == 0 {
        Err("meeting_atomic_replace".into())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
fn atomic_replace(from: &Path, to: &Path) -> RuntimeResult<()> {
    fs::rename(from, to).map_err(|_| "meeting_atomic_replace".into())
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
