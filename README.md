<div align="center">
  <img src="https://raw.githubusercontent.com/lperezmo/pyofiles/master/icons/pyofiles_logo.svg" alt="pyofiles logo" width="400">
  <br><br>

  <a href="https://github.com/lperezmo/pyofiles/actions/workflows/ci.yml"><img src="https://github.com/lperezmo/pyofiles/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://pypi.org/project/pyofiles/"><img src="https://img.shields.io/pypi/v/pyofiles" alt="PyPI version"></a>
  <a href="https://pypistats.org/packages/pyofiles"><img src="https://img.shields.io/pypi/dm/pyofiles" alt="Downloads"></a>
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

### Examples

```bash
# Walk a directory, showing only Python files
pyofiles walk ./src --ext .py --skip-hidden

# Long format: type, size, modified time, path
pyofiles walk ./src --ext .py -l

# Find files by name substring
pyofiles find ./data --names report invoice --ext .pdf

# Find files modified in the last 7 days
pyofiles find ./project --ext .py --modified-after 7d

# Find files created before a specific date
pyofiles find ./logs --ext .log --created-before 2024-06-01

# Find large files modified recently
pyofiles find ./data --ext .csv --min-size 100 --modified-after 24h

# Glob pattern matching
pyofiles glob ./project "**/*.rs"

# List directory contents
pyofiles ls ./some/dir -l

# Index files by stem
pyofiles index ./src --ext .py .pyi .pyc

# Disk usage analysis
pyofiles du ./project --depth 2 --top 10

# JSON output (works with all commands, pipe to jq)
pyofiles find ./src --ext .py --json
pyofiles du . --json | jq '.entries[:5]'
```

### Time formats

Time arguments (`--modified-after`, `--modified-before`, `--created-after`, `--created-before`) accept:

| Format | Example | Meaning |
|--------|---------|---------|
| Relative duration | `7d`, `24h`, `30m`, `1w` | ago from now |
| ISO date | `2024-03-15` | midnight on that date |
| ISO datetime | `2024-03-15T10:30:00` | specific moment |
| Unix timestamp | `1709251200` | raw epoch seconds |

## Python API

### `walk(directory, extensions=None, skip_hidden=False, max_depth=None, modified_after=None, modified_before=None, created_after=None, created_before=None)`
Parallel recursive directory walk. Returns `list[FileEntry]`.

```python
import pyofiles

# Walk everything
entries = pyofiles.walk("/path/to/dir")

# Only Python files
entries = pyofiles.walk("/path", extensions=[".py"])

# Files modified in the last 24 hours
import time
recent = pyofiles.walk("/path", modified_after=time.time() - 86400)

for e in entries:
    if e.is_file:
        print(f"{e.name} ({e.size} bytes)")
```

### `find(directory, names=None, extensions=None, min_size_mb=None, max_size_mb=None, modified_after=None, modified_before=None, created_after=None, created_before=None, ...)`
Search for files by name substrings, extensions, size, and time. Accepts **multiple substrings** -- a file matches if its name contains ANY of them (case-insensitive).

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
```

### `list_dir(directory)`
Non-recursive single-directory listing. Returns `list[FileEntry]`.

```python
entries = pyofiles.list_dir("/path")
```

### `index(directory, extensions, skip_hidden=False)`
Build a file index grouped by filename stem. Useful for finding related files with different extensions.

```python
idx = pyofiles.index("/src", extensions=[".py", ".pyi", ".pyc"])
# {"main": {".py": "/src/main.py", ".pyc": "/src/__pycache__/main.pyc"}}
```

### `glob(directory, pattern, skip_hidden=False)`
Parallel glob pattern matching. Returns `list[str]` of full paths.

```python
paths = pyofiles.glob("/project", "**/*.py")
paths = pyofiles.glob("/project", "src/**/*.{rs,toml}")
```

### `disk_usage(directory, depth=1, top=20, skip_hidden=False)`
Analyze disk space usage by directory. Returns a `DiskUsage` object.

```python
usage = pyofiles.disk_usage("/home", depth=2, top=10)
print(f"Total: {usage.total_size_gb:.2f} GB across {usage.total_files} files")
for entry in usage.entries:
    print(f"  {entry.path}: {entry.size_mb:.1f} MB ({entry.file_count} files)")
```

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

## Performance

Built on [jwalk](https://crates.io/crates/jwalk) (parallel directory walker) and [PyO3](https://pyo3.rs). Typically **5-50x faster** than equivalent Python code, especially on large directories and network drives.

## License

MIT
