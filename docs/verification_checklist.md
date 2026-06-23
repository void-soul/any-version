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

## 6. Environment variable tiering implemented
- `src-tauri/src/commands/project/types.rs`: added `EnvVarTier` enum (Core/Package/Compat) and `tier` field to `EnvVarDef`.
- `projects.json` / `src-tauri/projects.json`: all first-batch env vars annotated with `tier` field.
- Policy docs: `docs/env_policy.md`, `docs/env_tier_matrix.md`.

## Remaining Follow-ups
- Keep enriching any language entries that still show unchecked metadata cells in the matrix.
- Keep Scoop module as optional utility rather than runtime source of truth.

## Remaining Follow-ups
- Keep enriching any language entries that still show unchecked metadata cells in the matrix.
- Keep Scoop module as optional utility rather than runtime source of truth.


## 7. Frontend types aligned
- `src/components/project/types.ts`: `ProjectDef.env_vars` includes `tier` field.
- `src/components/project/types.ts`: `PackageManagerDef` includes `remote_versions_config`.

## 8. All JSON data fields consistent
- Both `projects.json` and `src-tauri/projects.json` contain identical first-batch language coverage.
- All env vars carry `tier` annotation (0 missing across all languages).
- `.NET SDK` entry added as first-batch language.


## Phase 2: Runtime Code Convergence

### 9. Tier-aware env management
- `configure_sdk_env_vars`: skips Compat tier vars (only sets Core + Package)
- `remove_sdk_env_vars`: skips Compat tier vars (only removes Core + Package)

### 10. JSON-driven cache path resolution
- `cache.rs`: added `resolve_cache_from_json()` that reads `package_managers[*].cache_detect_cmd` and `cache_default_path`
- `scanner.rs`: `build_cache_status` now tries JSON config first, falls back to hardcoded only if JSON has no match

### 11. versions.rs already JSON-first
- `get_download_url` reads from `download_url_template`, `version_prefix_map`, `version_url_prefix_map` in JSON
- No hardcoded download URLs in the version install flow


## Phase 3: Code Convergence + Frontend + Safety

### 12. EnvVarStatus now carries tier
- `src-tauri/src/commands/project/types.rs`: added `tier: Option<EnvVarTier>` to `EnvVarStatus`
- `src-tauri/src/commands/project/scanner.rs`: `build_env_vars_status` populates tier from `var_def.tier`

### 13. Frontend tier visibility
- `src/components/project/types.ts`: `EnvVarStatus` includes `tier` field
- `src/components/project/ProjectSubTabs.tsx`: EnvVarsTab shows Core/Package/Compat tier labels

### 14. Manage/unmanage tier safety
- `project_manage`: backup, set-env, and add-path loops all skip Compat tier
- `project_unmanage`: remove-env and restore-env loops all skip Compat tier
- Result: Compat vars (NODE_PATH, NVM_HOME, VOLTA_HOME, etc.) are never touched by AnyVersion

### 15. Hardcoded fallbacks documented
- scanner.rs get_bin_paths: has JSON-first + hardcoded fallback (acceptable)
- env.rs get_sdk_bin_paths: has JSON-first + hardcoded fallback (acceptable)
- versions.rs detect_installed_version: hardcoded per-language commands (follow-up)


## Phase 4: Configurable Detection + Second-Tier + Mirror

### 16. Version detection configurable via JSON
- `ProjectDef` now has `version_cmd` and `version_exe` fields
- `detect_version_from_path` in versions.rs uses JSON config first, hardcoded fallback
- All 17 non-platform projects have version_cmd (android/harmony/cuda/ffmpeg excluded)

### 17. Second-tier languages added
- PHP, Ruby, Perl, Lua added to both projects.json files
- Each has: env_vars (with tier), find_rules, version_cmd, package_managers, mirror_options
- Total projects: 21

### 18. Mirror management config-driven
- `set_mirror` now tries JSON `mirror_cmd_template` first, falls back to hardcoded
- `get_mirrors_list` extended with JSON-driven mirror entries

### Data Summary
- Total projects: 21
- Total env vars: 56 (27 core, 12 package, 17 compat)
- All first-batch languages: fully complete
