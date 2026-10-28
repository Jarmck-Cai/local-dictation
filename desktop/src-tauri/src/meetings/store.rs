//! Recoverable raw packet storage. Audio is streamed, never accumulated per meeting.
use super::types::*;
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{BufWriter, Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub type StoreResult<T> = Result<T, &'static str>;
pub const MAX_PACKET_BYTES: usize = 4 * 1024 * 1024;
const MAX_CHUNK_BYTES: u64 = 4 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 32 * 1024 * 1024;
const MAGIC: &[u8; 8] = b"QSMEET01";
const RETENTION_SECS: u64 = 7 * 24 * 60 * 60;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static ACTIVE: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

#[derive(Clone)]
pub struct MeetingStore {
    root: PathBuf,
}
pub struct MeetingWriter {
    store: MeetingStore,
    directory: PathBuf,
    manifest: MeetingManifest,
    open: [Option<(usize, File)>; 2],
    _lease: SessionLease,
    last_checkpoint: Instant,
    failed: bool,
    stats: WriterStats,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct WriterStats {
    pub checkpoints: u64,
    pub max_checkpoint_us: u64,
}
pub struct ChunkReader<R: Read = File> {
    file: R,
    meta: ChunkMeta,
    remaining: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChunkReadError {
    InvalidTail,
    Io,
}
impl ChunkReadError {
    fn code(self) -> &'static str {
        match self {
            Self::InvalidTail => "meeting_chunk",
            Self::Io => "meeting_io",
        }
    }
}
fn read_chunk_exact(reader: &mut impl Read, bytes: &mut [u8]) -> Result<(), ChunkReadError> {
    reader.read_exact(bytes).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            ChunkReadError::InvalidTail
        } else {
            ChunkReadError::Io
        }
    })
}
struct SessionLease {
    directory: PathBuf,
    file: Option<File>,
}

fn active() -> &'static Mutex<HashSet<PathBuf>> {
    ACTIVE.get_or_init(|| Mutex::new(HashSet::new()))
}
impl SessionLease {
    fn acquire(directory: &Path) -> StoreResult<Self> {
        let mut held = active().lock().map_err(|_| "meeting_lock")?;
        if held.contains(directory) {
            return Err("meeting_active");
        }
        let lock = directory.join(".recording.lock");
        safe_existing(&lock)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        let file = options.open(lock).map_err(|_| "meeting_active")?;
        reject_hardlink(&file)?;
        held.insert(directory.to_owned());
        Ok(Self {
            directory: directory.to_owned(),
            file: Some(file),
        })
    }
}
impl Drop for SessionLease {
    fn drop(&mut self) {
        self.file.take();
        // Keep the marker: stale after a crash is safe, with no unlink/open race.
        if let Ok(mut held) = active().lock() {
            held.remove(&self.directory);
        }
    }
}
fn valid_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 96 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return false;
    }
    let upper = id.to_ascii_uppercase();
    ![
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ]
    .contains(&upper.as_str())
}
fn chunk_name(name: &str) -> bool {
    let Some(number) = name
        .strip_prefix("chunk-")
        .and_then(|s| s.strip_suffix(".bin"))
    else {
        return false;
    };
    number.len() == 8 && number.bytes().all(|b| b.is_ascii_digit())
}
fn is_link(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}
/// Check every existing ancestor, including Windows junctions/reparse points.
fn safe_existing(path: &Path) -> StoreResult<()> {
    let mut part = PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::ParentDir) {
            return Err("meeting_path");
        }
        part.push(component);
        // A Windows verbatim drive prefix is not an independently openable path.
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        match fs::symlink_metadata(&part) {
            Ok(meta) if is_link(&meta) => return Err("meeting_link"),
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err("meeting_io"),
        }
    }
    Ok(())
}
fn regular(path: &Path) -> StoreResult<fs::Metadata> {
    safe_existing(path)?;
    let meta = fs::symlink_metadata(path).map_err(|_| "meeting_io")?;
    if !meta.is_file() {
        return Err("meeting_path");
    }
    reject_hardlink(&File::open(path).map_err(|_| "meeting_io")?)?;
    Ok(meta)
}
fn reject_hardlink(file: &File) -> StoreResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if file.metadata().map_err(|_| "meeting_io")?.nlink() != 1 {
            return Err("meeting_link");
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
            return Err("meeting_io");
        }
        if info.links != 1 {
            return Err("meeting_link");
        }
    }
    Ok(())
}
fn validate_format(format: &AudioFormat) -> StoreResult<usize> {
    if !(8000..=384000).contains(&format.sample_rate) || !(1..=32).contains(&format.channels) {
        return Err("meeting_format");
    }
    match format.encoding {
        AudioEncoding::Pcm
            if [8, 16, 24, 32].contains(&format.bits_per_sample)
                && (1..=format.bits_per_sample).contains(&format.valid_bits) => {}
        AudioEncoding::Float if format.bits_per_sample == 32 && format.valid_bits == 32 => (),
        _ => return Err("meeting_format"),
    }
    Ok(format.channels as usize * (format.bits_per_sample as usize / 8))
}
fn validate_packet(packet: &CapturePacket) -> StoreResult<()> {
    let align = validate_format(&packet.format)?;
    if packet.frames == 0
        || packet.frames as u64 > packet.format.sample_rate as u64 * 5
        || packet.data.len() > MAX_PACKET_BYTES
    {
        return Err("meeting_packet");
    }
    let bytes = (packet.frames as usize)
        .checked_mul(align)
        .ok_or("meeting_packet")?;
    if bytes > MAX_PACKET_BYTES
        || (!(packet.flags & 2 != 0 && packet.data.is_empty()) && packet.data.len() != bytes)
    {
        return Err("meeting_packet");
    }
    if packet.format.encoding == AudioEncoding::Float
        && packet
            .data
            .chunks_exact(4)
            .any(|b| !f32::from_le_bytes(b.try_into().unwrap()).is_finite())
    {
        return Err("meeting_packet");
    }
    Ok(())
}
fn packet_end(packet: &CapturePacket) -> u64 {
    packet
        .qpc_100ns
        .saturating_add(packet.frames as u64 * 10_000_000 / packet.format.sample_rate as u64)
}
fn source_index(source: Source) -> usize {
    match source {
        Source::Microphone => 0,
        Source::Remote => 1,
    }
}

impl MeetingStore {
    pub fn new(root: impl AsRef<Path>) -> StoreResult<Self> {
        let root = if root.as_ref().is_absolute() {
            root.as_ref().to_owned()
        } else {
            std::env::current_dir()
                .map_err(|_| "meeting_path")?
                .join(root)
        };
        safe_existing(&root)?;
        fs::create_dir_all(&root).map_err(|_| "meeting_io")?;
        safe_existing(&root)?;
        if !fs::metadata(&root).map_err(|_| "meeting_io")?.is_dir() {
            return Err("meeting_path");
        }
        Ok(Self {
            root: fs::canonicalize(root).map_err(|_| "meeting_path")?,
        })
    }
    fn directory(&self, id: &str) -> StoreResult<PathBuf> {
        if !valid_id(id) {
            return Err("meeting_id");
        }
        let directory = self.root.join(id);
        safe_existing(&directory)?;
        if !fs::metadata(&directory).map_err(|_| "meeting_io")?.is_dir() {
            return Err("meeting_path");
        }
        Ok(directory)
    }
    pub fn create(
        &self,
        id: &str,
        title: &str,
        start_qpc_100ns: u64,
        devices: Vec<DeviceInfo>,
    ) -> StoreResult<MeetingWriter> {
        if !valid_id(id) || title.len() > 4096 || devices.len() > 2 {
            return Err("meeting_id");
        }
        safe_existing(&self.root)?;
        let directory = self.root.join(id);
        fs::create_dir(&directory).map_err(|_| "meeting_exists")?;
        let lease = SessionLease::acquire(&directory)?;
        let manifest = MeetingManifest {
            schema_version: 1,
            id: id.into(),
            title: title.into(),
            created_at_unix_secs: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| "meeting_clock")?
                .as_secs(),
            start_qpc_100ns,
            end_qpc_100ns: None,
            devices,
            status: MeetingStatus::Recording,
            error: None,
            chunks: vec![],
            source_health: vec![],
        };
        atomic_manifest(&directory, &manifest)?;
        Ok(MeetingWriter {
            store: self.clone(),
            directory,
            manifest,
            open: [None, None],
            _lease: lease,
            last_checkpoint: Instant::now(),
            failed: false,
            stats: WriterStats::default(),
        })
    }
    pub fn load(&self, id: &str) -> StoreResult<MeetingManifest> {
        let directory = self.directory(id)?;
        let path = directory.join("manifest.json");
        if regular(&path)?.len() > MAX_MANIFEST_BYTES {
            return Err("meeting_manifest");
        }
        let manifest: MeetingManifest =
            serde_json::from_reader(File::open(path).map_err(|_| "meeting_io")?)
                .map_err(|_| "meeting_manifest")?;
        if manifest.schema_version != 1 || manifest.id != id || manifest.title.len() > 4096 {
            return Err("meeting_manifest");
        }
        let mut names = HashSet::new();
        for chunk in &manifest.chunks {
            if !chunk_name(&chunk.file)
                || !names.insert(&chunk.file)
                || chunk.committed_bytes < 8
                || chunk.committed_bytes > MAX_CHUNK_BYTES + MAX_PACKET_BYTES as u64 + 48
            {
                return Err("meeting_manifest");
            }
            validate_format(&chunk.format)?;
            safe_existing(&directory.join(&chunk.file))?;
        }
        Ok(manifest)
    }
    pub fn list(&self) -> StoreResult<Vec<MeetingManifest>> {
        safe_existing(&self.root)?;
        let mut result = vec![];
        for entry in fs::read_dir(&self.root).map_err(|_| "meeting_io")? {
            let entry = entry.map_err(|_| "meeting_io")?;
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if !valid_id(&id) {
                continue;
            }
            if is_link(&fs::symlink_metadata(entry.path()).map_err(|_| "meeting_io")?) {
                return Err("meeting_link");
            }
            if entry.file_type().map_err(|_| "meeting_io")?.is_dir() {
                result.push(self.load(&id)?);
            }
        }
        result.sort_by(|a, b| {
            b.created_at_unix_secs
                .cmp(&a.created_at_unix_secs)
                .then(a.id.cmp(&b.id))
        });
        Ok(result)
    }
    pub fn open_reader(&self, id: &str, file: &str) -> StoreResult<ChunkReader> {
        if !chunk_name(file) {
            return Err("meeting_path");
        }
        let manifest = self.load(id)?;
        let meta = manifest
            .chunks
            .into_iter()
            .find(|c| c.file == file)
            .ok_or("meeting_chunk")?;
        self.open_reader_for_chunk(id, &meta)
    }
    /// Reuse metadata returned by load/recover without reparsing an entire manifest for each chunk.
    pub fn open_reader_for_chunk(&self, id: &str, meta: &ChunkMeta) -> StoreResult<ChunkReader> {
        if !chunk_name(&meta.file)
            || meta.committed_bytes < 8
            || meta.committed_bytes > MAX_CHUNK_BYTES + MAX_PACKET_BYTES as u64 + 48
        {
            return Err("meeting_manifest");
        }
        validate_format(&meta.format)?;
        let path = self.directory(id)?.join(&meta.file);
        if regular(&path)?.len() != meta.committed_bytes {
            return Err("meeting_chunk");
        }
        ChunkReader::open(path, meta.clone())
    }
    pub fn recover(&self, id: &str) -> StoreResult<MeetingManifest> {
        self.recover_with_scanner(id, |path, meta| {
            let length = regular(path)?.len();
            scan_chunk(
                File::open(path).map_err(|_| "meeting_io")?,
                meta.clone(),
                length,
            )
        })
    }
    fn recover_with_scanner(
        &self,
        id: &str,
        mut scanner: impl FnMut(&Path, &ChunkMeta) -> StoreResult<ChunkMeta>,
    ) -> StoreResult<MeetingManifest> {
        let directory = self.directory(id)?;
        let _lease = SessionLease::acquire(&directory)?;
        let mut manifest = self.load(id)?;
        // Scan every chunk without mutation. A later read I/O failure must not truncate an earlier file.
        for meta in &mut manifest.chunks {
            let path = directory.join(&meta.file);
            *meta = scanner(&path, meta)?;
        }
        // All scans succeeded. Pre-open every destination before the first truncation.
        let mut destinations = Vec::with_capacity(manifest.chunks.len());
        for meta in &manifest.chunks {
            let path = directory.join(&meta.file);
            regular(&path)?;
            let file = OpenOptions::new()
                .write(true)
                .open(&path)
                .map_err(|_| "meeting_io")?;
            destinations.push(file);
        }
        for (meta, file) in manifest.chunks.iter().zip(&destinations) {
            file.set_len(meta.committed_bytes)
                .map_err(|_| "meeting_io")?;
            file.sync_all().map_err(|_| "meeting_io")?;
        }
        manifest.status = MeetingStatus::Interrupted;
        manifest.error = Some("meeting_recovered".into());
        manifest.end_qpc_100ns = manifest
            .chunks
            .iter()
            .filter(|c| c.packets != 0)
            .map(|c| c.end_qpc_100ns)
            .max();
        atomic_manifest(&directory, &manifest)?;
        Ok(manifest)
    }
    pub fn delete(&self, id: &str) -> StoreResult<()> {
        let directory = self.directory(id)?;
        let lease = SessionLease::acquire(&directory)?;
        self.load(id)?;
        let mut paths = vec![];
        for entry in fs::read_dir(&directory).map_err(|_| "meeting_io")? {
            let path = entry.map_err(|_| "meeting_io")?.path();
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or("meeting_path")?;
            if name != "manifest.json"
                && name != ".recording.lock"
                && !chunk_name(name)
                && !(name.starts_with(".manifest-")
                    && name.ends_with(".tmp")
                    && name[10..name.len() - 4]
                        .bytes()
                        .all(|b| b.is_ascii_digit() || b == b'-'))
            {
                return Err("meeting_path");
            }
            // The exclusive lease already checked the marker's handle/link count.
            if name != ".recording.lock" {
                regular(&path)?;
            }
            paths.push(path);
        }
        // Check all entries first, and never recurse into unknown entries.
        for path in paths
            .iter()
            .filter(|p| p.file_name().unwrap() != ".recording.lock")
        {
            safe_existing(path)?;
            fs::remove_file(path).map_err(|_| "meeting_io")?;
        }
        drop(lease);
        fs::remove_file(directory.join(".recording.lock")).map_err(|_| "meeting_io")?;
        fs::remove_dir(directory).map_err(|_| "meeting_io")
    }
    pub fn prune(&self, now_unix_secs: u64) -> StoreResult<Vec<String>> {
        let mut removed = vec![];
        for manifest in self.list()? {
            if now_unix_secs.saturating_sub(manifest.created_at_unix_secs) >= RETENTION_SECS {
                match self.delete(&manifest.id) {
                    Ok(()) => removed.push(manifest.id),
                    Err("meeting_active") => (),
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(removed)
    }
}
impl MeetingWriter {
    pub fn checkpoint_if_due(&mut self) -> StoreResult<bool> {
        if self.last_checkpoint.elapsed() < Duration::from_secs(1) {
            return Ok(false);
        }
        self.checkpoint(
            self.manifest.status,
            self.manifest.end_qpc_100ns,
            self.manifest.error.clone(),
        )?;
        Ok(true)
    }
    pub fn heartbeat(&mut self, source: Source, epoch: u32, qpc_100ns: u64) -> StoreResult<()> {
        if self.failed {
            return Err("meeting_writer_failed");
        }
        if let Some(health) = self
            .manifest
            .source_health
            .iter_mut()
            .find(|h| h.source == source && h.epoch == epoch)
        {
            health.last_heartbeat_qpc_100ns = health.last_heartbeat_qpc_100ns.max(qpc_100ns);
        } else {
            if self.manifest.source_health.len() >= 100_000 {
                return Err("meeting_epoch_limit");
            }
            self.manifest.source_health.push(SourceHealth {
                source,
                epoch,
                first_heartbeat_qpc_100ns: qpc_100ns,
                last_heartbeat_qpc_100ns: qpc_100ns,
            });
        }
        self.checkpoint_if_due()?;
        Ok(())
    }
    pub fn manifest(&self) -> &MeetingManifest {
        &self.manifest
    }
    pub fn stats(&self) -> WriterStats {
        self.stats
    }
    fn close_chunk(&mut self, source: usize) -> StoreResult<()> {
        if let Some((index, file)) = self.open[source].take() {
            file.sync_all().map_err(|_| "meeting_io")?;
            self.manifest.chunks[index].finalized = true;
        }
        Ok(())
    }
    pub fn append(&mut self, packet: CapturePacket) -> StoreResult<()> {
        if self.failed {
            return Err("meeting_writer_failed");
        }
        validate_packet(&packet)?;
        let result = self.append_valid(packet);
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn append_valid(&mut self, packet: CapturePacket) -> StoreResult<()> {
        self.store.directory(&self.manifest.id)?;
        let source = source_index(packet.source);
        let bytes = 40 + packet.data.len() as u64;
        let rollover = self.open[source].as_ref().is_some_and(|(index, _)| {
            let meta = &self.manifest.chunks[*index];
            meta.epoch != packet.epoch
                || meta.format != packet.format
                || meta.committed_bytes + bytes > MAX_CHUNK_BYTES
                || meta.frames + packet.frames as u64 > packet.format.sample_rate as u64 * 5
        });
        if rollover {
            self.close_chunk(source)?;
        }
        if self.open[source].is_none() {
            let index = self.manifest.chunks.len();
            if index >= 100_000 {
                return Err("meeting_chunk_limit");
            }
            let name = format!("chunk-{index:08}.bin");
            let path = self.directory.join(&name);
            safe_existing(&path)?;
            let mut file = OpenOptions::new()
                .write(true)
                .read(true)
                .create_new(true)
                .open(path)
                .map_err(|_| "meeting_io")?;
            file.write_all(MAGIC).map_err(|_| "meeting_io")?;
            file.sync_all().map_err(|_| "meeting_io")?;
            self.manifest.chunks.push(ChunkMeta {
                file: name,
                source: packet.source,
                epoch: packet.epoch,
                format: packet.format.clone(),
                start_qpc_100ns: packet.qpc_100ns,
                end_qpc_100ns: packet.qpc_100ns,
                packets: 0,
                frames: 0,
                committed_bytes: 8,
                finalized: false,
            });
            self.open[source] = Some((index, file));
            // Publish format/source before audio, so every saved packet is identifiable.
            self.checkpoint(
                self.manifest.status,
                self.manifest.end_qpc_100ns,
                self.manifest.error.clone(),
            )?;
        }
        let (index, file) = self.open[source].as_mut().unwrap();
        let mut header = [0u8; 40];
        header[0..4].copy_from_slice(&packet.epoch.to_le_bytes());
        header[4..12].copy_from_slice(&packet.sequence.to_le_bytes());
        header[12..20].copy_from_slice(&packet.device_frame_position.to_le_bytes());
        header[20..28].copy_from_slice(&packet.qpc_100ns.to_le_bytes());
        header[28..32].copy_from_slice(&packet.frames.to_le_bytes());
        header[32..36].copy_from_slice(&packet.flags.to_le_bytes());
        header[36..40].copy_from_slice(&(packet.data.len() as u32).to_le_bytes());
        file.write_all(&header)
            .and_then(|_| file.write_all(&packet.data))
            .map_err(|_| "meeting_io")?;
        let meta = &mut self.manifest.chunks[*index];
        meta.packets += 1;
        meta.frames += packet.frames as u64;
        meta.committed_bytes += bytes;
        meta.end_qpc_100ns = meta.end_qpc_100ns.max(packet_end(&packet));
        self.checkpoint_if_due()?;
        Ok(())
    }
    pub fn checkpoint(
        &mut self,
        status: MeetingStatus,
        end_qpc_100ns: Option<u64>,
        error: Option<String>,
    ) -> StoreResult<()> {
        let began = Instant::now();
        let result = self.checkpoint_impl(status, end_qpc_100ns, error);
        self.stats.max_checkpoint_us = self
            .stats
            .max_checkpoint_us
            .max(began.elapsed().as_micros().min(u64::MAX as u128) as u64);
        self.stats.checkpoints += 1;
        result
    }
    fn checkpoint_impl(
        &mut self,
        status: MeetingStatus,
        end_qpc_100ns: Option<u64>,
        error: Option<String>,
    ) -> StoreResult<()> {
        self.store.directory(&self.manifest.id)?;
        if error.as_ref().is_some_and(|e| {
            e.len() > 96
                || !e
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        }) {
            return Err("meeting_error_code");
        }
        for (_, file) in self.open.iter().flatten() {
            file.sync_all().map_err(|_| "meeting_io")?;
        }
        let mut next = self.manifest.clone();
        next.status = status;
        next.end_qpc_100ns = end_qpc_100ns;
        next.error = error;
        atomic_manifest(&self.directory, &next)?;
        self.manifest = next;
        self.last_checkpoint = Instant::now();
        Ok(())
    }
    pub fn finish(
        mut self,
        status: MeetingStatus,
        end_qpc_100ns: Option<u64>,
        error: Option<String>,
    ) -> StoreResult<MeetingManifest> {
        if self.failed && status != MeetingStatus::Interrupted && status != MeetingStatus::Failed {
            return Err("meeting_writer_failed");
        }
        self.close_chunk(0)?;
        self.close_chunk(1)?;
        self.checkpoint(status, end_qpc_100ns, error)?;
        Ok(self.manifest.clone())
    }
}
impl ChunkReader {
    fn open(path: impl AsRef<Path>, meta: ChunkMeta) -> StoreResult<Self> {
        Self::from_reader(File::open(path).map_err(|_| "meeting_io")?, meta)
    }
}
impl<R: Read> ChunkReader<R> {
    fn from_reader(mut file: R, meta: ChunkMeta) -> StoreResult<Self> {
        let mut magic = [0; 8];
        read_chunk_exact(&mut file, &mut magic).map_err(ChunkReadError::code)?;
        if &magic != MAGIC || meta.committed_bytes < 8 {
            return Err("meeting_chunk");
        }
        let remaining = meta.committed_bytes - 8;
        Ok(Self {
            file,
            meta,
            remaining,
        })
    }
    pub fn next_packet(&mut self) -> StoreResult<Option<CapturePacket>> {
        self.next_packet_checked().map_err(ChunkReadError::code)
    }
    fn next_packet_checked(&mut self) -> Result<Option<CapturePacket>, ChunkReadError> {
        if self.remaining == 0 {
            return Ok(None);
        }
        if self.remaining < 40 {
            return Err(ChunkReadError::InvalidTail);
        }
        let mut header = [0; 40];
        read_chunk_exact(&mut self.file, &mut header)?;
        let u32_at = |offset| u32::from_le_bytes(header[offset..offset + 4].try_into().unwrap());
        let u64_at = |offset| u64::from_le_bytes(header[offset..offset + 8].try_into().unwrap());
        let length = u32_at(36) as usize;
        if length > MAX_PACKET_BYTES || length as u64 + 40 > self.remaining {
            return Err(ChunkReadError::InvalidTail);
        }
        let mut packet = CapturePacket {
            source: self.meta.source,
            epoch: u32_at(0),
            sequence: u64_at(4),
            device_frame_position: u64_at(12),
            qpc_100ns: u64_at(20),
            frames: u32_at(28),
            flags: u32_at(32),
            format: self.meta.format.clone(),
            data: vec![0; length],
        };
        if packet.epoch != self.meta.epoch {
            return Err(ChunkReadError::InvalidTail);
        }
        read_chunk_exact(&mut self.file, &mut packet.data)?;
        validate_packet(&packet).map_err(|_| ChunkReadError::InvalidTail)?;
        self.remaining -= length as u64 + 40;
        Ok(Some(packet))
    }
}
fn scan_chunk(reader: impl Read, mut meta: ChunkMeta, length: u64) -> StoreResult<ChunkMeta> {
    if length > MAX_CHUNK_BYTES + MAX_PACKET_BYTES as u64 + 48 {
        return Err("meeting_chunk");
    }
    let mut scan_meta = meta.clone();
    scan_meta.committed_bytes = length;
    let mut reader = ChunkReader::from_reader(reader, scan_meta)?;
    let mut bytes = 8;
    let mut packets = 0;
    let mut frames = 0;
    let mut start = None;
    let mut end = 0;
    loop {
        match reader.next_packet_checked() {
            Ok(Some(packet)) => {
                bytes += 40 + packet.data.len() as u64;
                packets += 1;
                frames += packet.frames as u64;
                start.get_or_insert(packet.qpc_100ns);
                end = end.max(packet_end(&packet));
            }
            Ok(None) | Err(ChunkReadError::InvalidTail) => break,
            Err(ChunkReadError::Io) => return Err("meeting_io"),
        }
    }
    meta.committed_bytes = bytes;
    meta.packets = packets;
    meta.frames = frames;
    meta.start_qpc_100ns = start.unwrap_or(meta.start_qpc_100ns);
    meta.end_qpc_100ns = end;
    meta.finalized = true;
    Ok(meta)
}
fn atomic_manifest(directory: &Path, manifest: &MeetingManifest) -> StoreResult<()> {
    safe_existing(directory)?;
    let target = directory.join("manifest.json");
    safe_existing(&target)?;
    if target.exists() {
        regular(&target)?;
    }
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = directory.join(format!(".manifest-{}-{sequence}.tmp", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| "meeting_io")?;
        serialize_manifest_to(&mut file, manifest)?;
        file.sync_all().map_err(|_| "meeting_io")?;
        drop(file);
        atomic_replace(&temporary, &target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
fn serialize_manifest_to(writer: impl Write, manifest: &MeetingManifest) -> StoreResult<()> {
    let mut buffered = BufWriter::with_capacity(64 * 1024, writer);
    serde_json::to_writer(&mut buffered, manifest).map_err(|error| {
        if error.is_io() {
            "meeting_io"
        } else {
            "meeting_manifest"
        }
    })?;
    // Flush explicitly: BufWriter::drop discards flush failures. The caller syncs only after this succeeds.
    buffered.flush().map_err(|_| "meeting_io")
}
#[cfg(windows)]
fn atomic_replace(from: &Path, to: &Path) -> StoreResult<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // Replace in one operation, preserving the old manifest if replacement fails.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0x1 | 0x8) } == 0 {
        Err("meeting_atomic_replace")
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
fn atomic_replace(from: &Path, to: &Path) -> StoreResult<()> {
    fs::rename(from, to).map_err(|_| "meeting_atomic_replace")?;
    File::open(to.parent().ok_or("meeting_path")?)
        .and_then(|f| f.sync_all())
        .map_err(|_| "meeting_io")
}
#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
