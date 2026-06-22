# Registry Modernization Spec

## Primary Data Source
`projects.json` is the authoritative registry for supported languages and tools.
Scoop integration is optional and auxiliary only.

## Field Responsibility Matrix

### Required
- id
- display_name
- category
- official_website
- env_vars
- find_rules
- download_url_template
- download_file_ext
- remote_versions_config

### Recommended
- extract_subdir
- bin_dirs
- package_managers
- has_cache
- has_mirror
- cache_detect_cmd
- cache_default_path
- mirror_options
- post_install

### Optional/Auxiliary
- remote_versions_url
- pkg_homepage_template
- scoop_ref
- scoop_updated_at

## First-Batch Languages
1. nodejs
2. python
3. go
4. java
5. dotnet
6. rust

## Cross-Field Invariants
- `find_rules` is the primary local-install discovery mechanism.
- `package_managers` should encode install/version/cache/mirror/data configuration when applicable.
- If a language has package managers, its `package_managers[*]` metadata should be complete rather than implicit.
- Runtime behavior must prefer structured JSON metadata over hardcoded assumptions where available.
