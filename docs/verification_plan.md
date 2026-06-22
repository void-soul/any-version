# Verification Plan

## Evidence Sources
- `projects.json`
- `src-tauri/src/commands/project/types.rs`
- `src-tauri/src/commands/project/registry.rs`
- `src-tauri/src/commands/project/scanner.rs`
- `src-tauri/src/commands/project/versions.rs`
- `src/components/GlobalSettings.tsx`

## Checks
1. First-batch languages have complete core fields.
2. Schema docs match implemented Rust expectations.
3. Scoop is demoted to auxiliary posture in docs/UI copy.
4. Registry continues to load and resolve correctly after changes.
5. No required runtime behavior is removed without replacement.

## Pass Criteria
- All first-batch language objects contain core metadata fields.
- Rust parsing still succeeds for updated JSON.
- Updated UI copy reflects new authority model.
