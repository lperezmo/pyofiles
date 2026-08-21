"""pyofiles CLI - fast, Rust-powered file operations from the command line."""

from __future__ import annotations

import argparse
import json
import math
import os
import sys
import time
from datetime import datetime, timezone

import pyofiles


# ---------------------------------------------------------------------------
# Time parsing helpers
# ---------------------------------------------------------------------------

_DURATION_UNITS = {"s": 1, "m": 60, "h": 3600, "d": 86400, "w": 604800}


def parse_time(value: str) -> float:
    """Parse a time value into a unix timestamp.

    Accepts:
      - Relative durations (case-insensitive): "7d", "24H", "30m", "1w", "3600s"
      - ISO dates/datetimes, including UTC offsets: "2024-03-15",
        "2024-03-15T10:30:00", "2024-03-15T10:30:00+02:00", "...Z"
      - Raw unix timestamps: "1709251200"
    """
    if not value:
        raise argparse.ArgumentTypeError("empty time value")

    # Relative duration (e.g. "7d", "24H"); the unit is case-insensitive
    # and the amount must be plain ASCII digits, optionally fractional.
    unit = _DURATION_UNITS.get(value[-1].lower())
    prefix = value[:-1]
    if unit is not None and prefix.isascii() and prefix.replace(".", "", 1).isdigit():
        return time.time() - float(prefix) * unit

    # ISO date/datetime. fromisoformat covers plain dates, missing seconds,
    # fractional seconds, and UTC offsets on every supported Python, but it
    # grew more lenient in newer versions, so bare digit strings skip it:
    # they must always mean a unix timestamp, on every Python version. A
    # trailing "Z" is only understood by 3.11+, so normalize it here.
    if not value.isascii() or not value.isdigit():
        normalized = value[:-1] + "+00:00" if value[-1] in "Zz" else value
        parsed = None
        try:
            parsed = datetime.fromisoformat(normalized)
        except ValueError:
            # Keep accepting non-zero-padded dates ("2024-3-5"), which
            # strptime allowed before fromisoformat became the parser.
            for fmt in ("%Y-%m-%d", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"):
                try:
                    parsed = datetime.strptime(value, fmt)
                    break
                except ValueError:
                    continue
        if parsed is not None:
            # fromisoformat drops the offset for date-only values; an
            # explicit trailing "Z" still means UTC midnight there.
            if parsed.tzinfo is None and value[-1] in "Zz":
                parsed = parsed.replace(tzinfo=timezone.utc)
            return parsed.timestamp()

    # Raw unix timestamp. NaN compares false against everything and would
    # silently disable the filter, so non-finite values are rejected.
    try:
        ts = float(value)
    except ValueError:
        pass
    else:
        if math.isfinite(ts):
            return ts
        raise argparse.ArgumentTypeError(f"time value '{value}' must be finite")

    raise argparse.ArgumentTypeError(
        f"cannot parse time '{value}' - use relative (7d, 24h), ISO date (2024-03-15), or unix timestamp"
    )


def finite_float(value: str) -> float:
    """argparse type: a finite float (NaN would silently disable size bounds)."""
    try:
        fvalue = float(value)
    except ValueError:
        raise argparse.ArgumentTypeError(f"'{value}' is not a number")
    if not math.isfinite(fvalue):
        raise argparse.ArgumentTypeError(f"'{value}' must be finite")
    return fvalue


def _strict_int(value: str) -> int:
    """int() without its leniency for underscores and surrounding whitespace."""
    digits = value[1:] if value.startswith("-") else value
    if not (digits.isascii() and digits.isdigit()):
        raise argparse.ArgumentTypeError(f"'{value}' is not an integer")
    return int(value)


def non_negative_int(value: str) -> int:
    """argparse type: an integer >= 0."""
    ivalue = _strict_int(value)
    if ivalue < 0:
        raise argparse.ArgumentTypeError(f"'{value}' must be >= 0")
    return ivalue


def positive_int(value: str) -> int:
    """argparse type: an integer >= 1."""
    ivalue = _strict_int(value)
    if ivalue < 1:
        raise argparse.ArgumentTypeError(f"'{value}' must be >= 1")
    return ivalue


# ---------------------------------------------------------------------------
# Output formatting
# ---------------------------------------------------------------------------

def format_size(size_bytes: int) -> str:
    """Human-readable file size."""
    if size_bytes < 1024:
        return f"{size_bytes}B"
    elif size_bytes < 1024 * 1024:
        return f"{size_bytes / 1024:.1f}KB"
    elif size_bytes < 1024 * 1024 * 1024:
        return f"{size_bytes / (1024 * 1024):.1f}MB"
    else:
        return f"{size_bytes / (1024 * 1024 * 1024):.2f}GB"


def format_time(ts: float | None) -> str:
    """Format a unix timestamp for display."""
    if ts is None:
        return "-"
    return datetime.fromtimestamp(ts).strftime("%Y-%m-%d %H:%M")


def json_indent() -> int | None:
    """Pretty-print JSON on a terminal, compact when piped."""
    return 2 if sys.stdout.isatty() else None


def escape_terminal_controls(value: str) -> str:
    """Render terminal control characters as inert, visible escapes."""
    named = {
        "\a": r"\a",
        "\b": r"\b",
        "\t": r"\t",
        "\n": r"\n",
        "\v": r"\v",
        "\f": r"\f",
        "\r": r"\r",
    }
    escaped = []
    for char in value:
        codepoint = ord(char)
        if char in named:
            escaped.append(named[char])
        elif codepoint <= 0x1F or 0x7F <= codepoint <= 0x9F:
            escaped.append(f"\\x{codepoint:02x}")
        else:
            escaped.append(char)
    return "".join(escaped)


def print_lines(lines):
    """Write terminal-safe lines in a single buffered call."""
    if lines:
        sys.stdout.write("\n".join(escape_terminal_controls(line) for line in lines) + "\n")


def print_entries(entries, as_json: bool = False, long: bool = False):
    """Print a list of FileEntry objects."""
    if as_json:
        data = [
            {
                "path": e.path,
                "name": e.name,
                "is_file": e.is_file,
                "is_dir": e.is_dir,
                "size": e.size,
                "extension": e.extension,
                "modified": e.modified,
                "created": e.created,
            }
            for e in entries
        ]
        print(json.dumps(data, indent=json_indent()))
    elif long:
        lines = []
        for e in entries:
            kind = "f" if e.is_file else "d"
            size = format_size(e.size) if e.is_file else "-"
            mod_time = format_time(e.modified)
            lines.append(f"{kind}  {size:>8s}  {mod_time}  {e.path}")
        print_lines(lines)
    else:
        print_lines([e.path for e in entries])


def print_disk_usage(usage, as_json: bool = False):
    """Print a DiskUsage result."""
    if as_json:
        data = {
            "total_size": usage.total_size,
            "total_size_mb": usage.total_size_mb,
            "total_size_gb": usage.total_size_gb,
            "total_files": usage.total_files,
            "entries": [
                {
                    "path": e.path,
                    "size": e.size,
                    "size_mb": e.size_mb,
                    "file_count": e.file_count,
                }
                for e in usage.entries
            ],
        }
        print(json.dumps(data, indent=json_indent()))
    else:
        lines = [
            f"{format_size(e.size):>10s}  {e.file_count:>6d} files  {e.path}"
            for e in usage.entries
        ]
        lines.append("")
        lines.append(f"Total: {format_size(usage.total_size)} in {usage.total_files} files")
        print_lines(lines)


# ---------------------------------------------------------------------------
# Shared argument helpers
# ---------------------------------------------------------------------------

def add_time_args(parser: argparse.ArgumentParser):
    """Add time filter arguments to a subparser."""
    parser.add_argument("--modified-after", type=parse_time, default=None, metavar="TIME",
                        help="only files modified after TIME (e.g. 7d, 2024-01-15, 1709251200)")
    parser.add_argument("--modified-before", type=parse_time, default=None, metavar="TIME",
                        help="only files modified before TIME")
    parser.add_argument("--created-after", type=parse_time, default=None, metavar="TIME",
                        help="only files created after TIME")
    parser.add_argument("--created-before", type=parse_time, default=None, metavar="TIME",
                        help="only files created before TIME")


def add_output_args(parser: argparse.ArgumentParser):
    """Add output format arguments to a subparser."""
    parser.add_argument("--json", dest="as_json", action="store_true", help="output as JSON")
    parser.add_argument("-l", "--long", action="store_true", help="long format (type, size, modified, path)")


def add_name_args(parser: argparse.ArgumentParser):
    """Add name substring filter arguments to a subparser."""
    parser.add_argument("--names", nargs="+", default=None,
                        help="name substrings to match (OR logic)")


def add_size_args(parser: argparse.ArgumentParser):
    """Add size filter arguments to a subparser."""
    parser.add_argument("--min-size", type=finite_float, default=None, help="min file size in MB")
    parser.add_argument("--max-size", type=finite_float, default=None, help="max file size in MB")


def add_threads_arg(parser: argparse.ArgumentParser):
    """Add walker thread count argument to a subparser."""
    parser.add_argument("--threads", type=positive_int, default=None, metavar="N",
                        help="number of walker threads (default: number of CPUs)")


def add_mft_arg(parser: argparse.ArgumentParser):
    """Add the NTFS MFT fast path flag to a subparser."""
    parser.add_argument("--mft", action="store_true",
                        help="scan the NTFS Master File Table directly "
                             "(Windows only; needs admin and a local NTFS volume)")


# ---------------------------------------------------------------------------
# Subcommands
# ---------------------------------------------------------------------------

def cmd_walk(args):
    entries = pyofiles.walk(
        args.directory,
        extensions=args.ext,
        skip_hidden=args.skip_hidden,
        max_depth=args.max_depth,
        names=args.names,
        min_size_mb=args.min_size,
        max_size_mb=args.max_size,
        modified_after=args.modified_after,
        modified_before=args.modified_before,
        created_after=args.created_after,
        created_before=args.created_before,
        threads=args.threads,
        mft=args.mft,
    )
    print_entries(entries, as_json=args.as_json, long=args.long)


def cmd_find(args):
    entries = pyofiles.find(
        args.directory,
        names=args.names,
        extensions=args.ext,
        min_size_mb=args.min_size,
        max_size_mb=args.max_size,
        skip_hidden=args.skip_hidden,
        max_depth=args.max_depth,
        modified_after=args.modified_after,
        modified_before=args.modified_before,
        created_after=args.created_after,
        created_before=args.created_before,
        limit=args.limit,
        threads=args.threads,
        mft=args.mft,
    )
    print_entries(entries, as_json=args.as_json, long=args.long)


def cmd_ls(args):
    entries = pyofiles.list_dir(
        args.directory,
        extensions=args.ext,
        names=args.names,
        min_size_mb=args.min_size,
        max_size_mb=args.max_size,
        skip_hidden=args.skip_hidden,
        modified_after=args.modified_after,
        modified_before=args.modified_before,
        created_after=args.created_after,
        created_before=args.created_before,
    )
    print_entries(entries, as_json=args.as_json, long=args.long)


def cmd_glob(args):
    paths = pyofiles.glob(
        args.directory,
        args.pattern,
        skip_hidden=args.skip_hidden,
        max_depth=args.max_depth,
        min_size_mb=args.min_size,
        max_size_mb=args.max_size,
        modified_after=args.modified_after,
        modified_before=args.modified_before,
        created_after=args.created_after,
        created_before=args.created_before,
        threads=args.threads,
    )
    if args.as_json:
        print(json.dumps(paths, indent=json_indent()))
    else:
        print_lines(paths)


def cmd_index(args):
    idx = pyofiles.index(
        args.directory,
        extensions=args.ext,
        skip_hidden=args.skip_hidden,
        max_depth=args.max_depth,
        names=args.names,
        min_size_mb=args.min_size,
        max_size_mb=args.max_size,
        modified_after=args.modified_after,
        modified_before=args.modified_before,
        created_after=args.created_after,
        created_before=args.created_before,
        threads=args.threads,
    )
    if args.as_json:
        print(json.dumps(idx, indent=json_indent()))
    else:
        lines = []
        for stem, exts in sorted(idx.items()):
            ext_list = ", ".join(f"{k} -> {os.path.basename(v)}" for k, v in sorted(exts.items()))
            lines.append(f"  {stem}: {ext_list}")
        print_lines(lines)


def cmd_du(args):
    usage = pyofiles.disk_usage(
        args.directory,
        depth=args.depth,
        top=args.top,
        skip_hidden=args.skip_hidden,
        extensions=args.ext,
        names=args.names,
        min_size_mb=args.min_size,
        max_size_mb=args.max_size,
        modified_after=args.modified_after,
        modified_before=args.modified_before,
        created_after=args.created_after,
        created_before=args.created_before,
        threads=args.threads,
        mft=args.mft,
    )
    print_disk_usage(usage, as_json=args.as_json)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="pyofiles",
        description="Fast, Rust-powered file operations.",
    )
    parser.add_argument(
        "-v", "--version", action="version",
        version=f"%(prog)s {pyofiles.__version__}",
    )
    sub = parser.add_subparsers(dest="command", required=True)

    # -- walk --
    p_walk = sub.add_parser("walk", help="recursively walk a directory")
    p_walk.add_argument("directory", nargs="?", default=".", help="directory to walk (default: .)")
    p_walk.add_argument("--ext", nargs="+", default=None, help="filter by extensions (e.g. .py .rs)")
    p_walk.add_argument("--skip-hidden", action="store_true", help="skip hidden files/dirs")
    p_walk.add_argument("--max-depth", type=non_negative_int, default=None, metavar="N",
                        help="max recursion depth")
    add_name_args(p_walk)
    add_size_args(p_walk)
    add_time_args(p_walk)
    add_threads_arg(p_walk)
    add_mft_arg(p_walk)
    add_output_args(p_walk)
    p_walk.set_defaults(func=cmd_walk)

    # -- find --
    p_find = sub.add_parser("find", help="find files by name, extension, size, or time")
    p_find.add_argument("directory", nargs="?", default=".", help="directory to search (default: .)")
    p_find.add_argument("--ext", nargs="+", default=None, help="filter by extensions")
    p_find.add_argument("--skip-hidden", action="store_true", help="skip hidden files/dirs")
    p_find.add_argument("--max-depth", type=non_negative_int, default=None, metavar="N",
                        help="max recursion depth")
    p_find.add_argument("--limit", type=non_negative_int, default=None, metavar="N",
                        help="stop after N matches")
    add_name_args(p_find)
    add_size_args(p_find)
    add_time_args(p_find)
    add_threads_arg(p_find)
    add_mft_arg(p_find)
    add_output_args(p_find)
    p_find.set_defaults(func=cmd_find)

    # -- ls --
    p_ls = sub.add_parser("ls", help="list directory contents (non-recursive)")
    p_ls.add_argument("directory", nargs="?", default=".", help="directory to list (default: .)")
    p_ls.add_argument("--ext", nargs="+", default=None, help="filter by extensions")
    p_ls.add_argument("--skip-hidden", action="store_true", help="skip hidden files/dirs")
    add_name_args(p_ls)
    add_size_args(p_ls)
    add_time_args(p_ls)
    add_output_args(p_ls)
    p_ls.set_defaults(func=cmd_ls)

    # -- glob --
    p_glob = sub.add_parser("glob", help="match files with a glob pattern")
    p_glob.add_argument("directory", nargs="?", default=".", help="root directory (default: .)")
    p_glob.add_argument("pattern", help="glob pattern (e.g. '**/*.py')")
    p_glob.add_argument("--skip-hidden", action="store_true", help="skip hidden files")
    p_glob.add_argument("--max-depth", type=non_negative_int, default=None, metavar="N",
                        help="max recursion depth")
    add_size_args(p_glob)
    add_time_args(p_glob)
    add_threads_arg(p_glob)
    p_glob.add_argument("--json", dest="as_json", action="store_true", help="output as JSON")
    p_glob.set_defaults(func=cmd_glob)

    # -- index --
    p_index = sub.add_parser("index", help="index files by stem and extension")
    p_index.add_argument("directory", nargs="?", default=".", help="directory to index (default: .)")
    p_index.add_argument("--ext", nargs="+", required=True, help="extensions to index (e.g. .py .pyi .pyc)")
    p_index.add_argument("--skip-hidden", action="store_true", help="skip hidden files")
    p_index.add_argument("--max-depth", type=non_negative_int, default=None, metavar="N",
                         help="max recursion depth")
    add_name_args(p_index)
    add_size_args(p_index)
    add_time_args(p_index)
    add_threads_arg(p_index)
    p_index.add_argument("--json", dest="as_json", action="store_true", help="output as JSON")
    p_index.set_defaults(func=cmd_index)

    # -- du --
    p_du = sub.add_parser("du", help="disk usage analysis")
    p_du.add_argument("directory", nargs="?", default=".", help="directory to analyze (default: .)")
    p_du.add_argument("--depth", type=non_negative_int, default=1, metavar="N",
                      help="directory depth for grouping, 0 for totals only (default: 1)")
    p_du.add_argument("--top", type=non_negative_int, default=20, metavar="N",
                      help="number of top entries, 0 to omit them (default: 20)")
    p_du.add_argument("--skip-hidden", action="store_true", help="skip hidden files/dirs")
    p_du.add_argument("--ext", nargs="+", default=None, help="filter by extensions")
    add_name_args(p_du)
    add_size_args(p_du)
    add_time_args(p_du)
    add_threads_arg(p_du)
    add_mft_arg(p_du)
    p_du.add_argument("--json", dest="as_json", action="store_true", help="output as JSON")
    p_du.set_defaults(func=cmd_du)

    return parser


def main():
    parser = build_parser()
    args = parser.parse_args()
    try:
        args.func(args)
        sys.stdout.flush()
    except BrokenPipeError:
        # Downstream consumer (e.g. `| head`) closed the pipe: exit quietly.
        # Point stdout at devnull so the interpreter's final flush does not
        # raise a second error.
        try:
            devnull = os.open(os.devnull, os.O_WRONLY)
            os.dup2(devnull, sys.stdout.fileno())
        except OSError:
            pass
        sys.exit(141)
    except KeyboardInterrupt:
        sys.exit(130)
    except Exception as e:
        print(f"error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
