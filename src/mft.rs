//! NTFS Master File Table scanner (Windows only).
//!
//! Reads the metadata of every file on a volume straight out of the MFT
//! through a raw volume handle (e.g. `\\.\C:`) instead of walking
//! directories. This is the same trick tools like WizTree and Everything
//! use. It requires administrator privileges and a local NTFS volume.
//!
//! The scan reads all FILE records, collects every non-DOS $FILE_NAME
//! (hard links produce one entry per link), takes timestamps and the
//! hidden flag from $STANDARD_INFORMATION, and takes the file size from
//! the unnamed $DATA attribute (authoritative; $FILE_NAME's copy is only
//! lazily updated). Full paths are then reconstructed from the parent
//! directory references and filtered down to the requested subtree.

use std::collections::HashMap;
use std::ffi::c_void;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::thread;

use ntfs::structured_values::{
    NtfsFileAttributeFlags, NtfsFileName, NtfsFileNamespace, NtfsStandardInformation,
};
use ntfs::{KnownNtfsFileRecordNumber, Ntfs, NtfsAttributeType, NtfsFileFlags, NtfsTime};
use pyo3::exceptions::{PyOSError, PyValueError};
use pyo3::{PyErr, PyResult};

/// Raw volume reads must be sector aligned; 4096 is a multiple of every
/// sector size in practice (512e and 4Kn disks).
const SECTOR_SIZE: usize = 4096;
/// Cache granularity above the sector layer.
const CHUNK_SIZE: u64 = 1 << 20;
/// Number of chunks kept in the per-thread read cache.
const MAX_CHUNKS: usize = 8;
/// The first 24 file records are reserved for NTFS metadata files
/// ($MFT, $LogFile, $Extend, ...). Directory enumeration never returns
/// them, so the MFT backend hides them too.
const RESERVED_RECORDS: u64 = 24;
/// Seconds between the NT epoch (1601-01-01) and the unix epoch.
const NT_TO_UNIX_SECS: f64 = 11_644_473_600.0;

// CTL_CODE(FILE_DEVICE_DISK, 0x17, METHOD_BUFFERED, FILE_READ_ACCESS).
const IOCTL_DISK_GET_LENGTH_INFO: u32 = 0x0007_405c;

#[repr(C)]
struct GetLengthInformation {
    length: i64,
}

#[link(name = "kernel32")]
extern "system" {
    fn DeviceIoControl(
        device: *mut c_void,
        control_code: u32,
        in_buffer: *mut c_void,
        in_buffer_size: u32,
        out_buffer: *mut c_void,
        out_buffer_size: u32,
        bytes_returned: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
}

fn nt_to_unix(t: NtfsTime) -> Option<f64> {
    let nt = t.nt_timestamp();
    if nt == 0 {
        None
    } else {
        Some(nt as f64 / 1e7 - NT_TO_UNIX_SECS)
    }
}

fn requirement() -> &'static str {
    "MFT scanning requires administrator privileges and a local NTFS volume"
}

// ─── Sector-aligned reader ──────────────────────────────────

/// Adapted from the `ntfs` crate's ntfs-shell example (sector_reader.rs,
/// Copyright 2021 Colin Finck <colin@reactos.org>, MIT OR Apache-2.0).
/// Raw volume handles on Windows only accept reads and seeks on sector
/// boundaries, so this wrapper aligns every access. One change from the
/// original: `read` seeks the inner reader itself, so the position stays
/// correct even for consecutive reads without an interleaving seek.
struct SectorReader<R>
where
    R: Read + Seek,
{
    inner: R,
    sector_size: usize,
    /// Position as seen by the caller; the inner reader only ever sees
    /// sector-aligned positions.
    stream_position: u64,
    /// Kept allocated between reads as a small performance optimization.
    temp_buf: Vec<u8>,
}

impl<R> SectorReader<R>
where
    R: Read + Seek,
{
    fn new(inner: R, sector_size: usize) -> io::Result<Self> {
        if !sector_size.is_power_of_two() {
            return Err(io::Error::other("sector_size is not a power of two"));
        }

        Ok(Self {
            inner,
            sector_size,
            stream_position: 0,
            temp_buf: Vec::new(),
        })
    }

    fn align_down_to_sector_size(&self, n: u64) -> u64 {
        n / self.sector_size as u64 * self.sector_size as u64
    }

    fn align_up_to_sector_size(&self, n: u64) -> u64 {
        self.align_down_to_sector_size(n) + self.sector_size as u64
    }
}

impl<R> Read for SectorReader<R>
where
    R: Read + Seek,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Zero-length reads must not touch the underlying storage: the
        // sector-aligned read below fetches a padded sector, which could
        // spuriously fail with UnexpectedEof near the end of the volume.
        if buf.is_empty() {
            return Ok(0);
        }
        // We can only read from a sector boundary. Align down to find the
        // real read position and read enough extra bytes to cover both the
        // alignment difference and the alignment of the total length.
        let aligned_position = self.align_down_to_sector_size(self.stream_position);
        let start = (self.stream_position - aligned_position) as usize;
        let end = start + buf.len();
        let aligned_bytes_to_read = self.align_up_to_sector_size(end as u64) as usize;

        self.temp_buf.resize(aligned_bytes_to_read, 0);
        self.inner.seek(SeekFrom::Start(aligned_position))?;
        self.inner.read_exact(&mut self.temp_buf)?;
        buf.copy_from_slice(&self.temp_buf[start..end]);

        self.stream_position += buf.len() as u64;
        Ok(buf.len())
    }
}

impl<R> Seek for SectorReader<R>
where
    R: Read + Seek,
{
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(_) => {
                // Unsupported: the raw partition size cannot be determined
                // by seeking to the end on Windows.
                return Err(io::Error::other(
                    "SeekFrom::End is unsupported for SectorReader",
                ));
            }
            SeekFrom::Current(n) => {
                if n >= 0 {
                    self.stream_position.checked_add(n as u64)
                } else {
                    self.stream_position.checked_sub(n.wrapping_neg() as u64)
                }
            }
        };

        match new_pos {
            Some(n) => {
                // `read` aligns and seeks the inner reader itself, so only
                // the caller-visible position needs to be tracked here.
                self.stream_position = n;
                Ok(n)
            }
            None => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid seek to a negative or overflowing position",
            )),
        }
    }
}

// ─── Chunked read cache ─────────────────────────────────────

/// Read-through cache layered above `SectorReader`. The ntfs crate issues
/// thousands of small reads per file record; fetching fixed-size aligned
/// chunks lets those hit memory instead of the disk.
struct ChunkCache<R>
where
    R: Read + Seek,
{
    inner: R,
    pos: u64,
    /// Most recently used first.
    chunks: Vec<(u64, Vec<u8>)>,
}

impl<R> ChunkCache<R>
where
    R: Read + Seek,
{
    fn new(inner: R) -> Self {
        Self {
            inner,
            pos: 0,
            chunks: Vec::with_capacity(MAX_CHUNKS),
        }
    }

    fn chunk(&mut self, index: u64) -> io::Result<&Vec<u8>> {
        if let Some(i) = self.chunks.iter().position(|(idx, _)| *idx == index) {
            if i != 0 {
                let hit = self.chunks.remove(i);
                self.chunks.insert(0, hit);
            }
            return Ok(&self.chunks[0].1);
        }

        let start = index * CHUNK_SIZE;
        let mut data = vec![0u8; CHUNK_SIZE as usize];
        self.inner.seek(SeekFrom::Start(start))?;
        if let Err(e) = self.inner.read_exact(&mut data) {
            if e.kind() != io::ErrorKind::UnexpectedEof {
                return Err(e);
            }
            // Near the end of the volume a full chunk may not be readable:
            // fall back to sector-sized steps and keep whatever was read.
            data.clear();
            self.inner.seek(SeekFrom::Start(start))?;
            let mut step = vec![0u8; SECTOR_SIZE];
            while data.len() < CHUNK_SIZE as usize {
                match self.inner.read_exact(&mut step) {
                    Ok(()) => data.extend_from_slice(&step),
                    Err(_) => break,
                }
            }
        }

        if self.chunks.len() >= MAX_CHUNKS {
            self.chunks.pop();
        }
        self.chunks.insert(0, (index, data));
        Ok(&self.chunks[0].1)
    }
}

impl<R> Read for ChunkCache<R>
where
    R: Read + Seek,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut total = 0;
        while total < buf.len() {
            let index = self.pos / CHUNK_SIZE;
            let offset = (self.pos % CHUNK_SIZE) as usize;
            let n = {
                let chunk = self.chunk(index)?;
                if offset >= chunk.len() {
                    0
                } else {
                    let n = (buf.len() - total).min(chunk.len() - offset);
                    buf[total..total + n].copy_from_slice(&chunk[offset..offset + n]);
                    n
                }
            };
            if n == 0 {
                break;
            }
            total += n;
            self.pos += n as u64;
        }
        Ok(total)
    }
}

impl<R> Seek for ChunkCache<R>
where
    R: Read + Seek,
{
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new_pos = match pos {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(_) => {
                return Err(io::Error::other(
                    "SeekFrom::End is unsupported for ChunkCache",
                ));
            }
            SeekFrom::Current(n) => {
                if n >= 0 {
                    self.pos.checked_add(n as u64)
                } else {
                    self.pos.checked_sub(n.wrapping_neg() as u64)
                }
            }
        };

        match new_pos {
            Some(n) => {
                self.pos = n;
                Ok(n)
            }
            None => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid seek to a negative or overflowing position",
            )),
        }
    }
}

// ─── MFT scanning ───────────────────────────────────────────

type VolumeReader = ChunkCache<SectorReader<File>>;

/// One resolved name of a scanned entry.
pub struct MftEntry {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<f64>,
    pub created: Option<f64>,
    /// True when the entry or any ancestor below the scan root is hidden
    /// (hidden attribute or dot-prefixed name).
    pub hidden: bool,
    /// Path components below the scan root; the root itself is 0.
    pub depth: usize,
}

pub struct MftScan {
    /// Canonical path of the scanned directory; all entry paths start
    /// with it.
    pub root: String,
    pub entries: Vec<MftEntry>,
}

/// Everything extracted from one in-use FILE record.
struct RecordData {
    frn: u64,
    is_dir: bool,
    size: u64,
    modified: Option<f64>,
    created: Option<f64>,
    attr_hidden: bool,
    /// (parent record number, name, is Win32 or Win32AndDos namespace).
    names: Vec<(u64, String, bool)>,
}

struct DirInfo {
    parent: u64,
    name: String,
    hidden: bool,
}

fn unc_error(directory: &str) -> PyErr {
    PyOSError::new_err(format!(
        "Cannot MFT-scan '{}': UNC and network paths are not supported. {}.",
        directory,
        requirement()
    ))
}

fn open_volume_file(volume: &str) -> PyResult<File> {
    File::open(volume).map_err(|e| {
        PyOSError::new_err(format!(
            "Cannot open volume {} for MFT scanning: {}. {}.",
            volume,
            e,
            requirement()
        ))
    })
}

fn reader_from_file(file: File) -> PyResult<VolumeReader> {
    let sector_reader = SectorReader::new(file, SECTOR_SIZE)
        .map_err(|e| PyOSError::new_err(format!("MFT scan failed: {}", e)))?;
    Ok(ChunkCache::new(sector_reader))
}

fn open_volume_reader(volume: &str) -> PyResult<VolumeReader> {
    reader_from_file(open_volume_file(volume)?)
}

fn volume_length(file: &File) -> io::Result<u64> {
    let mut info = GetLengthInformation { length: 0 };
    let mut bytes_returned = 0u32;
    // SAFETY: `file` is a live raw-volume handle, the output pointer refers
    // to an initialized, correctly sized GET_LENGTH_INFORMATION-compatible
    // buffer, and all unused pointer arguments are null.
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            IOCTL_DISK_GET_LENGTH_INFO,
            std::ptr::null_mut(),
            0,
            (&mut info as *mut GetLengthInformation).cast(),
            std::mem::size_of::<GetLengthInformation>() as u32,
            &mut bytes_returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    u64::try_from(info.length)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid volume length"))
}

fn validated_record_count(mft_len: u64, record_size: u64, volume_len: u64) -> Result<u64, &'static str> {
    if record_size == 0 {
        return Err("MFT record size is zero");
    }
    if mft_len > volume_len {
        return Err("declared MFT length exceeds the physical volume length");
    }
    if mft_len % record_size != 0 {
        return Err("declared MFT length is not aligned to the record size");
    }
    Ok(mft_len / record_size)
}

fn select_child_record(candidates: &[(u64, String)], component: &str) -> Result<u64, &'static str> {
    let mut exact = candidates.iter().filter(|(_, name)| name == component);
    if let Some((frn, _)) = exact.next() {
        if exact.next().is_none() {
            return Ok(*frn);
        }
        return Err("multiple exact MFT directory matches");
    }
    if candidates.len() == 1 {
        return Ok(candidates[0].0);
    }
    Err("ambiguous case-folded MFT directory match")
}

fn parse_record(ntfs: &Ntfs, fs: &mut VolumeReader, frn: u64) -> Option<RecordData> {
    // Unallocated and invalid records fail to parse: skip them.
    let file = ntfs.file(fs, frn).ok()?;
    if !file.flags().contains(NtfsFileFlags::IN_USE) {
        return None;
    }

    let is_dir = file.is_directory();
    let mut modified = None;
    let mut created = None;
    let mut attr_hidden = false;
    let mut size = 0u64;
    let mut have_size = false;
    let mut names: Vec<(u64, String, bool)> = Vec::new();

    let mut attrs = file.attributes();
    while let Some(item) = attrs.next(fs) {
        let item = match item {
            Ok(i) => i,
            Err(_) => break,
        };
        let attribute = match item.to_attribute() {
            Ok(a) => a,
            Err(_) => continue,
        };
        let ty = match attribute.ty() {
            Ok(t) => t,
            Err(_) => continue,
        };
        match ty {
            NtfsAttributeType::StandardInformation => {
                if let Ok(info) = attribute.structured_value::<_, NtfsStandardInformation>(fs) {
                    modified = nt_to_unix(info.modification_time());
                    created = nt_to_unix(info.creation_time());
                    attr_hidden = info
                        .file_attributes()
                        .contains(NtfsFileAttributeFlags::HIDDEN);
                }
            }
            NtfsAttributeType::FileName => {
                if let Ok(file_name) = attribute.structured_value::<_, NtfsFileName>(fs) {
                    // Skip DOS-namespace-only short names (e.g. PROGRA~1);
                    // every remaining $FILE_NAME is one hard link.
                    if file_name.namespace() != NtfsFileNamespace::Dos {
                        let win32 = matches!(
                            file_name.namespace(),
                            NtfsFileNamespace::Win32 | NtfsFileNamespace::Win32AndDos
                        );
                        names.push((
                            file_name.parent_directory_reference().file_record_number(),
                            file_name.name().to_string_lossy(),
                            win32,
                        ));
                    }
                }
            }
            // The unnamed $DATA attribute holds the file data; its value
            // length is the authoritative logical size.
            NtfsAttributeType::Data if !have_size && attribute.name_length() == 0 => {
                size = attribute.value_length();
                have_size = true;
            }
            _ => {}
        }
    }

    // Records without any usable name (extension records, unnamed
    // metadata) cannot be placed in the tree.
    if names.is_empty() {
        return None;
    }

    Some(RecordData {
        frn,
        is_dir,
        size,
        modified,
        created,
        attr_hidden,
        names,
    })
}

fn scan_range(volume: &str, start: u64, end: u64) -> Result<Vec<RecordData>, String> {
    let mut fs = open_volume_reader(volume).map_err(|e| e.to_string())?;
    let ntfs = Ntfs::new(&mut fs).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for frn in start..end {
        if let Some(record) = parse_record(&ntfs, &mut fs, frn) {
            out.push(record);
        }
    }
    Ok(out)
}

fn join_path(parent: &str, name: &str) -> String {
    if parent.ends_with('\\') {
        format!("{}{}", parent, name)
    } else {
        format!("{}\\{}", parent, name)
    }
}

/// Resolve the path, hidden chain, and depth of a directory record,
/// memoized. Returns None for directories outside the scan root.
fn resolve_dir(
    frn: u64,
    dirs: &HashMap<u64, DirInfo>,
    memo: &mut HashMap<u64, Option<(String, bool, usize)>>,
) -> Option<(String, bool, usize)> {
    let mut chain: Vec<u64> = Vec::new();
    let mut current = frn;
    let base = loop {
        if let Some(known) = memo.get(&current) {
            break known.clone();
        }
        match dirs.get(&current) {
            // The parent self-reference and depth guards protect against
            // corrupt or cyclic parent chains.
            Some(info) if info.parent != current && chain.len() < 512 => {
                chain.push(current);
                current = info.parent;
            }
            _ => break None,
        }
    };

    let mut acc = base;
    for link in chain.into_iter().rev() {
        let info = &dirs[&link];
        let next = acc.as_ref().map(|(path, hidden, depth)| {
            (
                join_path(path, &info.name),
                *hidden || info.hidden,
                depth + 1,
            )
        });
        memo.insert(link, next.clone());
        acc = next;
    }
    acc
}

/// Scan the volume containing `directory` and return every entry below it.
///
/// `threads` is the number of parallel record-parsing workers; each opens
/// its own raw volume handle.
pub fn scan(directory: &str, threads: usize) -> PyResult<MftScan> {
    // Reject UNC and network paths up front (before existence checks) so
    // unreachable shares still get the right error.
    let raw = directory.trim();
    if raw.starts_with("\\\\") || raw.starts_with("//") {
        return Err(unc_error(directory));
    }

    let canonical = std::fs::canonicalize(directory)
        .map_err(|_| PyOSError::new_err(format!("Path not found: {}", directory)))?;
    if !canonical.is_dir() {
        return Err(PyValueError::new_err(format!(
            "Not a directory: {}",
            directory
        )));
    }
    let canonical = canonical.to_string_lossy().into_owned();
    let stripped = canonical
        .strip_prefix("\\\\?\\")
        .unwrap_or(&canonical)
        .to_string();
    if stripped.starts_with("UNC\\") || stripped.starts_with("\\\\") {
        return Err(unc_error(directory));
    }
    let mut chars = stripped.chars();
    let drive = chars.next().filter(|c| c.is_ascii_alphabetic());
    let colon = chars.next().filter(|c| *c == ':');
    let (drive, _) = match (drive, colon) {
        (Some(d), Some(c)) => (d, c),
        _ => {
            return Err(PyOSError::new_err(format!(
                "Cannot MFT-scan '{}': not a drive-letter path. {}.",
                directory,
                requirement()
            )));
        }
    };

    // "C:" for the volume root, otherwise "D:\some\dir" without a
    // trailing separator.
    let prefix = stripped.trim_end_matches('\\').to_string();
    let display_root = if prefix.len() == 2 {
        format!("{}\\", prefix)
    } else {
        prefix.clone()
    };
    let volume = format!("\\\\.\\{}:", drive);

    // Bootstrap on the main thread so open and parse errors are reported
    // clearly before any workers start.
    let volume_file = open_volume_file(&volume)?;
    let volume_len = volume_length(&volume_file).map_err(|e| {
        PyOSError::new_err(format!(
            "Cannot determine the length of {}: {}. {}.",
            volume,
            e,
            requirement()
        ))
    })?;
    let mut fs = reader_from_file(volume_file)?;
    let ntfs = Ntfs::new(&mut fs).map_err(|e| {
        PyOSError::new_err(format!(
            "Cannot read {} as NTFS: {}. {}.",
            volume,
            e,
            requirement()
        ))
    })?;

    // Total record count = size of the $MFT file's unnamed $DATA stream
    // divided by the file record size.
    let record_size = ntfs.file_record_size() as u64;
    let mft_len = (|| -> Result<u64, ntfs::NtfsError> {
        let mft = ntfs.file(&mut fs, KnownNtfsFileRecordNumber::MFT as u64)?;
        let data_item = match mft.data(&mut fs, "") {
            Some(item) => item?,
            None => return Ok(0),
        };
        Ok(data_item.to_attribute()?.value_length())
    })()
    .map_err(|e| {
        PyOSError::new_err(format!(
            "Cannot read the MFT of {}: {}. {}.",
            volume,
            e,
            requirement()
        ))
    })?;
    let record_count = validated_record_count(mft_len, record_size, volume_len).map_err(|e| {
        PyOSError::new_err(format!("Cannot scan the MFT of {}: {}.", volume, e))
    })?;
    drop(fs);

    // Parse all records in parallel over disjoint record ranges.
    let worker_count = threads.clamp(1, 16).min(record_count.max(1) as usize);
    let per_worker = record_count.div_euclid(worker_count as u64) + 1;
    let results: Vec<Result<Vec<RecordData>, String>> = thread::scope(|scope| {
        let mut handles = Vec::new();
        for worker in 0..worker_count as u64 {
            let start = worker * per_worker;
            let end = ((worker + 1) * per_worker).min(record_count);
            if start >= end {
                continue;
            }
            let volume = volume.as_str();
            handles.push(scope.spawn(move || scan_range(volume, start, end)));
        }
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|_| Err("MFT scan worker panicked".to_string()))
            })
            .collect()
    });

    let mut records: Vec<RecordData> = Vec::new();
    for result in results {
        let partial = result.map_err(|e| {
            PyOSError::new_err(format!("MFT scan of {} failed: {}. {}.", volume, e, requirement()))
        })?;
        records.extend(partial);
    }

    // Directory table for path reconstruction. Reserved metadata records
    // are excluded so nothing resolves through e.g. $Extend.
    let mut dirs: HashMap<u64, DirInfo> = HashMap::new();
    for record in &records {
        if !record.is_dir || record.frn < RESERVED_RECORDS {
            continue;
        }
        // Prefer the Win32 name; POSIX-namespace names are the fallback.
        let (parent, name, _) = record
            .names
            .iter()
            .find(|(_, _, win32)| *win32)
            .unwrap_or(&record.names[0]);
        dirs.insert(
            record.frn,
            DirInfo {
                parent: *parent,
                name: name.clone(),
                hidden: record.attr_hidden || name.starts_with('.'),
            },
        );
    }

    // Find the record number of the scan root by walking the requested
    // path components down from the volume root (record 5).
    let root_record = KnownNtfsFileRecordNumber::RootDirectory as u64;
    let root_frn = if prefix.len() == 2 {
        root_record
    } else {
        let mut lookup: HashMap<(u64, String), Vec<(u64, String)>> =
            HashMap::with_capacity(dirs.len());
        for (frn, info) in &dirs {
            lookup
                .entry((info.parent, info.name.to_lowercase()))
                .or_default()
                .push((*frn, info.name.clone()));
        }
        let mut current = root_record;
        for component in prefix[3..].split('\\') {
            let candidates = lookup
                .get(&(current, component.to_lowercase()))
                .ok_or_else(|| {
                    PyOSError::new_err(format!(
                        "Cannot locate '{}' in the MFT of {}. {}.",
                        directory,
                        volume,
                        requirement()
                    ))
                })?;
            current = select_child_record(candidates, component).map_err(|e| {
                PyOSError::new_err(format!(
                    "Cannot locate '{}' unambiguously in the MFT of {}: {}. {}.",
                    directory,
                    volume,
                    e,
                    requirement()
                ))
            })?;
        }
        current
    };

    // Reconstruct paths and emit every entry below the scan root. The
    // hidden chain starts fresh at the root, matching the walker, which
    // only evaluates hidden-ness below the directory it was given.
    let mut memo: HashMap<u64, Option<(String, bool, usize)>> = HashMap::new();
    memo.insert(root_frn, Some((display_root.clone(), false, 0)));

    let mut entries: Vec<MftEntry> = Vec::with_capacity(records.len());
    for record in &records {
        if record.is_dir {
            if record.frn == root_frn {
                // The scan root itself, like the walker's first entry.
                let name = Path::new(&display_root)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| display_root.clone());
                entries.push(MftEntry {
                    path: display_root.clone(),
                    name,
                    is_dir: true,
                    size: 0,
                    modified: record.modified,
                    created: record.created,
                    hidden: false,
                    depth: 0,
                });
                continue;
            }
            if record.frn < RESERVED_RECORDS {
                continue;
            }
            if let Some((path, hidden, depth)) = resolve_dir(record.frn, &dirs, &mut memo) {
                let name = dirs[&record.frn].name.clone();
                entries.push(MftEntry {
                    path,
                    name,
                    is_dir: true,
                    size: 0,
                    modified: record.modified,
                    created: record.created,
                    hidden,
                    depth,
                });
            }
        } else {
            if record.frn < RESERVED_RECORDS {
                continue;
            }
            for (parent, name, _) in &record.names {
                if let Some((parent_path, parent_hidden, parent_depth)) =
                    resolve_dir(*parent, &dirs, &mut memo)
                {
                    let hidden = parent_hidden || record.attr_hidden || name.starts_with('.');
                    entries.push(MftEntry {
                        path: join_path(&parent_path, name),
                        name: name.clone(),
                        is_dir: false,
                        size: record.size,
                        modified: record.modified,
                        created: record.created,
                        hidden,
                        depth: parent_depth + 1,
                    });
                }
            }
        }
    }

    Ok(MftScan {
        root: display_root,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_count_is_bounded_by_physical_volume() {
        assert_eq!(validated_record_count(4096, 1024, 8192), Ok(4));
        assert!(validated_record_count(8193, 1024, 8192).is_err());
        assert!(validated_record_count(4097, 1024, 8192).is_err());
        assert!(validated_record_count(4096, 0, 8192).is_err());
    }

    #[test]
    fn exact_mft_name_wins_case_fold_collision() {
        let candidates = vec![(10, "project".to_string()), (11, "Project".to_string())];
        assert_eq!(select_child_record(&candidates, "Project"), Ok(11));
        assert_eq!(select_child_record(&candidates, "PROJECT"),
                   Err("ambiguous case-folded MFT directory match"));
    }

    #[test]
    fn unique_folded_mft_name_remains_compatible() {
        let candidates = vec![(10, "Project".to_string())];
        assert_eq!(select_child_record(&candidates, "project"), Ok(10));
    }

    #[test]
    fn sector_reader_zero_length_read_is_a_noop() {
        // Four sectors of data; note that reads of an exact multiple of
        // the sector size intentionally fetch one extra sector (upstream
        // behavior), so the backing storage must have room for it.
        let data: Vec<u8> = (0u8..16).collect();
        let mut reader = SectorReader::new(io::Cursor::new(data), 4).unwrap();

        // Zero-length reads must succeed anywhere, including at EOF where
        // the aligned read would run past the end of the backing storage.
        assert_eq!(reader.seek(SeekFrom::Start(16)).unwrap(), 16);
        assert_eq!(reader.read(&mut []).unwrap(), 0);
        assert_eq!(reader.stream_position, 16);

        // Regular aligned reads still work.
        reader.seek(SeekFrom::Start(0)).unwrap();
        let mut buf = [0u8; 4];
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(buf, [0, 1, 2, 3]);
        // Zero-length reads keep succeeding at EOF afterwards.
        reader.seek(SeekFrom::Start(16)).unwrap();
        assert_eq!(reader.read(&mut []).unwrap(), 0);
    }
}
