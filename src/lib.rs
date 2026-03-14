use pyo3::prelude::*;
use pyo3::exceptions::{PyOSError, PyValueError};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use jwalk::WalkDir;
use globset::Glob as GlobPattern;

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

fn check_name_filter(name_lower: &str, names_lower: &Option<Vec<String>>) -> bool {
    match names_lower {
        Some(patterns) => patterns.iter().any(|p| name_lower.contains(p)),
        None => true,
    }
}

fn check_ext_filter(name_lower: &str, exts: &Option<Vec<String>>) -> bool {
    match exts {
        Some(exts) => exts.iter().any(|e| name_lower.ends_with(e)),
        None => true,
    }
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

// ─── Functions ──────────────────────────────────────────────

/// Recursively walk a directory in parallel, returning all entries.
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
///
/// Returns:
///     List of FileEntry objects (both files and directories).
#[pyfunction]
#[pyo3(signature = (directory, extensions=None, skip_hidden=false, max_depth=None, names=None, min_size_mb=None, max_size_mb=None, modified_after=None, modified_before=None, created_after=None, created_before=None))]
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
) -> PyResult<Vec<FileEntry>> {
    validate_dir(&directory)?;

    py.detach(|| {
        let mut walker = WalkDir::new(&directory).skip_hidden(skip_hidden);
        if let Some(depth) = max_depth {
            walker = walker.max_depth(depth);
        }

        let exts: Option<Vec<String>> = extensions.map(|e| normalize_exts(&e));
        let names_lower: Option<Vec<String>> = names.map(|n| n.iter().map(|s| s.to_lowercase()).collect());
        let min_bytes = mb_to_bytes(min_size_mb);
        let max_bytes = mb_to_bytes(max_size_mb);
        let has_time_filters = modified_after.is_some() || modified_before.is_some()
            || created_after.is_some() || created_before.is_some();

        let entries: Vec<FileEntry> = walker
            .into_iter()
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                let is_file = entry.file_type().is_file();
                let is_dir = entry.file_type().is_dir();

                // File-only filters: extension, name
                if is_file {
                    let name_lower = name.to_lowercase();
                    if !check_ext_filter(&name_lower, &exts) {
                        return None;
                    }
                    if !check_name_filter(&name_lower, &names_lower) {
                        return None;
                    }
                }

                let metadata = path.metadata().ok();
                let size = if is_file {
                    metadata.as_ref().map(|m| m.len()).unwrap_or(0)
                } else {
                    0
                };

                // File-only filters: size, time
                if is_file {
                    if !check_size_filters(size, min_bytes, max_bytes) {
                        return None;
                    }
                    if has_time_filters {
                        if let Some(ref meta) = metadata {
                            if !check_time_filters(meta, modified_after, modified_before, created_after, created_before) {
                                return None;
                            }
                        }
                    }
                }

                let modified = metadata.as_ref().and_then(|m| m.modified().ok()).and_then(systemtime_to_epoch);
                let created = metadata.as_ref().and_then(|m| m.created().ok()).and_then(systemtime_to_epoch);

                Some(make_entry(&path, name, is_file, is_dir, size, modified, created))
            })
            .collect();

        Ok(entries)
    })
}

/// List contents of a single directory (non-recursive).
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
///     List of FileEntry objects in the directory.
#[pyfunction]
#[pyo3(signature = (directory, extensions=None, names=None, min_size_mb=None, max_size_mb=None, skip_hidden=false, modified_after=None, modified_before=None, created_after=None, created_before=None))]
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

    py.detach(|| {
        let exts = extensions.map(|e| normalize_exts(&e));
        let names_lower: Option<Vec<String>> = names.map(|n| n.iter().map(|s| s.to_lowercase()).collect());
        let min_bytes = mb_to_bytes(min_size_mb);
        let max_bytes = mb_to_bytes(max_size_mb);
        let has_time_filters = modified_after.is_some() || modified_before.is_some()
            || created_after.is_some() || created_before.is_some();

        let mut entries = Vec::new();
        let dir = std::fs::read_dir(&directory)
            .map_err(|e| PyOSError::new_err(format!("Cannot read directory: {}", e)))?;

        for item in dir {
            if let Ok(item) = item {
                let path = item.path();
                let name = item.file_name().to_string_lossy().to_string();

                // Skip hidden
                if skip_hidden && name.starts_with('.') {
                    continue;
                }

                let metadata = item.metadata().ok();
                let is_file = metadata.as_ref().map(|m| m.is_file()).unwrap_or(false);
                let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                let size = if is_file {
                    metadata.as_ref().map(|m| m.len()).unwrap_or(0)
                } else {
                    0
                };

                // File-only filters
                if is_file {
                    let name_lower = name.to_lowercase();
                    if !check_ext_filter(&name_lower, &exts) {
                        continue;
                    }
                    if !check_name_filter(&name_lower, &names_lower) {
                        continue;
                    }
                    if !check_size_filters(size, min_bytes, max_bytes) {
                        continue;
                    }
                    if has_time_filters {
                        if let Some(ref meta) = metadata {
                            if !check_time_filters(meta, modified_after, modified_before, created_after, created_before) {
                                continue;
                            }
                        }
                    }
                }

                let modified = metadata.as_ref().and_then(|m| m.modified().ok()).and_then(systemtime_to_epoch);
                let created = metadata.as_ref().and_then(|m| m.created().ok()).and_then(systemtime_to_epoch);
                entries.push(make_entry(&path, name, is_file, is_dir, size, modified, created));
            }
        }
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
///
/// Returns:
///     List of matching FileEntry objects (files only).
///
/// Example:
///     find("/data", names=["report", "summary"], extensions=[".pdf", ".docx"])
#[pyfunction]
#[pyo3(signature = (directory, names=None, extensions=None, min_size_mb=None, max_size_mb=None, skip_hidden=false, max_depth=None, modified_after=None, modified_before=None, created_after=None, created_before=None))]
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
) -> PyResult<Vec<FileEntry>> {
    validate_dir(&directory)?;

    if names.is_none() && extensions.is_none()
        && min_size_mb.is_none() && max_size_mb.is_none()
        && modified_after.is_none() && modified_before.is_none()
        && created_after.is_none() && created_before.is_none()
    {
        return Err(PyValueError::new_err(
            "Must provide at least `names`, `extensions`, a size filter, or a time filter"
        ));
    }

    py.detach(|| {
        let mut walker = WalkDir::new(&directory).skip_hidden(skip_hidden);
        if let Some(depth) = max_depth {
            walker = walker.max_depth(depth);
        }

        let names_lower: Option<Vec<String>> =
            names.map(|n| n.iter().map(|s| s.to_lowercase()).collect());
        let exts_lower: Option<Vec<String>> =
            extensions.map(|e| normalize_exts(&e));
        let min_bytes = mb_to_bytes(min_size_mb);
        let max_bytes = mb_to_bytes(max_size_mb);

        let entries: Vec<FileEntry> = walker
            .into_iter()
            .filter_map(|entry| {
                let entry = entry.ok()?;
                if !entry.file_type().is_file() {
                    return None;
                }

                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                let name_lower = name.to_lowercase();

                // Name substring match (any substring matches -> include)
                if !check_name_filter(&name_lower, &names_lower) {
                    return None;
                }

                // Extension match
                if !check_ext_filter(&name_lower, &exts_lower) {
                    return None;
                }

                // Size and time filters (single metadata call)
                let metadata = path.metadata().ok();
                let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
                if !check_size_filters(size, min_bytes, max_bytes) {
                    return None;
                }

                // Time filters
                if let Some(ref meta) = metadata {
                    if !check_time_filters(meta, modified_after, modified_before, created_after, created_before) {
                        return None;
                    }
                }

                let modified = metadata.as_ref().and_then(|m| m.modified().ok()).and_then(systemtime_to_epoch);
                let created = metadata.as_ref().and_then(|m| m.created().ok()).and_then(systemtime_to_epoch);

                Some(make_entry(&path, name, true, false, size, modified, created))
            })
            .collect();

        Ok(entries)
    })
}

/// Build a file index grouped by filename stem.
///
/// Returns a dict mapping lowercase filename stems to dicts of {extension: full_path}.
/// Useful for finding related files with different extensions.
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
///
/// Returns:
///     Dict like {"main": {".py": "/src/main.py", ".pyc": "/src/main.pyc"}}
#[pyfunction]
#[pyo3(signature = (directory, extensions, skip_hidden=false, max_depth=None, names=None, min_size_mb=None, max_size_mb=None, modified_after=None, modified_before=None, created_after=None, created_before=None))]
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
) -> PyResult<HashMap<String, HashMap<String, String>>> {
    validate_dir(&directory)?;

    py.detach(|| {
        let exts = normalize_exts(&extensions);
        let names_lower: Option<Vec<String>> = names.map(|n| n.iter().map(|s| s.to_lowercase()).collect());
        let min_bytes = mb_to_bytes(min_size_mb);
        let max_bytes = mb_to_bytes(max_size_mb);
        let has_meta_filters = min_bytes.is_some() || max_bytes.is_some()
            || modified_after.is_some() || modified_before.is_some()
            || created_after.is_some() || created_before.is_some();

        let mut walker = WalkDir::new(&directory).skip_hidden(skip_hidden);
        if let Some(depth) = max_depth {
            walker = walker.max_depth(depth);
        }

        let mut file_index: HashMap<String, HashMap<String, String>> = HashMap::new();

        for entry in walker {
            if let Ok(entry) = entry {
                if entry.file_type().is_file() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let name_lower = name.to_lowercase();

                    // Name filter
                    if !check_name_filter(&name_lower, &names_lower) {
                        continue;
                    }

                    // Size and time filters (require metadata)
                    if has_meta_filters {
                        if let Ok(metadata) = entry.path().metadata() {
                            let size = metadata.len();
                            if !check_size_filters(size, min_bytes, max_bytes) {
                                continue;
                            }
                            if !check_time_filters(&metadata, modified_after, modified_before, created_after, created_before) {
                                continue;
                            }
                        }
                    }

                    for ext in &exts {
                        if name_lower.ends_with(ext) {
                            let key = name_lower.strip_suffix(ext).unwrap_or(&name_lower).to_string();
                            let full_path = entry.path().to_string_lossy().to_string();
                            file_index
                                .entry(key)
                                .or_default()
                                .insert(ext.clone(), full_path);
                        }
                    }
                }
            }
        }

        Ok(file_index)
    })
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
///
/// Returns:
///     List of full paths matching the pattern.
#[pyfunction]
#[pyo3(signature = (directory, pattern, skip_hidden=false, max_depth=None, min_size_mb=None, max_size_mb=None, modified_after=None, modified_before=None, created_after=None, created_before=None))]
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
) -> PyResult<Vec<String>> {
    validate_dir(&directory)?;

    py.detach(|| {
        let matcher = GlobPattern::new(&pattern)
            .map_err(|e| PyValueError::new_err(format!("Invalid glob pattern: {}", e)))?
            .compile_matcher();

        let base = Path::new(&directory);
        let min_bytes = mb_to_bytes(min_size_mb);
        let max_bytes = mb_to_bytes(max_size_mb);
        let has_meta_filters = min_bytes.is_some() || max_bytes.is_some()
            || modified_after.is_some() || modified_before.is_some()
            || created_after.is_some() || created_before.is_some();

        let mut walker = WalkDir::new(&directory).skip_hidden(skip_hidden);
        if let Some(depth) = max_depth {
            walker = walker.max_depth(depth);
        }

        let paths: Vec<String> = walker
            .into_iter()
            .filter_map(|entry| {
                let entry = entry.ok()?;
                if !entry.file_type().is_file() {
                    return None;
                }
                let path = entry.path();
                let relative = path.strip_prefix(base).ok()?;
                // Normalize separators for cross-platform glob matching
                let rel_str = relative.to_string_lossy().replace('\\', "/");
                if !matcher.is_match(&rel_str) {
                    return None;
                }

                // Size and time filters
                if has_meta_filters {
                    let metadata = path.metadata().ok()?;
                    let size = metadata.len();
                    if !check_size_filters(size, min_bytes, max_bytes) {
                        return None;
                    }
                    if !check_time_filters(&metadata, modified_after, modified_before, created_after, created_before) {
                        return None;
                    }
                }

                Some(path.to_string_lossy().to_string())
            })
            .collect();

        Ok(paths)
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
///
/// Returns:
///     DiskUsage object with .entries, .total_size, .total_files, .total_size_mb, .total_size_gb.
#[pyfunction]
#[pyo3(signature = (directory, depth=1, top=20, skip_hidden=false, extensions=None, names=None, min_size_mb=None, max_size_mb=None, modified_after=None, modified_before=None, created_after=None, created_before=None))]
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
) -> PyResult<DiskUsage> {
    validate_dir(&directory)?;

    py.detach(|| {
        let base = Path::new(&directory);
        let exts = extensions.map(|e| normalize_exts(&e));
        let names_lower: Option<Vec<String>> = names.map(|n| n.iter().map(|s| s.to_lowercase()).collect());
        let min_bytes = mb_to_bytes(min_size_mb);
        let max_bytes = mb_to_bytes(max_size_mb);
        let has_time_filters = modified_after.is_some() || modified_before.is_some()
            || created_after.is_some() || created_before.is_some();

        let mut folder_sizes: HashMap<String, (u64, usize)> = HashMap::new();
        let mut total_size: u64 = 0;
        let mut total_files: usize = 0;

        for entry in WalkDir::new(&directory).skip_hidden(skip_hidden) {
            if let Ok(entry) = entry {
                if entry.file_type().is_file() {
                    let path = entry.path();
                    let name = entry.file_name().to_string_lossy().to_string();
                    let name_lower = name.to_lowercase();

                    // Extension filter
                    if !check_ext_filter(&name_lower, &exts) {
                        continue;
                    }

                    // Name filter
                    if !check_name_filter(&name_lower, &names_lower) {
                        continue;
                    }

                    let metadata = path.metadata().ok();
                    let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);

                    // Size filter
                    if !check_size_filters(size, min_bytes, max_bytes) {
                        continue;
                    }

                    // Time filter
                    if has_time_filters {
                        if let Some(ref meta) = metadata {
                            if !check_time_filters(meta, modified_after, modified_before, created_after, created_before) {
                                continue;
                            }
                        }
                    }

                    total_size += size;
                    total_files += 1;

                    if let Some(bucket) = get_depth_path(&path, base, depth) {
                        let key = bucket.to_string_lossy().to_string();
                        let counter = folder_sizes.entry(key).or_insert((0, 0));
                        counter.0 += size;
                        counter.1 += 1;
                    }
                }
            }
        }

        let mut entries: Vec<SizeEntry> = folder_sizes
            .into_iter()
            .map(|(path, (size, count))| SizeEntry {
                path,
                size,
                file_count: count,
            })
            .collect();

        entries.sort_by(|a, b| b.size.cmp(&a.size));
        entries.truncate(top);

        Ok(DiskUsage {
            total_size,
            total_files,
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
