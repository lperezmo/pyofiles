# pyofiles

Fast, Rust-powered file operations for Python. Drop-in replacements for `os.walk`, `os.listdir`, and `glob.glob` -- built on parallel directory walkers for maximum speed.

## Install

```bash
pip install pyofiles
```

## Functions

### `walk(directory, extensions=None, skip_hidden=False, max_depth=None)`
Parallel recursive directory walk. Returns `list[FileEntry]`.

```python
import pyofiles

# Walk everything
entries = pyofiles.walk("/path/to/dir")

# Only Python files
entries = pyofiles.walk("/path", extensions=[".py"])

for e in entries:
    if e.is_file:
        print(f"{e.name} ({e.size} bytes)")
```

### `find(directory, names=None, extensions=None, min_size_mb=None, max_size_mb=None, ...)`
Search for files by name substrings, extensions, and size. Accepts **multiple substrings** -- a file matches if its name contains ANY of them (case-insensitive).

```python
# Find files containing "report" or "invoice" in the name
results = pyofiles.find("/data", names=["report", "invoice"])

# Find large videos
results = pyofiles.find("/media", extensions=[".mp4", ".avi"], min_size_mb=100)

# Combine: name + extension + size
results = pyofiles.find("/docs", names=["2024"], extensions=[".pdf"], max_size_mb=50)
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

| Attribute   | Type   |
|-------------|--------|
| `path`      | `str`  |
| `name`      | `str`  |
| `is_file`   | `bool` |
| `is_dir`    | `bool` |
| `size`      | `int`  |
| `extension` | `str`  |

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
