# Code Audit Summary

## Evidence Findings
- `projects.json` is loaded as `Vec<ProjectDef>` in registry and used directly by scanner/versions/env.
- Scoop is actively integrated:
  - `src-tauri/src/commands/project/scoop_manifest.rs` fetches manifest and can write derived fields back.
  - `src/components/GlobalSettings.tsx` exposes "从 Scoop 更新数据" UI.
- Current key consumption points:
  - `remote_versions_config` drives remote version listing.
  - `download_url_template` is used as fallback in install path.
  - `find_rules` drives local detection and manage/unmanage flows.
  - `package_managers` already present but unevenly filled.
  - `bin_dirs` preferred over hardcoded fallback in scanner.

## Risks
1. Data drift between manual definitions and Scoop-derived writes.
2. Mixed authority: UI still highlights Scoop sync as primary.
3. Partial package manager metadata: some tools have rich defs, others thin.

## Recommended Direction
- Make `projects.json` the authoritative registry.
- Keep Scoop helper function, but reduce its product prominence and runtime authority.
- Add schema/validation discipline for first-batch language definitions.
