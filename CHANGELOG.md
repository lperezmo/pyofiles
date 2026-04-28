# CHANGELOG


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
