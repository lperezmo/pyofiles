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
    finally:
        teardown_fixtures()

    section("RESULTS")
    print(f"  {passed} passed, {failed} failed, {passed + failed} total")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
