"""Pytest suite for pyofiles: bindings, filters, CLI parsing, and the MFT fast path.

Every test uses real asserts, so failures fail under pytest as well as
`python tests/test_pyofiles.py` (which delegates to pytest).
"""

from __future__ import annotations

import argparse
import contextlib
import ctypes
import io
import sys
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path
from types import SimpleNamespace

import pytest

import pyofiles
from pyofiles.cli import (
    build_parser,
    escape_terminal_controls,
    format_size,
    parse_time,
    print_entries,
)

# ---------------------------------------------------------------------------
# Fixture tree
# ---------------------------------------------------------------------------


@pytest.fixture
def tree(tmp_path: Path) -> Path:
    """A fresh directory tree with a known structure for every test."""
    root = tmp_path / "fixtures"
    (root / "src" / "helpers").mkdir(parents=True)
    (root / "data").mkdir()
    (root / "docs" / "images").mkdir(parents=True)

    # root-level files
    (root / ".hidden_file.txt").write_text("hidden")
    (root / "readme.txt").write_text("hello world")
    (root / "report_2024.pdf").write_bytes(b"%PDF-fake-report")
    (root / "invoice_march.pdf").write_bytes(b"%PDF-fake-invoice")

    # src/
    (root / "src" / "main.py").write_text("print('main')\n")
    (root / "src" / "main.pyc").write_bytes(b"\x00compiled")
    (root / "src" / "utils.py").write_text("# utils\n")
    (root / "src" / "helpers" / "io.py").write_text("# io helpers\n")
    (root / "src" / "helpers" / "io.pyi").write_text("# io stubs\n")

    # data/
    (root / "data" / "output.csv").write_text("a,b,c\n1,2,3\n")
    (root / "data" / "output.json").write_text('{"key": "value"}\n')
    # ~1.5 MB file for size-filter tests
    (root / "data" / "large_file.bin").write_bytes(b"\x00" * 1_500_000)

    return root


def file_names(entries) -> set[str]:
    return {e.name for e in entries if e.is_file}


# ---------------------------------------------------------------------------
# walk
# ---------------------------------------------------------------------------


def test_walk_basic(tree: Path):
    entries = pyofiles.walk(str(tree))
    names = {e.name for e in entries}
    assert entries
    assert "io.py" in names
    assert any(e.is_dir for e in entries)


def test_walk_extension_filter(tree: Path):
    py_entries = pyofiles.walk(str(tree), extensions=[".py"])
    assert file_names(py_entries) == {"main.py", "utils.py", "io.py"}
    # extensions normalize: no leading dot must work too
    assert file_names(pyofiles.walk(str(tree), extensions=["PY"])) == {
        "main.py", "utils.py", "io.py",
    }


def test_walk_skip_hidden(tree: Path):
    visible = pyofiles.walk(str(tree), skip_hidden=True)
    assert ".hidden_file.txt" not in {e.name for e in visible}


def test_walk_max_depth(tree: Path):
    shallow = pyofiles.walk(str(tree), max_depth=1)
    assert "io.py" not in {e.name for e in shallow}


def test_file_entry_attributes(tree: Path):
    fe = next(e for e in pyofiles.walk(str(tree)) if e.is_file and e.name == "readme.txt")
    assert isinstance(fe.path, str)
    assert fe.is_file is True
    assert fe.is_dir is False
    assert fe.size > 0
    assert fe.extension == "txt"


def test_walk_name_and_size_filters(tree: Path):
    assert {"main.py", "main.pyc"} <= file_names(pyofiles.walk(str(tree), names=["main"]))
    assert file_names(pyofiles.walk(str(tree), min_size_mb=1)) == {"large_file.bin"}
    assert "large_file.bin" not in file_names(pyofiles.walk(str(tree), max_size_mb=1))
    # with any filter active, only files are returned; without filters, dirs too
    assert all(e.is_file for e in pyofiles.walk(str(tree), extensions=[".py"]))
    assert any(e.is_dir for e in pyofiles.walk(str(tree)))
    # threads parameter accepted
    assert len(pyofiles.walk(str(tree), extensions=[".py"], threads=2)) == 3


# ---------------------------------------------------------------------------
# find
# ---------------------------------------------------------------------------


def test_find_by_name(tree: Path):
    assert "report_2024.pdf" in file_names(pyofiles.find(str(tree), names=["report"]))
    both = pyofiles.find(str(tree), names=["report", "invoice"])
    assert {"report_2024.pdf", "invoice_march.pdf"} <= file_names(both)


def test_find_by_extension(tree: Path):
    found = pyofiles.find(str(tree), extensions=[".csv", ".json"])
    assert file_names(found) == {"output.csv", "output.json"}


def test_find_size_filters(tree: Path):
    assert "large_file.bin" in file_names(
        pyofiles.find(str(tree), extensions=[".bin"], min_size_mb=1))
    assert "large_file.bin" not in file_names(
        pyofiles.find(str(tree), extensions=[".bin", ".txt", ".py"], max_size_mb=1))
    # size filter alone is allowed
    assert "large_file.bin" in file_names(pyofiles.find(str(tree), min_size_mb=1))


def test_find_combined_filters(tree: Path):
    found = pyofiles.find(str(tree), names=["output"], extensions=[".json"])
    assert file_names(found) == {"output.json"}


def test_find_limit(tree: Path):
    assert len(pyofiles.find(str(tree), extensions=[".py"], limit=2)) == 2
    assert len(pyofiles.find(str(tree), extensions=[".py"], limit=100)) == 3
    assert len(pyofiles.find(str(tree), extensions=[".py"], limit=0)) == 0
    assert len(pyofiles.find(str(tree), extensions=[".py"], threads=2)) == 3


def test_find_requires_a_filter(tree: Path):
    with pytest.raises(ValueError, match="at least"):
        pyofiles.find(str(tree))


# ---------------------------------------------------------------------------
# list_dir
# ---------------------------------------------------------------------------


def test_list_dir_basic(tree: Path):
    entries = pyofiles.list_dir(str(tree))
    names = {e.name for e in entries}
    assert "readme.txt" in names
    assert "src" in names
    assert "io.py" not in names  # non-recursive

    src_names = {e.name for e in pyofiles.list_dir(str(tree / "src"))}
    assert "main.py" in src_names


def test_list_dir_sorted(tree: Path):
    names = [e.name for e in pyofiles.list_dir(str(tree))]
    assert names == sorted(names)


def test_list_dir_filters(tree: Path):
    txt = pyofiles.list_dir(str(tree), extensions=[".txt"])
    assert "readme.txt" in file_names(txt)
    assert "report_2024.pdf" not in file_names(txt)

    visible = {e.name for e in pyofiles.list_dir(str(tree), skip_hidden=True)}
    assert ".hidden_file.txt" not in visible
    everything = {e.name for e in pyofiles.list_dir(str(tree))}
    assert ".hidden_file.txt" in everything

    reports = pyofiles.list_dir(str(tree), names=["report"])
    assert "report_2024.pdf" in file_names(reports)


# ---------------------------------------------------------------------------
# index
# ---------------------------------------------------------------------------


def test_index_basic(tree: Path):
    idx = pyofiles.index(str(tree / "src"), extensions=[".py", ".pyi", ".pyc"])
    assert isinstance(idx, dict)
    assert {"main", "io"} <= set(idx)
    assert {".py", ".pyc"} <= set(idx["main"])
    assert {".py", ".pyi"} <= set(idx["io"])


def test_index_filters(tree: Path):
    idx = pyofiles.index(str(tree / "src"), extensions=[".py", ".pyi"], max_depth=1)
    assert "main" in idx
    assert "io" not in idx

    cutoff = time.time() - 60
    recent = pyofiles.index(str(tree / "src"), extensions=[".py"], modified_after=cutoff)
    assert "main" in recent
    old = pyofiles.index(str(tree / "src"), extensions=[".py"], modified_before=cutoff)
    assert old == {}

    named = pyofiles.index(
        str(tree / "src"), extensions=[".py", ".pyi", ".pyc"], names=["io"])
    assert "io" in named and "main" not in named


def test_index_collisions_are_deterministic(tree: Path):
    (tree / "data" / "dup.py").write_text("# data copy\n")
    (tree / "src" / "dup.py").write_text("# src copy\n")

    expected = str(tree / "data" / "dup.py")
    for run in range(5):
        got = pyofiles.index(str(tree), extensions=[".py"])["dup"][".py"]
        assert got == expected, f"run {run}: got {got}"


# ---------------------------------------------------------------------------
# glob
# ---------------------------------------------------------------------------


def test_glob_patterns(tree: Path):
    def basenames(paths):
        return {Path(p).name for p in paths}

    assert {"main.py", "utils.py", "io.py"} <= basenames(
        pyofiles.glob(str(tree), "**/*.py"))
    assert "readme.txt" in basenames(pyofiles.glob(str(tree), "*.txt"))
    assert {"report_2024.pdf", "invoice_march.pdf"} <= basenames(
        pyofiles.glob(str(tree), "**/*.pdf"))


def test_glob_filters(tree: Path):
    deep = {Path(p).name for p in pyofiles.glob(str(tree), "**/*.py", max_depth=2)}
    assert "io.py" not in deep

    cutoff = time.time() - 60
    recent = {Path(p).name for p in pyofiles.glob(str(tree), "**/*.py", modified_after=cutoff)}
    assert "main.py" in recent
    assert pyofiles.glob(str(tree), "**/*.py", modified_before=cutoff) == []

    large = {Path(p).name for p in pyofiles.glob(str(tree), "**/*", min_size_mb=1)}
    assert "large_file.bin" in large


# ---------------------------------------------------------------------------
# disk_usage
# ---------------------------------------------------------------------------


def test_disk_usage_basic(tree: Path):
    usage = pyofiles.disk_usage(str(tree), depth=2, top=10)

    assert usage.total_size > 0
    assert usage.total_files > 0
    assert usage.total_size_mb > 1
    assert isinstance(usage.total_size_gb, float)

    assert usage.entries
    first = usage.entries[0]
    assert isinstance(first.path, str)
    assert isinstance(first.size, int) and first.size >= 0
    assert isinstance(first.file_count, int)
    assert isinstance(first.size_mb, float)
    assert isinstance(first.size_gb, float)

    # sorted by size, largest first, truncated to top
    sizes = [e.size for e in usage.entries]
    assert sizes == sorted(sizes, reverse=True)
    assert len(sizes) <= 10


def test_disk_usage_depth(tree: Path):
    depth1 = pyofiles.disk_usage(str(tree), depth=1)
    depth2 = pyofiles.disk_usage(str(tree), depth=2)
    assert len(depth2.entries) >= len(depth1.entries)


def test_disk_usage_filters(tree: Path):
    py_only = pyofiles.disk_usage(str(tree), extensions=[".py"])
    assert py_only.total_files > 0
    assert py_only.total_size < 500_000  # no large_file.bin counted

    large = pyofiles.disk_usage(str(tree), names=["large"])
    assert large.total_files == 1
    assert large.total_size >= 1_400_000

    cutoff = time.time() - 60
    assert pyofiles.disk_usage(str(tree), modified_after=cutoff).total_files > 0
    assert pyofiles.disk_usage(str(tree), modified_before=cutoff).total_files == 0
    assert pyofiles.disk_usage(str(tree), min_size_mb=1).total_files == 1


# ---------------------------------------------------------------------------
# time filters
# ---------------------------------------------------------------------------


def test_time_filters(tree: Path):
    root = str(tree)
    one_minute_ago = time.time() - 60

    fe = next(e for e in pyofiles.walk(root) if e.is_file)
    assert isinstance(fe.modified, float)
    assert fe.modified > one_minute_ago
    assert fe.created is None or isinstance(fe.created, float)

    recent = [e for e in pyofiles.walk(root, modified_after=one_minute_ago) if e.is_file]
    assert recent
    old = [e for e in pyofiles.walk(root, modified_before=one_minute_ago) if e.is_file]
    assert old == []

    found = {e.name for e in pyofiles.find(root, extensions=[".py"], modified_after=one_minute_ago)}
    assert "main.py" in found
    # time filter alone is allowed
    assert pyofiles.find(root, modified_after=one_minute_ago)
    found_old = pyofiles.find(root, extensions=[".py"], modified_before=one_minute_ago)
    assert found_old == []


# ---------------------------------------------------------------------------
# CLI: time parsing
# ---------------------------------------------------------------------------


class TestParseTime:
    def test_relative_durations(self):
        for value, seconds in [("7d", 7 * 86400), ("24h", 86400),
                               ("30m", 1800), ("1w", 604800), ("3600s", 3600)]:
            assert abs(parse_time(value) - (time.time() - seconds)) < 5, value

    def test_relative_units_are_case_insensitive(self):
        assert abs(parse_time("24H") - (time.time() - 86400)) < 5
        assert abs(parse_time("7D") - (time.time() - 7 * 86400)) < 5

    def test_fractional_duration(self):
        assert abs(parse_time("1.5d") - (time.time() - 129600)) < 5

    def test_iso_date_and_datetime(self):
        assert parse_time("2024-03-15") == datetime(2024, 3, 15).timestamp()
        assert parse_time("2024-03-15T10:30") == \
            datetime(2024, 3, 15, 10, 30).timestamp()
        assert parse_time("2024-03-15T10:30:00") == \
            datetime(2024, 3, 15, 10, 30).timestamp()

    def test_iso_with_utc_offset(self):
        expected = datetime(2024, 3, 15, 8, 30, tzinfo=timezone.utc).timestamp()
        assert parse_time("2024-03-15T10:30:00+02:00") == expected
        assert parse_time("2024-03-15T10:30:00-02:00") != expected

    def test_iso_with_z_suffix(self):
        expected = datetime(2024, 3, 15, 10, 30, tzinfo=timezone.utc).timestamp()
        assert parse_time("2024-03-15T10:30:00Z") == expected
        assert parse_time("2024-03-15T10:30:00z") == expected
        # date-only + Z means UTC midnight, not local midnight
        assert parse_time("2024-03-15Z") == \
            datetime(2024, 3, 15, tzinfo=timezone.utc).timestamp()

    def test_non_zero_padded_dates(self):
        # strptime leniency kept from the original parser
        assert parse_time("2024-3-5") == datetime(2024, 3, 5).timestamp()
        assert parse_time("2024-03-15T10:5") == \
            datetime(2024, 3, 15, 10, 5).timestamp()

    def test_unix_timestamp(self):
        assert parse_time("1709251200") == 1709251200.0

    def test_digit_strings_always_mean_timestamps(self):
        # fromisoformat grew more lenient in newer Pythons (basic-format
        # "20240101" parses as a date on 3.11+ but not 3.9/3.10); digit-only
        # values must mean unix seconds on every version.
        assert parse_time("20240101") == 20240101.0
        assert parse_time("2024") == 2024.0

    @pytest.mark.parametrize("value", ["nan", "inf", "-inf", "infinity"])
    def test_non_finite_rejected(self, value):
        with pytest.raises(argparse.ArgumentTypeError):
            parse_time(value)

    @pytest.mark.parametrize("value", ["", "garbage", "2024-13-45", "7x",
                                       "1_000d", "１２d", "²d"])
    def test_invalid_values(self, value):
        with pytest.raises(argparse.ArgumentTypeError):
            parse_time(value)


# ---------------------------------------------------------------------------
# CLI: numeric argument validation
# ---------------------------------------------------------------------------

BAD_NUMERIC_ARGS = [
    ["walk", ".", "--max-depth", "-1"],
    ["walk", ".", "--max-depth", "abc"],
    ["walk", ".", "--max-depth", "1_000"],
    ["walk", ".", "--threads", "0"],
    ["walk", ".", "--threads", "-4"],
    ["find", ".", "--max-depth", "-3"],
    ["find", ".", "--limit", "-5"],
    ["find", ".", "--threads", "0"],
    ["glob", ".", "--max-depth", "-1"],
    ["glob", ".", "--threads", "-2"],
    ["index", ".", "--ext", ".py", "--max-depth", "-1"],
    ["du", ".", "--depth", "-2"],
    ["du", ".", "--top", "-1"],
    ["du", ".", "--top", "abc"],
    ["du", ".", "--threads", "x"],
]

# (argv, attribute, expected parsed value) -- exact equality so a dropped
# validator or wrong default cannot slip through.
GOOD_NUMERIC_ARGS = [
    (["walk", ".", "--max-depth", "0"], "max_depth", 0),
    (["walk", ".", "--threads", "1"], "threads", 1),
    (["find", ".", "--limit", "0"], "limit", 0),
    (["find", ".", "--limit", "10"], "limit", 10),
    (["glob", ".", "--max-depth", "2"], "max_depth", 2),
    (["index", ".", "--ext", ".py", "--max-depth", "3"], "max_depth", 3),
    (["du", ".", "--depth", "0"], "depth", 0),
    (["du", ".", "--top", "0"], "top", 0),
    (["du", ".", "--depth", "4"], "depth", 4),
    (["du", ".", "--top", "5"], "top", 5),
]


@pytest.mark.parametrize("argv", BAD_NUMERIC_ARGS)
def test_cli_rejects_bad_numbers(argv):
    with pytest.raises(SystemExit) as excinfo:
        build_parser().parse_args(argv)
    assert excinfo.value.code == 2


@pytest.mark.parametrize(("argv", "attr", "expected"), GOOD_NUMERIC_ARGS)
def test_cli_accepts_boundary_numbers(argv, attr, expected):
    args = build_parser().parse_args(argv)  # must not raise SystemExit
    assert getattr(args, attr) == expected


# ---------------------------------------------------------------------------
# CLI: output formatting helpers
# ---------------------------------------------------------------------------


def test_format_size():
    assert format_size(512) == "512B"
    assert format_size(2048) == "2.0KB"
    assert format_size(5 * 1024 * 1024) == "5.0MB"
    assert format_size(3 * 1024 * 1024 * 1024) == "3.00GB"


def test_terminal_output_escaping(tree: Path):
    malicious_name = "forged\x1b]2;title\x07\rline.txt"
    entries = [SimpleNamespace(
        path=str(tree / malicious_name),
        name=malicious_name,
        is_file=True,
        is_dir=False,
        size=1,
        extension="txt",
        modified=None,
        created=None,
    )]
    captured = io.StringIO()
    with contextlib.redirect_stdout(captured):
        print_entries(entries)
    output = captured.getvalue()

    assert "\x1b" not in output
    assert "\x07" not in output
    assert "\r" not in output
    assert r"\x1b]2;title\a\rline.txt" in output
    # ordinary unicode remains unchanged
    assert escape_terminal_controls("café/資料.txt") == "café/資料.txt"


# ---------------------------------------------------------------------------
# MFT fast path
# ---------------------------------------------------------------------------


def _is_windows_admin():
    """True when running elevated on Windows (e.g. GitHub windows runners)."""
    if sys.platform != "win32":
        return False
    try:
        return ctypes.windll.shell32.IsUserAnAdmin() != 0
    except Exception:
        return False


MFT_CALLS = [
    ("disk_usage", lambda root: pyofiles.disk_usage(root, mft=True)),
    ("walk", lambda root: pyofiles.walk(root, mft=True)),
    ("find", lambda root: pyofiles.find(root, names=["readme"], mft=True)),
]


@pytest.mark.skipif(sys.platform == "win32", reason="Windows exposes the MFT fast path")
@pytest.mark.parametrize(("label", "call"), MFT_CALLS)
def test_mft_unavailable_off_windows(label, call, tree):
    with pytest.raises(ValueError, match="only available on Windows"):
        call(str(tree))


@pytest.mark.skipif(sys.platform != "win32", reason="UNC rejection is Windows-specific")
def test_mft_rejects_unc_paths():
    with pytest.raises(OSError) as excinfo:
        pyofiles.find(r"\\localhost\nonexistent\share", names=["x"], mft=True)
    message = str(excinfo.value)
    assert "administrator" in message
    assert "NTFS" in message


@pytest.mark.skipif(sys.platform != "win32", reason="raw volume access is Windows-specific")
@pytest.mark.parametrize(("label", "call"), MFT_CALLS)
def test_mft_requires_admin_without_elevation(label, call, tree):
    if _is_windows_admin():
        pytest.skip("running elevated; CI covers the real scan separately")
    with pytest.raises(OSError, match="administrator"):
        call(str(tree))


if __name__ == "__main__":
    sys.exit(pytest.main([__file__, "-q"]))
