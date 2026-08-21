<div align="center">
  <img src="https://raw.githubusercontent.com/lperezmo/pyofiles/master/icons/pyofiles_logo.svg" alt="pyofiles logo" width="400">
  <br><br>

  <a href="https://github.com/lperezmo/pyofiles/actions/workflows/ci.yml"><img src="https://github.com/lperezmo/pyofiles/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://pypi.org/project/pyofiles/"><img src="https://img.shields.io/pypi/v/pyofiles" alt="PyPI version"></a>
  <a href="https://www.python.org/"><img src="https://img.shields.io/badge/python-%E2%89%A53.9-blue" alt="Python ≥3.9"></a>
  <a href="https://github.com/lperezmo/pyofiles/blob/master/LICENSE"><img src="https://img.shields.io/github/license/lperezmo/pyofiles" alt="License"></a>
</div>

# pyofiles

Fast, Rust-powered file operations for Python. Drop-in replacements for `os.walk`, `os.listdir`, and `glob.glob` -- built on parallel directory walkers for maximum speed.

## Install

```bash
uv add pyofiles
```

or with pip:

```bash
pip install pyofiles
```

## CLI

pyofiles includes a command-line interface. Run directly with `uvx`:

```bash
uvx pyofiles find . --ext .py --modified-after 7d
```

Or install globally with `uv tool`:

```bash
uv tool install pyofiles
```

### Version

```bash
pyofiles --version
pyofiles -v
```

You can also check the version from Python:

```python
import pyofiles
print(pyofiles.__version__)
```

### Examples

```bash
# Walk a directory, showing only Python files
pyofiles walk ./src --ext .py --skip-hidden

# Long format: type, size, modified time, path
pyofiles walk ./src --ext .py -l

# Walk with name and size filters
pyofiles walk ./src --names main utils --min-size 0.001

# Find files by name substring
pyofiles find ./data --names report invoice --ext .pdf

# Find files modified in the last 7 days
pyofiles find ./project --ext .py --modified-after 7d

# Find files created before a specific date
pyofiles find ./logs --ext .log --created-before 2024-06-01

# Find large files modified recently
pyofiles find ./data --ext .csv --min-size 100 --modified-after 24h

# Find by size alone
pyofiles find . --min-size 500

# Stop after the first match (fast lookups in huge trees)
pyofiles find ./drawings --names P-1042 --limit 1

# Tune walker threads (more helps on network drives)
pyofiles find //server/share --ext .pdf --threads 16

# Glob pattern matching (with time/size filters)
pyofiles glob ./project "**/*.rs"
pyofiles glob ./project "**/*.py" --modified-after 7d --max-depth 3

# List directory contents (with filters)
pyofiles ls ./some/dir -l
pyofiles ls ./src --ext .py --skip-hidden

# Index files by stem (with time filters)
pyofiles index ./src --ext .py .pyi .pyc
pyofiles index ./src --ext .py --created-after 2024-01-01

# Disk usage analysis (with extension/time/name filters)
pyofiles du ./project --depth 2 --top 10
pyofiles du ./project --ext .py --modified-after 30d
pyofiles du ./project --names test --ext .py

# NTFS MFT fast path (Windows, elevated shell, local NTFS volume)
pyofiles du C:\ --mft --depth 2 --top 20
pyofiles find C:\Users --mft --ext .mp4 --min-size 500
pyofiles walk C:\projects --mft --ext .py

# JSON output (works with all commands, pipe to jq)
pyofiles find ./src --ext .py --json
pyofiles du . --json | jq '.entries[:5]'
```

### Time formats

Time arguments (`--modified-after`, `--modified-before`, `--created-after`, `--created-before`) accept:

| Format | Example | Meaning |
|--------|---------|---------|
| Relative duration (case-insensitive) | `7d`, `24H`, `1.5d` | ago from now |
| ISO date | `2024-03-15` | midnight on that date |
| ISO datetime | `2024-03-15T10:30:00` | specific moment |
| ISO datetime with offset | `2024-03-15T10:30:00+02:00`, `2024-03-15T10:30:00Z` | specific moment in the given timezone |
| Unix timestamp | `1709251200` | raw epoch seconds |

### Filter availability

All filters are available across commands where they make sense:

| Filter | walk | find | ls | glob | index | du |
|--------|------|------|----|------|-------|----|
| `--ext` | yes | yes | yes | — | yes (required) | yes |
| `--names` | yes | yes | yes | — | yes | yes |
| `--min/max-size` | yes | yes | yes | yes | yes | yes |
| `--skip-hidden` | yes | yes | yes | yes | yes | yes |
| `--max-depth` | yes | yes | — | yes | yes | — |
| time filters | yes | yes | yes | yes | yes | yes |
| `--limit` | — | yes | — | — | — | — |
| `--threads` | yes | yes | — | yes | yes | yes |
| `--mft` | yes | yes | no | no | no | yes |

## Python API

### `walk(directory, extensions=None, skip_hidden=False, max_depth=None, names=None, min_size_mb=None, max_size_mb=None, modified_after=None, modified_before=None, created_after=None, created_before=None, threads=None, mft=False)`
Parallel recursive directory walk. Returns `list[FileEntry]`.

When any filter is given, only matching files are returned; directories are
omitted. Without filters, files and directories are both included.

```python
import pyofiles

# Walk everything
entries = pyofiles.walk("/path/to/dir")

# Only Python files
entries = pyofiles.walk("/path", extensions=[".py"])

# Files modified in the last 24 hours
import time
recent = pyofiles.walk("/path", modified_after=time.time() - 86400)

# Walk with name and size filters
entries = pyofiles.walk("/path", names=["test"], min_size_mb=0.01)

for e in entries:
    if e.is_file:
        print(f"{e.name} ({e.size} bytes)")
```

### `find(directory, names=None, extensions=None, min_size_mb=None, max_size_mb=None, skip_hidden=False, max_depth=None, modified_after=None, modified_before=None, created_after=None, created_before=None, limit=None, threads=None, mft=False)`
Search for files by name substrings, extensions, size, and time. Accepts **multiple substrings** -- a file matches if its name contains ANY of them (case-insensitive).

`limit=N` stops the search as soon as N matches are found, which makes
single-file lookups in huge trees return almost immediately. Which matches
are returned when the limit truncates is unspecified.

```python
# Find files containing "report" or "invoice" in the name
results = pyofiles.find("/data", names=["report", "invoice"])

# Find large videos
results = pyofiles.find("/media", extensions=[".mp4", ".avi"], min_size_mb=100)

# Combine: name + extension + size
results = pyofiles.find("/docs", names=["2024"], extensions=[".pdf"], max_size_mb=50)

# Find recently modified Python files
results = pyofiles.find("/src", extensions=[".py"], modified_after=time.time() - 7*86400)

# Find files created in a date range
results = pyofiles.find("/logs", names=["error"],
                        created_after=1709251200, created_before=1711929600)

# Find by size alone
results = pyofiles.find("/data", min_size_mb=500)

# First match only: stops walking as soon as it is found
results = pyofiles.find("/drawings", names=["P-1042"], limit=1)
```

### `list_dir(directory, extensions=None, names=None, min_size_mb=None, max_size_mb=None, skip_hidden=False, modified_after=None, modified_before=None, created_after=None, created_before=None)`
Non-recursive single-directory listing. Returns `list[FileEntry]` sorted by name.

```python
entries = pyofiles.list_dir("/path")

# List only Python files, skip hidden
entries = pyofiles.list_dir("/src", extensions=[".py"], skip_hidden=True)

# List recently modified files
entries = pyofiles.list_dir("/data", modified_after=time.time() - 86400)
```

### `index(directory, extensions, skip_hidden=False, max_depth=None, names=None, min_size_mb=None, max_size_mb=None, modified_after=None, modified_before=None, created_after=None, created_before=None, threads=None)`
Build a file index grouped by filename stem. Useful for finding related files with different extensions.

If two files share the same stem and extension (e.g. in different
subdirectories), the lexicographically smallest full path is kept, so
results are deterministic.

```python
idx = pyofiles.index("/src", extensions=[".py", ".pyi", ".pyc"])
# {"main": {".py": "/src/main.py", ".pyc": "/src/__pycache__/main.pyc"}}

# Index only recently modified files
idx = pyofiles.index("/src", extensions=[".py"], modified_after=time.time() - 7*86400)

# Index with depth limit
idx = pyofiles.index("/project", extensions=[".py"], max_depth=3)
```

### `glob(directory, pattern, skip_hidden=False, max_depth=None, min_size_mb=None, max_size_mb=None, modified_after=None, modified_before=None, created_after=None, created_before=None, threads=None)`
Parallel glob pattern matching. Returns `list[str]` of full paths.

Patterns with a literal directory prefix (e.g. `src/**/*.py`) start the
walk at that directory instead of scanning the whole tree.

```python
paths = pyofiles.glob("/project", "**/*.py")
paths = pyofiles.glob("/project", "src/**/*.{rs,toml}")

# Glob with time filter
paths = pyofiles.glob("/project", "**/*.py", modified_after=time.time() - 7*86400)

# Glob with size filter
paths = pyofiles.glob("/data", "**/*.csv", min_size_mb=10)
```

### `disk_usage(directory, depth=1, top=20, skip_hidden=False, extensions=None, names=None, min_size_mb=None, max_size_mb=None, modified_after=None, modified_before=None, created_after=None, created_before=None, threads=None, mft=False)`
Analyze disk space usage by directory. Returns a `DiskUsage` object.

```python
usage = pyofiles.disk_usage("/home", depth=2, top=10)
print(f"Total: {usage.total_size_gb:.2f} GB across {usage.total_files} files")
for entry in usage.entries:
    print(f"  {entry.path}: {entry.size_mb:.1f} MB ({entry.file_count} files)")

# Disk usage for Python files only
usage = pyofiles.disk_usage("/project", extensions=[".py"])

# Disk usage of recently modified files
usage = pyofiles.disk_usage("/project", modified_after=time.time() - 30*86400)

# Disk usage of test files
usage = pyofiles.disk_usage("/project", names=["test"], extensions=[".py"])
```

## MFT fast path (Windows)

`walk`, `find`, and `disk_usage` accept `mft=True` (CLI: `--mft`) on Windows.
Instead of walking directories, pyofiles then reads the volume's NTFS Master
File Table directly through a raw volume handle -- the same approach WizTree
and Everything use. One pass over the MFT yields the metadata of every file
on the volume, which is dramatically faster than directory traversal for
large subtrees or whole drives.

```python
# Whole-drive usage in seconds instead of minutes
usage = pyofiles.disk_usage("C:\\", mft=True, depth=2, top=20)

# Volume-wide name lookups, Everything-style
hits = pyofiles.find("C:\\", names=["report_2024"], extensions=[".pdf"], mft=True)
```

All filters, return types, and result shapes match the walk backend. It is
always an explicit opt-in and never enabled automatically, because the
requirements and semantics differ:

- **Requirements**: administrator privileges and a local NTFS volume.
  Without them an `OSError` explains what is missing. UNC and network paths
  are rejected. On non-Windows builds `mft=True` raises `ValueError`.
- **Whole-volume scan**: the MFT is read for the entire volume and results
  are filtered to the requested subtree, so cost is proportional to the
  volume's total file count, not the subtree's. Huge wins for big trees;
  overkill for scanning a small folder.
- **Hard links**: every hard link appears once per name, exactly like a
  directory walker sees them.
- **Sizes**: logical sizes of the unnamed data stream, matching
  `os.stat().st_size`. Alternate data streams are not counted.
- **Snapshot semantics**: reads the on-disk state. Writes from the last few
  seconds may not be visible yet, and results are a point-in-time snapshot
  of the volume.
- **Access**: raw volume reads bypass file ACLs, so `mft=True` can return
  entries inside directories a normal walk cannot open.
- **Reparse points**: symlinks and junctions are listed, not followed.
- **Paths**: results always use absolute canonical paths (e.g.
  `C:\Users\...`), regardless of how the directory argument was spelled.
- `skip_hidden` uses the NTFS hidden attribute plus dot-prefixed names, and
  hides everything beneath a hidden directory, like the walker does.
- NTFS metadata files (`$MFT`, `$LogFile`, `$Extend`, ...) are excluded,
  matching what directory enumeration shows.

## Types

### `FileEntry`
Returned by `walk`, `find`, `list_dir`.

| Attribute   | Type             |
|-------------|------------------|
| `path`      | `str`            |
| `name`      | `str`            |
| `is_file`   | `bool`           |
| `is_dir`    | `bool`           |
| `size`      | `int`            |
| `extension` | `str`            |
| `modified`  | `float` or `None`|
| `created`   | `float` or `None`|

Timestamps are unix epoch seconds. Use `datetime.fromtimestamp(entry.modified)` to convert.
`created` may be `None` on Linux systems that don't support creation time.

### `SizeEntry`
Returned inside `DiskUsage.entries`.

| Attribute    | Type    |
|--------------|---------|
| `path`       | `str`   |
| `size`       | `int`   |
| `file_count` | `int`   |
| `size_mb`    | `float` |
| `size_gb`    | `float` |

### `DiskUsage`
Returned by `disk_usage`.

| Attribute       | Type              |
|-----------------|-------------------|
| `entries`       | `list[SizeEntry]` |
| `total_size`    | `int`             |
| `total_files`   | `int`             |
| `total_size_mb` | `float`           |
| `total_size_gb` | `float`           |

## Behavior notes

- **Hidden files**: `skip_hidden` skips dot-prefixed names everywhere, and also
  files with the hidden attribute on Windows.
- **Creation time**: on filesystems that do not support creation time (some
  Linux setups), `created_after`/`created_before` match no files. `FileEntry.created`
  is `None` there.
- **Unreadable metadata**: when a size or time filter is active, files whose
  metadata cannot be read are excluded.
- **Result order**: `walk`, `find`, and `glob` run in parallel and return results
  in no particular order. `list_dir` is sorted by name.

## Performance

Every recursive operation runs on the [ignore](https://crates.io/crates/ignore)
crate's parallel directory walker (the same engine ripgrep uses), with metadata
taken from the directory read itself wherever the platform provides it (free on
Windows). Bindings via [PyO3](https://pyo3.rs). Typically **5-50x faster** than
equivalent Python code, especially on large directories and network drives.
Wheels are abi3: one wheel per platform covers Python 3.9+.

## License

MIT
