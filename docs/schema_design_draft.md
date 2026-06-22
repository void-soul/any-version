# Schema Design Draft

## Field Classes
### Required
- id
- display_name
- category
- official_website
- download_url_template
- download_file_ext
- remote_versions_config
- env_vars
- find_rules
- package_managers

### Recommended
- extract_subdir
- bin_dirs
- has_cache
- has_mirror
- cache_detect_cmd
- cache_default_path
- mirror_options
- post_install

### Optional / Auxiliary
- remote_versions_url
- pkg_homepage_template
- scoop_ref
- scoop_updated_at

## Field Notes
- `find_rules` remains the primary mechanism for discovering locally installed roots.
- `package_managers` should encode install command, version command, cache command, mirror command, and cache/data paths.
- Scoop should be demoted to optional bootstrap/fill helper, not runtime authority.

## Verification Targets
1. projects.json uses the schema consistently for first-batch languages.
2. Rust types reflect required/recommended/optional semantics.
3. Frontend explanations align with the new schema posture.
