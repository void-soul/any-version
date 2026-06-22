# First-Batch Registry Data Plan

## First Batch
1. Node.js
2. Python
3. Go
4. Java (JDK)
5. .NET SDK
6. Rust

## Each Language Must Answer
- Download source
- Version source
- Local discovery rules
- Environment variables
- Package managers and install commands
- Cache paths and migration
- Mirror configuration

## Concrete Template Shape (example sketch)
{
  "id": "...",
  "download_url_template": "...",
  "download_file_ext": "...",
  "remote_versions_config": { ... },
  "env_vars": [ ... ],
  "find_rules": [ ... ],
  "package_managers": [ ... ],
  "has_cache": true,
  "cache_detect_cmd": "...",
  "has_mirror": true,
  "mirror_options": [ ... ]
}

## Migration Notes
- Keep existing Scoop fields temporarily, but mark them as auxiliary.
- Expand `package_managers[*]` for yarn/pnpm/pip/uv/poetry/maven/gradle/cargo/etc.
- Normalize fields that are currently uneven across languages.
