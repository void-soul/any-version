# Verification Checklist

## 1. Rust schema aligned with actual package manager metadata
- `src-tauri/src/commands/project/types.rs` now includes package-manager cache/data fields and `remote_versions_config`.
- Evidence lines: 86-107.

## 2. Scoop demoted to optional helper
- `src/components/GlobalSettings.tsx` updated UI copy to say Scoop is an auxiliary data helper and projects.json remains primary.
- Evidence line: 485.

## 3. First-batch language coverage in registry data
- `projects.json` and `src-tauri/projects.json` now contain all first-batch ids: `nodejs`, `python`, `go`, `java`, `dotnet`, `rust`.

## 4. Registry docs created
- `docs/registry_schema.md` defines primary data source and field responsibility matrix.

## 5. First-batch dimension coverage generated
- Added `docs/first_batch_matrix.md` to track required metadata coverage across the first six languages.

## Remaining Follow-ups
- Keep enriching any language entries that still show unchecked metadata cells in the matrix.
- Keep Scoop module as optional utility rather than runtime source of truth.
