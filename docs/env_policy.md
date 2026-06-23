# Environment Variable Policy

## Tiers
### 1. Core language variables
These directly affect SDK discovery, runtime behavior, or primary toolchain resolution.
AnyVersion should manage them strongly.

Examples:
- GOROOT, GOPATH, GOBIN
- JAVA_HOME
- DOTNET_ROOT
- RUSTUP_HOME, CARGO_HOME
- PYTHONHOME (when applicable)

### 2. Package manager / cache variables
These belong to the package manager or cache layer, not language core.
Prefer managing them under `package_managers[*]`.

Examples:
- NPM_CONFIG_PREFIX, NPM_CONFIG_CACHE
- PIP_CACHE_DIR, CONDA_PREFIX
- GOCACHE
- DOTNET_CLI_HOME, NUGET_PACKAGES
- CARGO_TARGET_DIR

### 3. Compat / discovery variables
These are useful for detecting existing installations or legacy workflows,
but should not be treated as primary management targets under AnyVersion.

Examples:
- NODE_PATH, NVM_HOME, NVM_DIR, VOLTA_HOME
- PYTHONPATH, PYENV_ROOT
- JDK_HOME, JRE_HOME, CLASSPATH
- GOPROXY, GONOSUMDB, GONOSUMCHECK

## Implementation Guidance
- Keep core vars in project-level `env_vars`.
- Move cache/path/tool-config vars toward `package_managers[*]` metadata.
- Treat compat vars mainly as `find_rules` evidence and diagnostics context.
- Avoid treating all env vars as equally important.
