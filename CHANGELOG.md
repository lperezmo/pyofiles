# CHANGELOG


## Unreleased

### Bug Fixes

- Validate numeric CLI arguments and harden time parsing
  (tests-pytest-and-cli-hardening)

`--max-depth` and `--limit` are validated by argparse as integers >= 0 (--limit 0 and
--max-depth 0 stay meaningful: no matches, and no recursion below the root), du's --depth and
--top keep accepting 0 for totals-only output, and --threads must now be >= 1 instead of
silently falling back to the default. Underscore forms like "1_000" are rejected.

parse_time now accepts case-insensitive duration units ("7D", "24H") and timezone-aware ISO
datetimes ("2024-03-15T10:30:00+02:00" or a trailing "Z") on all supported Python versions; a
date-only value with a trailing "Z" means UTC midnight, and previously-accepted lenient forms
like "2024-3-5" keep working. Bare digit strings always mean unix seconds regardless of Python
version (fromisoformat grew more lenient in 3.11+ and would otherwise reinterpret "20240101" as
a date), and non-finite values (nan, inf) are rejected instead of silently disabling the time
filters.

Size filters get the same treatment in the extension itself, so the Python API is covered too:
min_size_mb/max_size_mb values that are NaN, infinite, or negative raise ValueError instead of
silently disabling the bound (NaN compares false against everything; a negative cast clamped
to zero).

The MFT sector reader no longer issues a padded sector read for zero-length requests, which
could spuriously report UnexpectedEof near the end of a volume.

### Testing

- Convert the Python suite to real pytest tests

tests/test_pyofiles.py previously used a print-only check harness: under pytest every test passed
trivially because nothing asserted, so only the script-mode run in CI caught failures. The suite is
now genuine pytest tests with asserts and per-test tmp_path fixtures (including new coverage for
CLI parsing, time formats, output escaping, and boundary values). CI installs pytest and runs
`python -m pytest tests/ -q` instead of invoking the file as a script.


## v0.7.1 (2026-07-12)

### Bug Fixes

- Bump pyo3 to 0.29 to resolve two security advisories
  ([`7aa5a7e`](https://github.com/lperezmo/pyofiles/commit/7aa5a7e36c447c858c7e8dacb11b4375c481eaec))

Resolves GHSA-36hh-v3qg-5jq4 (high, out-of-bounds read in nth / nth_back for PyList and PyTuple
  iterators) and GHSA-chgr-c6px-7xpp (medium, missing Sync bound on PyCFunction::new_closure
  closures), both fixed in pyo3 0.29.0.

Also opts FileEntry and SizeEntry out of the automatic FromPyObject impl via skip_from_py_object,
  silencing the 0.29 deprecation warning; both types are output-only and never extracted from
  Python.


## v0.7.0 (2026-07-03)

### Chores

- Document the MFT fast path and its caveats
  ([`4226b30`](https://github.com/lperezmo/pyofiles/commit/4226b3030ba83ddaf99be803c47df78f9289f796))

### Features

- Ntfs MFT fast path for disk_usage, find and walk
  ([`212b29a`](https://github.com/lperezmo/pyofiles/commit/212b29a9b65725e09c725b80f6ada470634037ba))

New mft=False keyword on disk_usage, find and walk (CLI: --mft on du, find and walk). On Windows it
  reads the volume's Master File Table through a raw volume handle instead of walking directories,
  WizTree style: one pass over all FILE records collects every non-DOS FILE_NAME (hard links emit
  one entry per link), STANDARD_INFORMATION times and hidden flag, and the unnamed DATA stream size,
  then reconstructs paths from parent references and filters to the requested subtree. Records are
  parsed by parallel workers, each with its own volume handle behind a sector-aligned reader
  (adapted from the ntfs crate's ntfs-shell example) and a chunked read cache.

Results flow through the same Filters and output shapes as the walk backend. Requires administrator
  privileges and a local NTFS volume; failures raise OSError stating both, UNC paths are rejected,
  and non-Windows builds raise ValueError.

### Testing

- Cover MFT error paths locally and the real scan in CI
  ([`2baae39`](https://github.com/lperezmo/pyofiles/commit/2baae39a2d2db5ce8ef684565053b2e0ba957d65))

The local suite asserts the OSError (mentioning administrator) on non-elevated Windows, the UNC
  rejection, and the ValueError on non-Windows, gated by an elevation check so it behaves
  everywhere. CI adds a Windows step that runs the real MFT scan on the elevated runner against
  C:\Windows\System32\drivers, checks find with a limit, and cross-checks file counts against the
  walk backend within 10 percent, plus an ubuntu step asserting the ValueError.


## v0.6.0 (2026-07-03)

### Bug Fixes

- Ship abi3 wheels and let Rust panics raise Python exceptions
  ([`77c9c9c`](https://github.com/lperezmo/pyofiles/commit/77c9c9c6f0932f4eecb0794a7e1b1d0f26de3870))

The release workflows only passed an interpreter list to the Linux builds, so Windows wheels were
  published for a single Python version (cp312 for 0.5.0) and macOS for whatever the runner had
  (cp314). Everyone else silently fell back to the sdist and needed a Rust toolchain. Build with
  pyo3 abi3-py39 instead: one wheel per platform covers Python 3.9 and up, including future
  versions.

Also drop panic=abort from the release profile. In a cdylib loaded into a host interpreter it turned
  any Rust panic into a hard abort of the whole Python process; without it PyO3 converts panics into
  a catchable PanicException.

Bump the trove classifier from Alpha to Beta.

- **cli**: Restore Python 3.9 support, handle closed pipes, and batch output writes
  ([`347eca7`](https://github.com/lperezmo/pyofiles/commit/347eca7921d4541ad495a8b4d0f22015383ba75f))

PEP 604 annotations in cli.py made the console script crash at import on Python 3.9 even though the
  package claims to support it. Add from __future__ import annotations.

Exit cleanly on BrokenPipeError (e.g. piping to head) and on KeyboardInterrupt instead of printing
  an error or a traceback.

Write output lines in a single buffered call instead of one print per line, and only pretty-print
  JSON when stdout is a terminal.

### Chores

- Document new options and behavior, exercise the CLI in CI
  ([`e60c584`](https://github.com/lperezmo/pyofiles/commit/e60c584a0dc20800a5c4bec8eeb7d904b6221fc4))

README: document limit and threads, the behavior notes (hidden files, creation time on Linux,
  unreadable metadata, result ordering, index collisions), update the performance section, and drop
  the downloads badge (it permanently rendered as rate limited).

CI now runs the installed console script and python -m pyofiles across every matrix cell, which
  would have caught the Python 3.9 CLI import crash.

### Features

- Parallel walk, find, glob and index with limit and threads options
  ([`2a8f770`](https://github.com/lperezmo/pyofiles/commit/2a8f7703c05a83f0cb8a4c222820297917964ebc))

Move walk, find, glob and index onto the ignore crate's parallel walker, the same engine disk_usage
  already uses, and drop the jwalk dependency. Filtering and metadata reads now run on N worker
  threads, and metadata comes from the directory read itself where the platform provides it (free on
  Windows) instead of a second stat per file.

On a 442k-file tree on Windows: walk 19.1s -> 0.38s, find by size 17.3s -> 0.31s, find by name 1.9s
  -> 0.31s.

New options: find(limit=N) stops the search as soon as N matches are found (a single-file lookup on
  the same tree returns in ~1.5ms), and every recursive function takes threads=N to tune walker
  parallelism, which helps on network drives.

Also in this change: - glob starts walking at the pattern's literal directory prefix (src/**/*.py no
  longer scans the whole tree) - walk returns only matching files when any filter is active instead
  of interleaving every directory with the matches - index resolves stem collisions
  deterministically by keeping the lexicographically smallest path - files whose metadata cannot be
  read are excluded consistently when a size or time filter is active - list_dir results are sorted
  by name, and skip_hidden honors the Windows hidden attribute everywhere - name and extension
  filters no longer allocate per file when unset, which speeds up unfiltered disk_usage


## v0.5.0 (2026-04-28)

### Features

- Parallelize disk_usage and tune release profile for max throughput
  ([`c776450`](https://github.com/lperezmo/pyofiles/commit/c7764503cdbecca14489e8fc4d3040572364d821))

Rewrite disk_usage on top of ignore::WalkBuilder::build_parallel with DashMap + atomic aggregation,
  mirroring the RustSizer architecture. Per-file metadata calls now run on N worker threads instead
  of serialized through the main thread, eliminating the single-consumer bottleneck that made
  `pyofiles du` slow on large trees.

Also enable LTO=fat, codegen-units=1, opt-level=3, panic=abort, strip in [profile.release], which
  benefits every function in the crate.

No API change; behavior is preserved (gitignore is explicitly disabled so disk_usage continues to
  count every file).


## v0.4.0 (2026-03-19)

### Bug Fixes

- Release workflow for publishing to pypi, and update readme with new version flag.
  ([`f6fd3ba`](https://github.com/lperezmo/pyofiles/commit/f6fd3ba2043521219c62e1e421f35b06b09b5bd5))

### Continuous Integration

- Add semantic release workflow with auto version bumping
  ([`83eda39`](https://github.com/lperezmo/pyofiles/commit/83eda399c8ab5311cb9586e6dadc7857b9e55d04))

- Add semantic release workflow with auto version bumping
  ([`b23227e`](https://github.com/lperezmo/pyofiles/commit/b23227e06c38466ee470afc27659c809655e3841))

### Features

- Add --version/-v flag and expose __version__ in Python package
  ([`724ed62`](https://github.com/lperezmo/pyofiles/commit/724ed62815dcca5d2dfae44661d3a10ccbdee0fb))


## v0.3.0 (2026-03-14)


## v0.2.0 (2026-03-12)


## v0.1.0 (2026-03-12)
