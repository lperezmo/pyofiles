"""pyofiles CLI — fast, Rust-powered file operations from the command line."""

import argparse
import json
import os
import sys
import time
from datetime import datetime

import pyofiles


# ---------------------------------------------------------------------------
# Time parsing helpers
# ---------------------------------------------------------------------------

_DURATION_UNITS = {"s": 1, "m": 60, "h": 3600, "d": 86400, "w": 604800}


def parse_time(value: str) -> float:
    """Parse a time value into a unix timestamp.

    Accepts:
      - Relative durations: "7d", "24h", "30m", "1w", "3600s"
      - ISO dates: "2024-03-15", "2024-03-15T10:30:00"
      - Raw unix timestamps: "1709251200"
    """
    if not value:
        raise argparse.ArgumentTypeError("empty time value")

    # Relative duration (e.g. "7d", "24h")
    if value[-1] in _DURATION_UNITS and value[:-1].replace(".", "", 1).isdigit():
        seconds = float(value[:-1]) * _DURATION_UNITS[value[-1]]
        return time.time() - seconds

    # ISO date/datetime
    for fmt in ("%Y-%m-%d", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"):
        try:
            return datetime.strptime(value, fmt).timestamp()
        except ValueError:
            continue

    # Raw unix timestamp
    try:
        return float(value)
    except ValueError:
        pass

    raise argparse.ArgumentTypeError(
        f"cannot parse time '{value}' — use relative (7d, 24h), ISO date (2024-03-15), or unix timestamp"
    )


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
        print(json.dumps(data, indent=2))
    elif long:
        for e in entries:
            kind = "f" if e.is_file else "d"
            size = format_size(e.size) if e.is_file else "-"
            mod_time = format_time(e.modified)
            print(f"{kind}  {size:>8s}  {mod_time}  {e.path}")
    else:
        for e in entries:
            print(e.path)


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
        print(json.dumps(data, indent=2))
    else:
        for e in usage.entries:
            print(f"{format_size(e.size):>10s}  {e.file_count:>6d} files  {e.path}")
        print(f"\nTotal: {format_size(usage.total_size)} in {usage.total_files} files")


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
    parser.add_argument("--min-size", type=float, default=None, help="min file size in MB")
    parser.add_argument("--max-size", type=float, default=None, help="max file size in MB")


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
    )
    if args.as_json:
        print(json.dumps(paths, indent=2))
    else:
        for p in paths:
            print(p)


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
    )
    if args.as_json:
        print(json.dumps(idx, indent=2))
    else:
        for stem, exts in sorted(idx.items()):
            ext_list = ", ".join(f"{k} -> {os.path.basename(v)}" for k, v in sorted(exts.items()))
            print(f"  {stem}: {ext_list}")


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
    sub = parser.add_subparsers(dest="command", required=True)

    # -- walk --
    p_walk = sub.add_parser("walk", help="recursively walk a directory")
    p_walk.add_argument("directory", nargs="?", default=".", help="directory to walk (default: .)")
    p_walk.add_argument("--ext", nargs="+", default=None, help="filter by extensions (e.g. .py .rs)")
    p_walk.add_argument("--skip-hidden", action="store_true", help="skip hidden files/dirs")
    p_walk.add_argument("--max-depth", type=int, default=None, help="max recursion depth")
    add_name_args(p_walk)
    add_size_args(p_walk)
    add_time_args(p_walk)
    add_output_args(p_walk)
    p_walk.set_defaults(func=cmd_walk)

    # -- find --
    p_find = sub.add_parser("find", help="find files by name, extension, size, or time")
    p_find.add_argument("directory", nargs="?", default=".", help="directory to search (default: .)")
    p_find.add_argument("--ext", nargs="+", default=None, help="filter by extensions")
    p_find.add_argument("--skip-hidden", action="store_true", help="skip hidden files/dirs")
    p_find.add_argument("--max-depth", type=int, default=None, help="max recursion depth")
    add_name_args(p_find)
    add_size_args(p_find)
    add_time_args(p_find)
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
    p_glob.add_argument("--max-depth", type=int, default=None, help="max recursion depth")
    add_size_args(p_glob)
    add_time_args(p_glob)
    p_glob.add_argument("--json", dest="as_json", action="store_true", help="output as JSON")
    p_glob.set_defaults(func=cmd_glob)

    # -- index --
    p_index = sub.add_parser("index", help="index files by stem and extension")
    p_index.add_argument("directory", nargs="?", default=".", help="directory to index (default: .)")
    p_index.add_argument("--ext", nargs="+", required=True, help="extensions to index (e.g. .py .pyi .pyc)")
    p_index.add_argument("--skip-hidden", action="store_true", help="skip hidden files")
    p_index.add_argument("--max-depth", type=int, default=None, help="max recursion depth")
    add_name_args(p_index)
    add_size_args(p_index)
    add_time_args(p_index)
    p_index.add_argument("--json", dest="as_json", action="store_true", help="output as JSON")
    p_index.set_defaults(func=cmd_index)

    # -- du --
    p_du = sub.add_parser("du", help="disk usage analysis")
    p_du.add_argument("directory", nargs="?", default=".", help="directory to analyze (default: .)")
    p_du.add_argument("--depth", type=int, default=1, help="directory depth for grouping (default: 1)")
    p_du.add_argument("--top", type=int, default=20, help="number of top entries (default: 20)")
    p_du.add_argument("--skip-hidden", action="store_true", help="skip hidden files/dirs")
    p_du.add_argument("--ext", nargs="+", default=None, help="filter by extensions")
    add_name_args(p_du)
    add_size_args(p_du)
    add_time_args(p_du)
    p_du.add_argument("--json", dest="as_json", action="store_true", help="output as JSON")
    p_du.set_defaults(func=cmd_du)

    return parser


def main():
    parser = build_parser()
    args = parser.parse_args()
    try:
        args.func(args)
    except Exception as e:
        print(f"error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
