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

## Python API

### `walk(directory, extensions=None, skip_hidden=False, max_depth=None, names=None, min_size_mb=None, max_size_mb=None, modified_after=None, modified_before=None, created_after=None, created_before=None)`
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

# Walk with name and size filters
entries = pyofiles.walk("/path", names=["test"], min_size_mb=0.01)

for e in entries:
    if e.is_file:
        print(f"{e.name} ({e.size} bytes)")
```

### `find(directory, names=None, extensions=None, min_size_mb=None, max_size_mb=None, skip_hidden=False, max_depth=None, modified_after=None, modified_before=None, created_after=None, created_before=None)`
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

# Find by size alone
results = pyofiles.find("/data", min_size_mb=500)
```

### `list_dir(directory, extensions=None, names=None, min_size_mb=None, max_size_mb=None, skip_hidden=False, modified_after=None, modified_before=None, created_after=None, created_before=None)`
Non-recursive single-directory listing. Returns `list[FileEntry]`.

```python
entries = pyofiles.list_dir("/path")

# List only Python files, skip hidden
entries = pyofiles.list_dir("/src", extensions=[".py"], skip_hidden=True)

# List recently modified files
entries = pyofiles.list_dir("/data", modified_after=time.time() - 86400)
```

### `index(directory, extensions, skip_hidden=False, max_depth=None, names=None, min_size_mb=None, max_size_mb=None, modified_after=None, modified_before=None, created_after=None, created_before=None)`
Build a file index grouped by filename stem. Useful for finding related files with different extensions.

```python
idx = pyofiles.index("/src", extensions=[".py", ".pyi", ".pyc"])
# {"main": {".py": "/src/main.py", ".pyc": "/src/__pycache__/main.pyc"}}

# Index only recently modified files
idx = pyofiles.index("/src", extensions=[".py"], modified_after=time.time() - 7*86400)

# Index with depth limit
idx = pyofiles.index("/project", extensions=[".py"], max_depth=3)
```

### `glob(directory, pattern, skip_hidden=False, max_depth=None, min_size_mb=None, max_size_mb=None, modified_after=None, modified_before=None, created_after=None, created_before=None)`
Parallel glob pattern matching. Returns `list[str]` of full paths.

```python
paths = pyofiles.glob("/project", "**/*.py")
paths = pyofiles.glob("/project", "src/**/*.{rs,toml}")

# Glob with time filter
paths = pyofiles.glob("/project", "**/*.py", modified_after=time.time() - 7*86400)

# Glob with size filter
paths = pyofiles.glob("/data", "**/*.csv", min_size_mb=10)
```

### `disk_usage(directory, depth=1, top=20, skip_hidden=False, extensions=None, names=None, min_size_mb=None, max_size_mb=None, modified_after=None, modified_before=None, created_after=None, created_before=None)`
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
