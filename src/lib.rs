use pyo3::prelude::*;
use pyo3::exceptions::{PyOSError, PyValueError};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use dashmap::DashMap;
use globset::Glob as GlobPattern;
use ignore::{WalkBuilder, WalkState};

// ─── Data Types ─────────────────────────────────────────────

/// A file or directory entry returned by walk/find/list_dir.
#[pyclass(frozen, get_all)]
#[derive(Clone, Debug)]
pub struct FileEntry {
    pub path: String,
    pub name: String,
    pub is_file: bool,
    pub is_dir: bool,
    pub size: u64,
    pub extension: String,
    pub modified: Option<f64>,
    pub created: Option<f64>,
}

#[pymethods]
impl FileEntry {
    fn __repr__(&self) -> String {
        if self.is_file {
            format!("FileEntry('{}', size={})", self.name, self.size)
        } else {
            format!("FileEntry('{}', dir)", self.name)
        }
    }

    fn __str__(&self) -> &str {
        &self.path
    }
}

/// A disk usage entry for a path.
#[pyclass(frozen, get_all)]
#[derive(Clone, Debug)]
pub struct SizeEntry {
    pub path: String,
    pub size: u64,
    pub file_count: usize,
}

#[pymethods]
impl SizeEntry {
    fn __repr__(&self) -> String {
        format!("SizeEntry('{}', size={}, files={})", self.path, self.size, self.file_count)
    }

    #[getter]
    fn size_mb(&self) -> f64 {
        self.size as f64 / (1024.0 * 1024.0)
    }

    #[getter]
    fn size_gb(&self) -> f64 {
        self.size as f64 / (1024.0 * 1024.0 * 1024.0)
    }
}

/// Result of disk_usage analysis.
#[pyclass(frozen)]
pub struct DiskUsage {
    #[pyo3(get)]
    pub total_size: u64,
    #[pyo3(get)]
    pub total_files: usize,
    entries_vec: Vec<SizeEntry>,
}

#[pymethods]
impl DiskUsage {
    #[getter]
    fn entries(&self) -> Vec<SizeEntry> {
        self.entries_vec.clone()
    }

    #[getter]
    fn total_size_mb(&self) -> f64 {
        self.total_size as f64 / (1024.0 * 1024.0)
    }

    #[getter]
    fn total_size_gb(&self) -> f64 {
        self.total_size as f64 / (1024.0 * 1024.0 * 1024.0)
    }

    fn __repr__(&self) -> String {
        format!(
            "DiskUsage(total_size={}, total_files={}, top_entries={})",
            self.total_size, self.total_files, self.entries_vec.len()
        )
    }
}

// ─── Helpers ────────────────────────────────────────────────

fn systemtime_to_epoch(t: SystemTime) -> Option<f64> {
    t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs_f64())
}

fn make_entry(path: &Path, name: String, is_file: bool, is_dir: bool, size: u64, modified: Option<f64>, created: Option<f64>) -> FileEntry {
    let extension = path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_string();
    FileEntry {
        path: path.to_string_lossy().to_string(),
        name,
        is_file,
        is_dir,
        size,
        extension,
        modified,
        created,
    }
}

fn check_time_filters(
    metadata: &std::fs::Metadata,
    modified_after: Option<f64>,
    modified_before: Option<f64>,
    created_after: Option<f64>,
    created_before: Option<f64>,
) -> bool {
    if let Some(after) = modified_after {
        let mtime = metadata.modified().ok().and_then(systemtime_to_epoch).unwrap_or(0.0);
        if mtime < after { return false; }
    }
    if let Some(before) = modified_before {
        let mtime = metadata.modified().ok().and_then(systemtime_to_epoch).unwrap_or(f64::MAX);
        if mtime > before { return false; }
    }
    if let Some(after) = created_after {
        let ctime = metadata.created().ok().and_then(systemtime_to_epoch).unwrap_or(0.0);
        if ctime < after { return false; }
    }
    if let Some(before) = created_before {
        let ctime = metadata.created().ok().and_then(systemtime_to_epoch).unwrap_or(f64::MAX);
        if ctime > before { return false; }
    }
    true
}

fn check_size_filters(size: u64, min_bytes: Option<u64>, max_bytes: Option<u64>) -> bool {
    if let Some(min) = min_bytes {
        if size < min { return false; }
    }
    if let Some(max) = max_bytes {
        if size > max { return false; }
    }
    true
}

fn mb_to_bytes(mb: Option<f64>) -> Option<u64> {
    mb.map(|v| (v * 1024.0 * 1024.0) as u64)
}

fn validate_dir(directory: &str) -> PyResult<()> {
    let path = Path::new(directory);
    if !path.exists() {
        return Err(PyOSError::new_err(format!("Path not found: {}", directory)));
    }
    if !path.is_dir() {
        return Err(PyValueError::new_err(format!("Not a directory: {}", directory)));
    }
    Ok(())
}

/// Normalize extensions: ensure they start with '.' and are lowercase.
fn normalize_exts(extensions: &[String]) -> Vec<String> {
    extensions.iter().map(|s| {
        let s = s.to_lowercase();
        if s.starts_with('.') { s } else { format!(".{}", s) }
    }).collect()
}

fn get_depth_path(path: &Path, base: &Path, target_depth: usize) -> Option<PathBuf> {
    let relative = path.strip_prefix(base).ok()?;
    let components: Vec<_> = relative.components().collect();
    if components.is_empty() {
        return None;
    }
    let depth = components.len().min(target_depth);
    let mut result = base.to_path_buf();
    for component in &components[..depth] {
        result.push(component);
    }
    Some(result)
}

fn default_threads(threads: Option<usize>) -> usize {
    threads
        .filter(|&t| t > 0)
        .unwrap_or_else(|| thread::available_parallelism().map(|n| n.get()).unwrap_or(4))
}

/// Hidden check for list_dir: dot prefix everywhere, plus the hidden
/// attribute on Windows (matching what the parallel walker does).
fn is_hidden(name: &str, metadata: Option<&std::fs::Metadata>) -> bool {
    if name.starts_with('.') {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        if let Some(m) = metadata {
            if m.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0 {
                return true;
            }
        }
    }
    #[cfg(not(windows))]
    let _ = metadata;
    false
}

/// Shared file filters. Name checks allocate only when a name-based
/// filter is actually set; metadata checks exclude files whose metadata
/// cannot be read whenever a size or time filter is active.
#[derive(Clone)]
struct Filters {
    exts: Option<Vec<String>>,
    names: Option<Vec<String>>,
    min_bytes: Option<u64>,
    max_bytes: Option<u64>,
    modified_after: Option<f64>,
    modified_before: Option<f64>,
    created_after: Option<f64>,
    created_before: Option<f64>,
}

impl Filters {
    #[allow(clippy::too_many_arguments)]
    fn new(
        extensions: Option<Vec<String>>,
        names: Option<Vec<String>>,
        min_size_mb: Option<f64>,
        max_size_mb: Option<f64>,
        modified_after: Option<f64>,
        modified_before: Option<f64>,
        created_after: Option<f64>,
        created_before: Option<f64>,
    ) -> Self {
        Filters {
            exts: extensions.map(|e| normalize_exts(&e)),
            names: names.map(|n| n.iter().map(|s| s.to_lowercase()).collect()),
            min_bytes: mb_to_bytes(min_size_mb),
            max_bytes: mb_to_bytes(max_size_mb),
            modified_after,
            modified_before,
            created_after,
            created_before,
        }
    }

    fn has_name_filters(&self) -> bool {
        self.exts.is_some() || self.names.is_some()
    }

    fn has_time_filters(&self) -> bool {
        self.modified_after.is_some() || self.modified_before.is_some()
            || self.created_after.is_some() || self.created_before.is_some()
    }

    fn has_meta_filters(&self) -> bool {
        self.min_bytes.is_some() || self.max_bytes.is_some() || self.has_time_filters()
    }

    fn has_any(&self) -> bool {
        self.has_name_filters() || self.has_meta_filters()
    }

    fn matches_name(&self, name: &str) -> bool {
        if !self.has_name_filters() {
            return true;
        }
        let name_lower = name.to_lowercase();
        if let Some(exts) = &self.exts {
            if !exts.iter().any(|e| name_lower.ends_with(e.as_str())) {
                return false;
            }
        }
        if let Some(patterns) = &self.names {
            if !patterns.iter().any(|p| name_lower.contains(p.as_str())) {
                return false;
            }
        }
        true
    }

    fn matches_metadata(&self, metadata: Option<&std::fs::Metadata>) -> bool {
        if !self.has_meta_filters() {
            return true;
        }
        let meta = match metadata {
            Some(m) => m,
            None => return false,
        };
        if !check_size_filters(meta.len(), self.min_bytes, self.max_bytes) {
            return false;
        }
        check_time_filters(meta, self.modified_after, self.modified_before, self.created_after, self.created_before)
    }
}

/// Build the parallel walker used by every recursive function. All
/// gitignore semantics are disabled: pyofiles always sees every file.
fn build_walker(directory: &Path, skip_hidden: bool, max_depth: Option<usize>, threads: Option<usize>) -> ignore::WalkParallel {
    WalkBuilder::new(directory)
        .follow_links(false)
        .hidden(skip_hidden)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .ignore(false)
        .parents(false)
        .max_depth(max_depth)
        .threads(default_threads(threads))
        .build_parallel()
}

// ─── Functions ──────────────────────────────────────────────

/// Recursively walk a directory in parallel, returning all entries.
///
/// When any filter is given, only matching files are returned;
/// directories are omitted. Without filters, files and directories
/// are both included.
///
/// Args:
///     directory: Path to walk.
///     extensions: Optional list of extensions to filter by (e.g. [".py", ".rs"]).
///     skip_hidden: Skip hidden files and directories.
///     max_depth: Maximum recursion depth.
///     names: Optional list of name substrings to match (case-insensitive, OR logic).
///     min_size_mb: Minimum file size in megabytes.
///     max_size_mb: Maximum file size in megabytes.
///     modified_after: Only include files modified after this unix timestamp.
///     modified_before: Only include files modified before this unix timestamp.
///     created_after: Only include files created after this unix timestamp.
///     created_before: Only include files created before this unix timestamp.
///     threads: Number of walker threads (default: number of CPUs).
///
/// Returns:
///     List of FileEntry objects.
#[pyfunction]
#[pyo3(signature = (directory, extensions=None, skip_hidden=false, max_depth=None, names=None, min_size_mb=None, max_size_mb=None, modified_after=None, modified_before=None, created_after=None, created_before=None, threads=None))]
#[allow(clippy::too_many_arguments)]
fn walk(
    py: Python<'_>,
    directory: String,
    extensions: Option<Vec<String>>,
    skip_hidden: bool,
    max_depth: Option<usize>,
    names: Option<Vec<String>>,
    min_size_mb: Option<f64>,
    max_size_mb: Option<f64>,
    modified_after: Option<f64>,
    modified_before: Option<f64>,
    created_after: Option<f64>,
    created_before: Option<f64>,
    threads: Option<usize>,
) -> PyResult<Vec<FileEntry>> {
    validate_dir(&directory)?;
    let filters = Filters::new(extensions, names, min_size_mb, max_size_mb, modified_after, modified_before, created_after, created_before);

    py.detach(|| {
        let include_non_files = !filters.has_any();
        let (tx, rx) = mpsc::channel::<FileEntry>();
        let walker = build_walker(Path::new(&directory), skip_hidden, max_depth, threads);

        walker.run(|| {
            let tx = tx.clone();
            let filters = filters.clone();
            Box::new(move |result| {
                let entry = match result {
                    Ok(e) => e,
                    Err(_) => return WalkState::Continue,
                };
                let file_type = entry.file_type();
                let is_file = file_type.map(|ft| ft.is_file()).unwrap_or(false);
                let is_dir = file_type.map(|ft| ft.is_dir()).unwrap_or(false);
                if !is_file && !include_non_files {
                    return WalkState::Continue;
                }

                let name = entry.file_name().to_string_lossy().to_string();
                if is_file && !filters.matches_name(&name) {
                    return WalkState::Continue;
                }

                let metadata = entry.metadata().ok();
                if is_file && !filters.matches_metadata(metadata.as_ref()) {
                    return WalkState::Continue;
                }

                let size = if is_file {
                    metadata.as_ref().map(|m| m.len()).unwrap_or(0)
                } else {
                    0
                };
                let modified = metadata.as_ref().and_then(|m| m.modified().ok()).and_then(systemtime_to_epoch);
                let created = metadata.as_ref().and_then(|m| m.created().ok()).and_then(systemtime_to_epoch);

                let _ = tx.send(make_entry(entry.path(), name, is_file, is_dir, size, modified, created));
                WalkState::Continue
            })
        });

        drop(tx);
        Ok(rx.into_iter().collect())
    })
}

/// List contents of a single directory (non-recursive), sorted by name.
///
/// Args:
///     directory: Path to list.
///     extensions: Optional list of extensions to filter by.
///     names: Optional list of name substrings to match (case-insensitive, OR logic).
///     min_size_mb: Minimum file size in megabytes.
///     max_size_mb: Maximum file size in megabytes.
///     skip_hidden: Skip hidden files and directories.
///     modified_after: Only include files modified after this unix timestamp.
///     modified_before: Only include files modified before this unix timestamp.
///     created_after: Only include files created after this unix timestamp.
///     created_before: Only include files created before this unix timestamp.
///
/// Returns:
///     List of FileEntry objects in the directory, sorted by name.
#[pyfunction]
#[pyo3(signature = (directory, extensions=None, names=None, min_size_mb=None, max_size_mb=None, skip_hidden=false, modified_after=None, modified_before=None, created_after=None, created_before=None))]
#[allow(clippy::too_many_arguments)]
fn list_dir(
    py: Python<'_>,
    directory: String,
    extensions: Option<Vec<String>>,
    names: Option<Vec<String>>,
    min_size_mb: Option<f64>,
    max_size_mb: Option<f64>,
    skip_hidden: bool,
    modified_after: Option<f64>,
    modified_before: Option<f64>,
    created_after: Option<f64>,
    created_before: Option<f64>,
) -> PyResult<Vec<FileEntry>> {
    validate_dir(&directory)?;
    let filters = Filters::new(extensions, names, min_size_mb, max_size_mb, modified_after, modified_before, created_after, created_before);

    py.detach(|| {
        let mut entries = Vec::new();
        let dir = std::fs::read_dir(&directory)
            .map_err(|e| PyOSError::new_err(format!("Cannot read directory: {}", e)))?;

        for item in dir.flatten() {
            let path = item.path();
            let name = item.file_name().to_string_lossy().to_string();
            let metadata = item.metadata().ok();

            if skip_hidden && is_hidden(&name, metadata.as_ref()) {
                continue;
            }

            let is_file = metadata.as_ref().map(|m| m.is_file()).unwrap_or(false);
            let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let size = if is_file {
                metadata.as_ref().map(|m| m.len()).unwrap_or(0)
            } else {
                0
            };

            if is_file {
                if !filters.matches_name(&name) {
                    continue;
                }
                if !filters.matches_metadata(metadata.as_ref()) {
                    continue;
                }
            }

            let modified = metadata.as_ref().and_then(|m| m.modified().ok()).and_then(systemtime_to_epoch);
            let created = metadata.as_ref().and_then(|m| m.created().ok()).and_then(systemtime_to_epoch);
            entries.push(make_entry(&path, name, is_file, is_dir, size, modified, created));
        }

        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    })
}

/// Find files matching name substrings, extensions, and/or size filters.
///
/// The primary search function. `names` accepts a list of substrings --
/// a file matches if its name contains ANY of the given substrings (case-insensitive).
///
/// Args:
///     directory: Root directory to search.
///     names: List of substrings to match against filenames (case-insensitive, OR logic).
///     extensions: Optional list of extensions to filter by.
///     min_size_mb: Minimum file size in megabytes.
///     max_size_mb: Maximum file size in megabytes.
///     skip_hidden: Skip hidden files and directories.
///     max_depth: Maximum recursion depth.
///     modified_after: Only include files modified after this unix timestamp.
///     modified_before: Only include files modified before this unix timestamp.
///     created_after: Only include files created after this unix timestamp.
///     created_before: Only include files created before this unix timestamp.
///     limit: Stop searching once this many matches are found. Which
///         matches are returned when the limit truncates is unspecified.
///     threads: Number of walker threads (default: number of CPUs).
///
/// Returns:
///     List of matching FileEntry objects (files only).
///
/// Example:
///     find("/data", names=["report", "summary"], extensions=[".pdf", ".docx"])
#[pyfunction]
#[pyo3(signature = (directory, names=None, extensions=None, min_size_mb=None, max_size_mb=None, skip_hidden=false, max_depth=None, modified_after=None, modified_before=None, created_after=None, created_before=None, limit=None, threads=None))]
#[allow(clippy::too_many_arguments)]
fn find(
    py: Python<'_>,
    directory: String,
    names: Option<Vec<String>>,
    extensions: Option<Vec<String>>,
    min_size_mb: Option<f64>,
    max_size_mb: Option<f64>,
    skip_hidden: bool,
    max_depth: Option<usize>,
    modified_after: Option<f64>,
    modified_before: Option<f64>,
    created_after: Option<f64>,
    created_before: Option<f64>,
    limit: Option<usize>,
    threads: Option<usize>,
) -> PyResult<Vec<FileEntry>> {
    validate_dir(&directory)?;
    let filters = Filters::new(extensions, names, min_size_mb, max_size_mb, modified_after, modified_before, created_after, created_before);

    if !filters.has_any() {
        return Err(PyValueError::new_err(
            "Must provide at least `names`, `extensions`, a size filter, or a time filter"
        ));
    }
    if limit == Some(0) {
        return Ok(Vec::new());
    }

    py.detach(|| {
        let (tx, rx) = mpsc::channel::<FileEntry>();
        let walker = build_walker(Path::new(&directory), skip_hidden, max_depth, threads);
        let sent = Arc::new(AtomicUsize::new(0));

        walker.run(|| {
            let tx = tx.clone();
            let filters = filters.clone();
            let sent = Arc::clone(&sent);
            Box::new(move |result| {
                if let Some(lim) = limit {
                    if sent.load(Ordering::Relaxed) >= lim {
                        return WalkState::Quit;
                    }
                }

                let entry = match result {
                    Ok(e) => e,
                    Err(_) => return WalkState::Continue,
                };
                if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                    return WalkState::Continue;
                }

                let name = entry.file_name().to_string_lossy().to_string();
                if !filters.matches_name(&name) {
                    return WalkState::Continue;
                }

                let metadata = entry.metadata().ok();
                if !filters.matches_metadata(metadata.as_ref()) {
                    return WalkState::Continue;
                }

                let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
                let modified = metadata.as_ref().and_then(|m| m.modified().ok()).and_then(systemtime_to_epoch);
                let created = metadata.as_ref().and_then(|m| m.created().ok()).and_then(systemtime_to_epoch);

                let _ = tx.send(make_entry(entry.path(), name, true, false, size, modified, created));

                if let Some(lim) = limit {
                    if sent.fetch_add(1, Ordering::Relaxed) + 1 >= lim {
                        return WalkState::Quit;
                    }
                }
                WalkState::Continue
            })
        });

        drop(tx);
        let mut entries: Vec<FileEntry> = rx.into_iter().collect();
        if let Some(lim) = limit {
            entries.truncate(lim);
        }
        Ok(entries)
    })
}

/// Build a file index grouped by filename stem.
///
/// Returns a dict mapping lowercase filename stems to dicts of {extension: full_path}.
/// Useful for finding related files with different extensions.
///
/// If two files share the same stem and extension (e.g. in different
/// subdirectories), the lexicographically smallest full path is kept,
/// so results are deterministic.
///
/// Args:
///     directory: Root directory to index.
///     extensions: Extensions to index (e.g. [".py", ".pyi", ".pyc"]).
///     skip_hidden: Skip hidden files.
///     max_depth: Maximum recursion depth.
///     names: Optional list of name substrings to match (case-insensitive, OR logic).
///     min_size_mb: Minimum file size in megabytes.
///     max_size_mb: Maximum file size in megabytes.
///     modified_after: Only include files modified after this unix timestamp.
///     modified_before: Only include files modified before this unix timestamp.
///     created_after: Only include files created after this unix timestamp.
///     created_before: Only include files created before this unix timestamp.
///     threads: Number of walker threads (default: number of CPUs).
///
/// Returns:
///     Dict like {"main": {".py": "/src/main.py", ".pyc": "/src/main.pyc"}}
#[pyfunction]
#[pyo3(signature = (directory, extensions, skip_hidden=false, max_depth=None, names=None, min_size_mb=None, max_size_mb=None, modified_after=None, modified_before=None, created_after=None, created_before=None, threads=None))]
#[allow(clippy::too_many_arguments)]
fn index(
    py: Python<'_>,
    directory: String,
    extensions: Vec<String>,
    skip_hidden: bool,
    max_depth: Option<usize>,
    names: Option<Vec<String>>,
    min_size_mb: Option<f64>,
    max_size_mb: Option<f64>,
    modified_after: Option<f64>,
    modified_before: Option<f64>,
    created_after: Option<f64>,
    created_before: Option<f64>,
    threads: Option<usize>,
) -> PyResult<HashMap<String, HashMap<String, String>>> {
    validate_dir(&directory)?;
    let exts = normalize_exts(&extensions);
    let filters = Filters::new(None, names, min_size_mb, max_size_mb, modified_after, modified_before, created_after, created_before);

    py.detach(|| {
        let (tx, rx) = mpsc::channel::<(String, String, String)>();
        let walker = build_walker(Path::new(&directory), skip_hidden, max_depth, threads);

        walker.run(|| {
            let tx = tx.clone();
            let exts = exts.clone();
            let filters = filters.clone();
            Box::new(move |result| {
                let entry = match result {
                    Ok(e) => e,
                    Err(_) => return WalkState::Continue,
                };
                if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                    return WalkState::Continue;
                }

                let name = entry.file_name().to_string_lossy();
                let name_lower = name.to_lowercase();

                if let Some(patterns) = &filters.names {
                    if !patterns.iter().any(|p| name_lower.contains(p.as_str())) {
                        return WalkState::Continue;
                    }
                }
                if filters.has_meta_filters() && !filters.matches_metadata(entry.metadata().ok().as_ref()) {
                    return WalkState::Continue;
                }

                for ext in &exts {
                    if name_lower.ends_with(ext.as_str()) {
                        let stem = name_lower.strip_suffix(ext.as_str()).unwrap_or(&name_lower).to_string();
                        let full_path = entry.path().to_string_lossy().to_string();
                        let _ = tx.send((stem, ext.clone(), full_path));
                    }
                }
                WalkState::Continue
            })
        });

        drop(tx);
        let mut file_index: HashMap<String, HashMap<String, String>> = HashMap::new();
        for (stem, ext, path) in rx {
            use std::collections::hash_map::Entry;
            match file_index.entry(stem).or_default().entry(ext) {
                Entry::Vacant(slot) => {
                    slot.insert(path);
                }
                Entry::Occupied(mut slot) => {
                    if path < *slot.get() {
                        slot.insert(path);
                    }
                }
            }
        }
        Ok(file_index)
    })
}

const GLOB_META_CHARS: &[char] = &['*', '?', '[', '{'];

/// Leading literal directory components of a glob pattern, used to start
/// the walk as deep as possible instead of scanning the whole tree.
fn literal_prefix_components(pattern: &str) -> Vec<&str> {
    let components: Vec<&str> = pattern.split('/').collect();
    let mut n = 0;
    for c in &components {
        if c.is_empty() || *c == "." || *c == ".." || c.contains(GLOB_META_CHARS) {
            break;
        }
        n += 1;
    }
    // A fully literal pattern still needs its last component matched as
    // the file name, so never consume the final component as a directory.
    if n == components.len() && n > 0 {
        n -= 1;
    }
    components[..n].to_vec()
}

/// Match files against a glob pattern.
///
/// Args:
///     directory: Root directory to search.
///     pattern: Glob pattern (e.g. "**/*.py", "src/*.rs", "*.{js,ts}").
///     skip_hidden: Skip hidden files.
///     max_depth: Maximum recursion depth.
///     min_size_mb: Minimum file size in megabytes.
///     max_size_mb: Maximum file size in megabytes.
///     modified_after: Only include files modified after this unix timestamp.
///     modified_before: Only include files modified before this unix timestamp.
///     created_after: Only include files created after this unix timestamp.
///     created_before: Only include files created before this unix timestamp.
///     threads: Number of walker threads (default: number of CPUs).
///
/// Returns:
///     List of full paths matching the pattern.
#[pyfunction]
#[pyo3(signature = (directory, pattern, skip_hidden=false, max_depth=None, min_size_mb=None, max_size_mb=None, modified_after=None, modified_before=None, created_after=None, created_before=None, threads=None))]
#[allow(clippy::too_many_arguments)]
fn glob(
    py: Python<'_>,
    directory: String,
    pattern: String,
    skip_hidden: bool,
    max_depth: Option<usize>,
    min_size_mb: Option<f64>,
    max_size_mb: Option<f64>,
    modified_after: Option<f64>,
    modified_before: Option<f64>,
    created_after: Option<f64>,
    created_before: Option<f64>,
    threads: Option<usize>,
) -> PyResult<Vec<String>> {
    validate_dir(&directory)?;
    let filters = Filters::new(None, None, min_size_mb, max_size_mb, modified_after, modified_before, created_after, created_before);

    py.detach(|| {
        let matcher = GlobPattern::new(&pattern)
            .map_err(|e| PyValueError::new_err(format!("Invalid glob pattern: {}", e)))?
            .compile_matcher();

        let base = Arc::new(PathBuf::from(&directory));

        // Start the walk at the deepest literal directory in the pattern.
        let prefix = literal_prefix_components(&pattern);
        let prefix_depth = prefix.len();
        let mut start = base.as_ref().clone();
        for component in &prefix {
            start.push(component);
        }
        if prefix_depth > 0 && !start.is_dir() {
            return Ok(Vec::new());
        }
        let effective_depth = match max_depth {
            Some(d) if d < prefix_depth => return Ok(Vec::new()),
            Some(d) => Some(d - prefix_depth),
            None => None,
        };

        let (tx, rx) = mpsc::channel::<String>();
        let walker = build_walker(&start, skip_hidden, effective_depth, threads);

        walker.run(|| {
            let tx = tx.clone();
            let filters = filters.clone();
            let matcher = matcher.clone();
            let base = Arc::clone(&base);
            Box::new(move |result| {
                let entry = match result {
                    Ok(e) => e,
                    Err(_) => return WalkState::Continue,
                };
                if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                    return WalkState::Continue;
                }

                let path = entry.path();
                let relative = match path.strip_prefix(base.as_path()) {
                    Ok(r) => r,
                    Err(_) => return WalkState::Continue,
                };
                // Normalize separators for cross-platform glob matching
                let rel_str = relative.to_string_lossy().replace('\\', "/");
                if !matcher.is_match(&rel_str) {
                    return WalkState::Continue;
                }

                if filters.has_meta_filters() && !filters.matches_metadata(entry.metadata().ok().as_ref()) {
                    return WalkState::Continue;
                }

                let _ = tx.send(path.to_string_lossy().to_string());
                WalkState::Continue
            })
        });

        drop(tx);
        Ok(rx.into_iter().collect())
    })
}

/// Analyze disk space usage by directory.
///
/// Groups files into buckets at the specified depth and returns them sorted
/// by size (largest first).
///
/// Args:
///     directory: Path to analyze.
///     depth: Directory depth for grouping (default: 1).
///     top: Number of top entries to return (default: 20).
///     skip_hidden: Skip hidden files and directories.
///     extensions: Optional list of extensions to filter by.
///     names: Optional list of name substrings to match (case-insensitive, OR logic).
///     min_size_mb: Minimum file size in megabytes.
///     max_size_mb: Maximum file size in megabytes.
///     modified_after: Only include files modified after this unix timestamp.
///     modified_before: Only include files modified before this unix timestamp.
///     created_after: Only include files created after this unix timestamp.
///     created_before: Only include files created before this unix timestamp.
///     threads: Number of walker threads (default: number of CPUs).
///
/// Returns:
///     DiskUsage object with .entries, .total_size, .total_files, .total_size_mb, .total_size_gb.
#[pyfunction]
#[pyo3(signature = (directory, depth=1, top=20, skip_hidden=false, extensions=None, names=None, min_size_mb=None, max_size_mb=None, modified_after=None, modified_before=None, created_after=None, created_before=None, threads=None))]
#[allow(clippy::too_many_arguments)]
fn disk_usage(
    py: Python<'_>,
    directory: String,
    depth: usize,
    top: usize,
    skip_hidden: bool,
    extensions: Option<Vec<String>>,
    names: Option<Vec<String>>,
    min_size_mb: Option<f64>,
    max_size_mb: Option<f64>,
    modified_after: Option<f64>,
    modified_before: Option<f64>,
    created_after: Option<f64>,
    created_before: Option<f64>,
    threads: Option<usize>,
) -> PyResult<DiskUsage> {
    validate_dir(&directory)?;
    let filters = Filters::new(extensions, names, min_size_mb, max_size_mb, modified_after, modified_before, created_after, created_before);

    py.detach(|| {
        let base = Arc::new(PathBuf::from(&directory));
        let folder_sizes: Arc<DashMap<PathBuf, (AtomicU64, AtomicUsize)>> = Arc::new(DashMap::new());
        let total_size = Arc::new(AtomicU64::new(0));
        let total_files = Arc::new(AtomicUsize::new(0));

        let walker = build_walker(base.as_path(), skip_hidden, None, threads);

        walker.run(|| {
            let folder_sizes = Arc::clone(&folder_sizes);
            let total_size = Arc::clone(&total_size);
            let total_files = Arc::clone(&total_files);
            let base = Arc::clone(&base);
            let filters = filters.clone();

            Box::new(move |entry| {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => return WalkState::Continue,
                };

                let is_file = entry.file_type().map(|ft| ft.is_file()).unwrap_or(false);
                if !is_file {
                    return WalkState::Continue;
                }

                // Cow borrow: no allocation unless a name filter is set
                let name = entry.file_name().to_string_lossy();
                if !filters.matches_name(&name) {
                    return WalkState::Continue;
                }

                let metadata = entry.metadata().ok();
                if !filters.matches_metadata(metadata.as_ref()) {
                    return WalkState::Continue;
                }
                let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);

                total_size.fetch_add(size, Ordering::Relaxed);
                total_files.fetch_add(1, Ordering::Relaxed);

                if let Some(bucket) = get_depth_path(entry.path(), base.as_path(), depth) {
                    let counter = folder_sizes
                        .entry(bucket)
                        .or_insert_with(|| (AtomicU64::new(0), AtomicUsize::new(0)));
                    counter.value().0.fetch_add(size, Ordering::Relaxed);
                    counter.value().1.fetch_add(1, Ordering::Relaxed);
                }

                WalkState::Continue
            })
        });

        let mut entries: Vec<SizeEntry> = folder_sizes
            .iter()
            .map(|e| SizeEntry {
                path: e.key().to_string_lossy().to_string(),
                size: e.value().0.load(Ordering::Relaxed),
                file_count: e.value().1.load(Ordering::Relaxed),
            })
            .collect();

        entries.sort_by(|a, b| b.size.cmp(&a.size));
        entries.truncate(top);

        Ok(DiskUsage {
            total_size: total_size.load(Ordering::Relaxed),
            total_files: total_files.load(Ordering::Relaxed),
            entries_vec: entries,
        })
    })
}

// ─── Module ─────────────────────────────────────────────────

#[pymodule]
fn pyofiles(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<FileEntry>()?;
    m.add_class::<SizeEntry>()?;
    m.add_class::<DiskUsage>()?;
    m.add_function(wrap_pyfunction!(walk, m)?)?;
    m.add_function(wrap_pyfunction!(list_dir, m)?)?;
    m.add_function(wrap_pyfunction!(find, m)?)?;
    m.add_function(wrap_pyfunction!(index, m)?)?;
    m.add_function(wrap_pyfunction!(glob, m)?)?;
    m.add_function(wrap_pyfunction!(disk_usage, m)?)?;
    Ok(())
}
