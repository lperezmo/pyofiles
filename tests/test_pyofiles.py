"""
Comprehensive test script for pyofiles library.
Tests all functions: walk, find, list_dir, index, glob, disk_usage.
"""

import os
import sys
import time
import shutil
from pathlib import Path

import pyofiles

# ---------------------------------------------------------------------------
# Setup: build a fixture directory tree with known structure
# ---------------------------------------------------------------------------

FIXTURES = Path(__file__).parent / "test_fixtures"


def setup_fixtures():
    """Create a reproducible directory tree for testing."""
    if FIXTURES.exists():
        shutil.rmtree(FIXTURES)

    # directories
    (FIXTURES / "src" / "helpers").mkdir(parents=True)
    (FIXTURES / "data").mkdir()
    (FIXTURES / "docs" / "images").mkdir(parents=True)

    # root-level files
    (FIXTURES / ".hidden_file.txt").write_text("hidden")
    (FIXTURES / "readme.txt").write_text("hello world")
    (FIXTURES / "report_2024.pdf").write_bytes(b"%PDF-fake-report")
    (FIXTURES / "invoice_march.pdf").write_bytes(b"%PDF-fake-invoice")

    # src/
    (FIXTURES / "src" / "main.py").write_text("print('main')\n")
    (FIXTURES / "src" / "main.pyc").write_bytes(b"\x00compiled")
    (FIXTURES / "src" / "utils.py").write_text("# utils\n")
    (FIXTURES / "src" / "helpers" / "io.py").write_text("# io helpers\n")
    (FIXTURES / "src" / "helpers" / "io.pyi").write_text("# io stubs\n")

    # data/
    (FIXTURES / "data" / "output.csv").write_text("a,b,c\n1,2,3\n")
    (FIXTURES / "data" / "output.json").write_text('{"key": "value"}\n')
    # ~1.5 MB file for size-filter tests
    (FIXTURES / "data" / "large_file.bin").write_bytes(b"\x00" * (1_500_000))

    # docs/
    (FIXTURES / "docs" / "guide.md").write_text("# Guide\n")
    (FIXTURES / "docs" / "images" / "logo.png").write_bytes(b"\x89PNG-fake")


def teardown_fixtures():
    if FIXTURES.exists():
        shutil.rmtree(FIXTURES)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

passed = 0
failed = 0


def check(label: str, condition: bool, detail: str = ""):
    global passed, failed
    if condition:
        passed += 1
        print(f"  PASS  {label}")
    else:
        failed += 1
        msg = f"  FAIL  {label}"
        if detail:
            msg += f"  -- {detail}"
        print(msg)


def section(title: str):
    print(f"\n{'='*60}\n  {title}\n{'='*60}")


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

def test_walk():
    section("walk")
    root = str(FIXTURES)

    # basic walk — should find all files and dirs
    entries = pyofiles.walk(root)
    names = {e.name for e in entries}
    check("returns entries", len(entries) > 0)
    check("includes nested file", "io.py" in names, f"names={names}")
    check("includes directory", "helpers" in names or any(e.is_dir for e in entries))

    # extension filter
    py_entries = pyofiles.walk(root, extensions=[".py"])
    py_names = {e.name for e in py_entries if e.is_file}
    check("extension filter .py", py_names == {"main.py", "utils.py", "io.py"},
          f"got {py_names}")

    # skip_hidden
    visible = pyofiles.walk(root, skip_hidden=True)
    visible_names = {e.name for e in visible}
    check("skip_hidden excludes dotfiles", ".hidden_file.txt" not in visible_names,
          f"visible_names={visible_names}")

    # max_depth
    shallow = pyofiles.walk(root, max_depth=1)
    shallow_names = {e.name for e in shallow if e.is_file}
    check("max_depth=1 excludes deep files", "io.py" not in shallow_names,
          f"shallow_names={shallow_names}")

    # FileEntry attributes
    file_entries = [e for e in entries if e.is_file and e.name == "readme.txt"]
    if file_entries:
        fe = file_entries[0]
        check("FileEntry.path is str", isinstance(fe.path, str))
        check("FileEntry.name", fe.name == "readme.txt")
        check("FileEntry.is_file", fe.is_file is True)
        check("FileEntry.is_dir", fe.is_dir is False)
        check("FileEntry.size > 0", fe.size > 0, f"size={fe.size}")
        check("FileEntry.extension (no dot)", fe.extension == "txt", f"ext={fe.extension}")
    else:
        check("FileEntry lookup", False, "readme.txt not found")


def test_find():
    section("find")
    root = str(FIXTURES)

    # find by name substring
    results = pyofiles.find(root, names=["report"])
    found = {e.name for e in results}
    check("find by name 'report'", "report_2024.pdf" in found, f"found={found}")

    # multiple name substrings (OR logic)
    results = pyofiles.find(root, names=["report", "invoice"])
    found = {e.name for e in results}
    check("find multiple names", "report_2024.pdf" in found and "invoice_march.pdf" in found,
          f"found={found}")

    # find by extension
    results = pyofiles.find(root, extensions=[".csv", ".json"])
    found = {e.name for e in results}
    check("find by extensions", found == {"output.csv", "output.json"}, f"found={found}")

    # find by min_size_mb (need at least names or extensions too)
    results = pyofiles.find(root, extensions=[".bin"], min_size_mb=1)
    found = {e.name for e in results}
    check("find min_size_mb=1", "large_file.bin" in found, f"found={found}")

    # find by max_size_mb
    results = pyofiles.find(root, extensions=[".bin", ".txt", ".py"], max_size_mb=1)
    found = {e.name for e in results}
    check("find max_size_mb=1 excludes large", "large_file.bin" not in found, f"found={found}")

    # combined filters
    results = pyofiles.find(root, names=["output"], extensions=[".json"])
    found = {e.name for e in results}
    check("find combined name+ext", found == {"output.json"}, f"found={found}")

    # limit stops early
    results = pyofiles.find(root, extensions=[".py"], limit=2)
    check("find limit=2 returns 2", len(results) == 2, f"count={len(results)}")

    results = pyofiles.find(root, extensions=[".py"], limit=100)
    check("find limit above match count returns all", len(results) == 3,
          f"count={len(results)}")

    results = pyofiles.find(root, extensions=[".py"], limit=0)
    check("find limit=0 returns empty", len(results) == 0, f"count={len(results)}")

    # threads parameter accepted
    results = pyofiles.find(root, extensions=[".py"], threads=2)
    check("find threads=2 works", len(results) == 3, f"count={len(results)}")


def test_list_dir():
    section("list_dir")
    root = str(FIXTURES)

    entries = pyofiles.list_dir(root)
    names = {e.name for e in entries}
    check("list_dir returns entries", len(entries) > 0)
    check("list_dir has root files", "readme.txt" in names, f"names={names}")
    check("list_dir has subdirs", "src" in names)
    check("list_dir is non-recursive", "io.py" not in names,
          "should not contain deeply nested files")

    # check a subdirectory
    src_entries = pyofiles.list_dir(str(FIXTURES / "src"))
    src_names = {e.name for e in src_entries}
    check("list_dir src/", "main.py" in src_names, f"src_names={src_names}")


def test_index():
    section("index")

    # Index src/ for Python-related extensions
    idx = pyofiles.index(str(FIXTURES / "src"), extensions=[".py", ".pyi", ".pyc"])
    check("index returns dict", isinstance(idx, dict))
    check("index has 'main' stem", "main" in idx, f"keys={list(idx.keys())}")
    check("index has 'io' stem", "io" in idx, f"keys={list(idx.keys())}")

    if "main" in idx:
        main_exts = set(idx["main"].keys())
        check("main has .py", ".py" in main_exts, f"exts={main_exts}")
        check("main has .pyc", ".pyc" in main_exts, f"exts={main_exts}")

    if "io" in idx:
        io_exts = set(idx["io"].keys())
        check("io has .py and .pyi", {".py", ".pyi"} <= io_exts, f"exts={io_exts}")


def test_glob():
    section("glob")
    root = str(FIXTURES)

    # all .py files recursively
    paths = pyofiles.glob(root, "**/*.py")
    basenames = {os.path.basename(p) for p in paths}
    check("glob **/*.py", {"main.py", "utils.py", "io.py"} <= basenames,
          f"basenames={basenames}")

    # single-level glob
    paths = pyofiles.glob(root, "*.txt")
    basenames = {os.path.basename(p) for p in paths}
    check("glob *.txt", "readme.txt" in basenames, f"basenames={basenames}")

    # pdf glob
    paths = pyofiles.glob(root, "**/*.pdf")
    basenames = {os.path.basename(p) for p in paths}
    check("glob **/*.pdf", {"report_2024.pdf", "invoice_march.pdf"} <= basenames,
          f"basenames={basenames}")

    # skip_hidden
    paths_visible = pyofiles.glob(root, "**/*.txt", skip_hidden=True)
    basenames_vis = {os.path.basename(p) for p in paths_visible}
    check("glob skip_hidden", ".hidden_file.txt" not in basenames_vis)


def test_disk_usage():
    section("disk_usage")
    root = str(FIXTURES)

    usage = pyofiles.disk_usage(root, depth=2, top=10)

    check("DiskUsage.total_size > 0", usage.total_size > 0, f"total={usage.total_size}")
    check("DiskUsage.total_files > 0", usage.total_files > 0, f"files={usage.total_files}")
    check("DiskUsage.total_size_mb > 1", usage.total_size_mb > 1,
          f"size_mb={usage.total_size_mb}")
    check("DiskUsage.total_size_gb is float", isinstance(usage.total_size_gb, float))

    check("entries is list", isinstance(usage.entries, list))
    if usage.entries:
        e = usage.entries[0]
        check("SizeEntry.path", isinstance(e.path, str))
        check("SizeEntry.size", isinstance(e.size, int) and e.size >= 0)
        check("SizeEntry.file_count", isinstance(e.file_count, int))
        check("SizeEntry.size_mb", isinstance(e.size_mb, float))
        check("SizeEntry.size_gb", isinstance(e.size_gb, float))

    # depth=1 should give fewer entries than depth=2
    usage1 = pyofiles.disk_usage(root, depth=1)
    usage2 = pyofiles.disk_usage(root, depth=2)
    check("depth=2 >= depth=1 entries",
          len(usage2.entries) >= len(usage1.entries),
          f"d1={len(usage1.entries)} d2={len(usage2.entries)}")


def test_time_filters():
    section("time filters")
    root = str(FIXTURES)

    # All fixture files were just created, so modified time is recent
    now = time.time()
    one_minute_ago = now - 60

    # FileEntry should have timestamps
    entries = pyofiles.walk(root)
    file_entries = [e for e in entries if e.is_file]
    if file_entries:
        fe = file_entries[0]
        check("FileEntry.modified is float", isinstance(fe.modified, float),
              f"type={type(fe.modified)}")
        check("FileEntry.modified is recent", fe.modified > one_minute_ago,
              f"modified={fe.modified}, threshold={one_minute_ago}")
        check("FileEntry.created is float or None",
              fe.created is None or isinstance(fe.created, float))
    else:
        check("fixture files exist", False, "no file entries found")

    # walk with modified_after should return recently created fixtures
    recent = pyofiles.walk(root, modified_after=one_minute_ago)
    recent_files = [e for e in recent if e.is_file]
    check("walk modified_after finds recent files", len(recent_files) > 0,
          f"count={len(recent_files)}")

    # walk with modified_before=one_minute_ago should return nothing (all files are newer)
    old = pyofiles.walk(root, modified_before=one_minute_ago)
    old_files = [e for e in old if e.is_file]
    check("walk modified_before excludes recent", len(old_files) == 0,
          f"count={len(old_files)}")

    # find with modified_after
    found = pyofiles.find(root, extensions=[".py"], modified_after=one_minute_ago)
    found_names = {e.name for e in found}
    check("find modified_after + ext", "main.py" in found_names,
          f"found={found_names}")

    # find with only time filter (no names or extensions)
    found_time_only = pyofiles.find(root, modified_after=one_minute_ago)
    check("find with time filter only", len(found_time_only) > 0,
          f"count={len(found_time_only)}")

    # find with modified_before should exclude all recent files
    found_old = pyofiles.find(root, extensions=[".py"], modified_before=one_minute_ago)
    check("find modified_before excludes recent", len(found_old) == 0,
          f"count={len(found_old)}")


def test_walk_name_and_size_filters():
    section("walk — name and size filters")
    root = str(FIXTURES)

    # walk with names filter
    entries = pyofiles.walk(root, names=["main"])
    file_names = {e.name for e in entries if e.is_file}
    check("walk names=['main'] finds main files",
          "main.py" in file_names and "main.pyc" in file_names,
          f"got {file_names}")

    # walk with min_size_mb — only large_file.bin is >1MB
    entries = pyofiles.walk(root, min_size_mb=1)
    file_names = {e.name for e in entries if e.is_file}
    check("walk min_size_mb=1 finds large file",
          file_names == {"large_file.bin"},
          f"got {file_names}")

    # walk with max_size_mb — should exclude large_file.bin
    entries = pyofiles.walk(root, max_size_mb=1)
    file_names = {e.name for e in entries if e.is_file}
    check("walk max_size_mb=1 excludes large file",
          "large_file.bin" not in file_names,
          f"got {file_names}")

    # when any filter is active, directories are excluded from results
    entries = pyofiles.walk(root, extensions=[".py"])
    check("walk with filters returns files only",
          all(e.is_file for e in entries),
          f"non-files: {[e.name for e in entries if not e.is_file]}")

    # without filters, directories are included
    entries = pyofiles.walk(root)
    check("walk without filters includes dirs",
          any(e.is_dir for e in entries))

    # threads parameter accepted
    entries = pyofiles.walk(root, extensions=[".py"], threads=2)
    check("walk threads=2 works", len(entries) == 3, f"count={len(entries)}")


def test_list_dir_filters():
    section("list_dir — filters")
    root = str(FIXTURES)

    # extension filter
    entries = pyofiles.list_dir(root, extensions=[".txt"])
    file_names = {e.name for e in entries if e.is_file}
    check("list_dir ext=.txt", "readme.txt" in file_names, f"got {file_names}")
    check("list_dir ext=.txt excludes pdf", "report_2024.pdf" not in file_names)

    # skip_hidden
    entries = pyofiles.list_dir(root, skip_hidden=True)
    names = {e.name for e in entries}
    check("list_dir skip_hidden", ".hidden_file.txt" not in names)

    entries_all = pyofiles.list_dir(root, skip_hidden=False)
    names_all = {e.name for e in entries_all}
    check("list_dir shows hidden by default", ".hidden_file.txt" in names_all)

    # names filter
    entries = pyofiles.list_dir(root, names=["report"])
    file_names = {e.name for e in entries if e.is_file}
    check("list_dir names=['report']", "report_2024.pdf" in file_names, f"got {file_names}")

    # time filter
    now = time.time()
    entries = pyofiles.list_dir(root, modified_after=now - 60)
    file_names = {e.name for e in entries if e.is_file}
    check("list_dir modified_after finds recent", len(file_names) > 0, f"got {file_names}")


def test_glob_filters():
    section("glob — filters")
    root = str(FIXTURES)

    # max_depth
    paths = pyofiles.glob(root, "**/*.py", max_depth=2)
    basenames = {os.path.basename(p) for p in paths}
    check("glob max_depth=2 excludes deep files", "io.py" not in basenames,
          f"basenames={basenames}")

    # time filter
    now = time.time()
    paths = pyofiles.glob(root, "**/*.py", modified_after=now - 60)
    basenames = {os.path.basename(p) for p in paths}
    check("glob modified_after finds recent .py", "main.py" in basenames,
          f"basenames={basenames}")

    paths_old = pyofiles.glob(root, "**/*.py", modified_before=now - 60)
    check("glob modified_before excludes recent", len(paths_old) == 0,
          f"count={len(paths_old)}")

    # size filter
    paths = pyofiles.glob(root, "**/*", min_size_mb=1)
    basenames = {os.path.basename(p) for p in paths}
    check("glob min_size_mb=1 finds large file", "large_file.bin" in basenames,
          f"basenames={basenames}")


def test_index_filters():
    section("index — filters")

    # max_depth — index src/ with max_depth=1, should not find helpers/io
    idx = pyofiles.index(str(FIXTURES / "src"), extensions=[".py", ".pyi"], max_depth=1)
    check("index max_depth=1 excludes deep", "io" not in idx, f"keys={list(idx.keys())}")
    check("index max_depth=1 includes shallow", "main" in idx, f"keys={list(idx.keys())}")

    # time filter
    now = time.time()
    idx = pyofiles.index(str(FIXTURES / "src"), extensions=[".py"], modified_after=now - 60)
    check("index modified_after finds recent", "main" in idx, f"keys={list(idx.keys())}")

    idx_old = pyofiles.index(str(FIXTURES / "src"), extensions=[".py"], modified_before=now - 60)
    check("index modified_before excludes recent", len(idx_old) == 0,
          f"keys={list(idx_old.keys())}")

    # names filter
    idx = pyofiles.index(str(FIXTURES / "src"), extensions=[".py", ".pyi", ".pyc"], names=["io"])
    check("index names=['io']", "io" in idx and "main" not in idx,
          f"keys={list(idx.keys())}")


def test_disk_usage_filters():
    section("disk_usage — filters")
    root = str(FIXTURES)

    # extensions filter — only .py files
    usage = pyofiles.disk_usage(root, extensions=[".py"])
    py_names = set()
    for e in usage.entries:
        py_names.add(os.path.basename(e.path))
    check("du extensions=.py counts only py", usage.total_files > 0,
          f"total_files={usage.total_files}")
    # total size should be much less than the 1.5MB large_file.bin
    check("du extensions=.py small total", usage.total_size < 500_000,
          f"total_size={usage.total_size}")

    # names filter
    usage = pyofiles.disk_usage(root, names=["large"])
    check("du names=['large'] finds large file", usage.total_files == 1,
          f"total_files={usage.total_files}")
    check("du names=['large'] correct size", usage.total_size >= 1_400_000,
          f"total_size={usage.total_size}")

    # time filter
    now = time.time()
    usage = pyofiles.disk_usage(root, modified_after=now - 60)
    check("du modified_after finds recent", usage.total_files > 0,
          f"total_files={usage.total_files}")

    usage_old = pyofiles.disk_usage(root, modified_before=now - 60)
    check("du modified_before excludes recent", usage_old.total_files == 0,
          f"total_files={usage_old.total_files}")

    # min_size_mb filter
    usage = pyofiles.disk_usage(root, min_size_mb=1)
    check("du min_size_mb=1 only large", usage.total_files == 1,
          f"total_files={usage.total_files}")


def test_find_size_only():
    section("find — size filter only")
    root = str(FIXTURES)

    # find with only size filter (no names or extensions) — should work now
    results = pyofiles.find(root, min_size_mb=1)
    found = {e.name for e in results}
    check("find min_size_mb=1 alone works", "large_file.bin" in found, f"found={found}")


def test_index_collisions():
    section("index — stem collisions are deterministic")

    # Two files with the same stem and extension in different directories:
    # the lexicographically smallest full path must win, every time.
    (FIXTURES / "data" / "dup.py").write_text("# data copy\n")
    (FIXTURES / "src" / "dup.py").write_text("# src copy\n")

    expected = str(FIXTURES / "data" / "dup.py")
    for run in range(5):
        idx = pyofiles.index(str(FIXTURES), extensions=[".py"])
        got = idx.get("dup", {}).get(".py")
        if got != expected:
            check(f"index collision keeps smallest path (run {run})", False,
                  f"got {got}, expected {expected}")
            return
    check("index collision keeps smallest path across 5 runs", True)


def test_list_dir_sorted():
    section("list_dir — sorted output")
    entries = pyofiles.list_dir(str(FIXTURES))
    names = [e.name for e in entries]
    check("list_dir returns sorted names", names == sorted(names), f"names={names}")


def _is_windows_admin():
    """True when running elevated on Windows (e.g. GitHub windows runners)."""
    if sys.platform != "win32":
        return False
    import ctypes
    try:
        return ctypes.windll.shell32.IsUserAnAdmin() != 0
    except Exception:
        return False


def test_mft():
    section("mft fast path")
    root = str(FIXTURES)

    calls = [
        ("disk_usage", lambda: pyofiles.disk_usage(root, mft=True)),
        ("walk", lambda: pyofiles.walk(root, mft=True)),
        ("find", lambda: pyofiles.find(root, names=["readme"], mft=True)),
    ]

    if sys.platform != "win32":
        # mft=True must raise ValueError on non-Windows builds.
        for label, call in calls:
            try:
                call()
                check(f"{label} mft=True raises on non-Windows", False, "no exception")
            except ValueError as e:
                check(f"{label} mft=True raises ValueError on non-Windows",
                      "only available on Windows" in str(e), f"msg={e}")
            except Exception as e:
                check(f"{label} mft=True raises ValueError on non-Windows", False,
                      f"wrong exception {type(e).__name__}: {e}")
        return

    # Windows: UNC and network paths are rejected regardless of elevation,
    # even when the share does not exist.
    try:
        pyofiles.find(r"\\localhost\nonexistent\share", names=["x"], mft=True)
        check("find mft=True rejects UNC path", False, "no exception")
    except OSError as e:
        msg = str(e)
        check("find mft=True rejects UNC path with OSError", True)
        check("UNC error states the requirements",
              "administrator" in msg and "NTFS" in msg, f"msg={msg}")
    except Exception as e:
        check("find mft=True rejects UNC path with OSError", False,
              f"wrong exception {type(e).__name__}: {e}")

    if _is_windows_admin():
        # The unprivileged error path cannot be exercised from an elevated
        # shell; CI covers the real scan in a dedicated workflow step.
        print("  SKIP  non-admin mft error tests (running elevated)")
        return

    # Without elevation, opening the raw volume must fail with a clear
    # OSError that tells the user what is needed.
    for label, call in calls:
        try:
            call()
            check(f"{label} mft=True raises without admin", False, "no exception")
        except OSError as e:
            msg = str(e)
            check(f"{label} mft=True raises OSError without admin", True)
            check(f"{label} error mentions administrator",
                  "administrator" in msg, f"msg={msg}")
        except Exception as e:
            check(f"{label} mft=True raises OSError without admin", False,
                  f"wrong exception {type(e).__name__}: {e}")


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    print("Setting up test fixtures...")
    setup_fixtures()

    try:
        test_walk()
        test_find()
        test_list_dir()
        test_index()
        test_glob()
        test_disk_usage()
        test_time_filters()
        test_walk_name_and_size_filters()
        test_list_dir_filters()
        test_glob_filters()
        test_index_filters()
        test_disk_usage_filters()
        test_find_size_only()
        test_index_collisions()
        test_list_dir_sorted()
        test_mft()
    finally:
        teardown_fixtures()

    section("RESULTS")
    print(f"  {passed} passed, {failed} failed, {passed + failed} total")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
