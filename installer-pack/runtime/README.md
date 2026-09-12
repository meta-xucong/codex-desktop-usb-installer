# Python runtime materialization

The public kit deliberately does not contain wheel binaries or a fake “complete” lock file.
`requirements-python-win-x64-py312.in` is the direct input. Run the materializer on an approved
build machine with access to the package index; it downloads the full transitive closure for
Windows x64 / CPython 3.12, writes one SHA-256-pinned requirements lock and a machine-readable
wheel manifest under `runtime/materialized/`.

The pending public manifest is not install-ready. The private USB release process must verify every
wheel, run the import health checks, sign the media manifest, and only then mark the private copy
`materializationStatus=ready`. If any requirement has no compatible wheel, the release must fail and
the affected Skill remains `auto-installable-runtime` with an explicit missing-wheel report.

The wheelhouse must be installed with the generated lock, not the direct `.in` file:

```powershell
python -m pip install --no-index --find-links <wheelhouse> --require-hashes -r <generated-lock>
```
