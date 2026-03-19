# Re-export everything from the native Rust module.
# The Rust extension is built by maturin and injected into this package.
from .pyofiles import *  # noqa: F401,F403
from .pyofiles import FileEntry, SizeEntry, DiskUsage  # noqa: F401

from importlib.metadata import version as _version

__version__ = _version("pyofiles")
