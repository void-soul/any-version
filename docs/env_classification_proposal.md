# Env Classification Proposal

| language | var | proposed tier | reason |
|---|---|---|---|
| nodejs | NODE_PATH | compat/discovery | legacy/helper discovery, lower priority under AnyVersion model |
| nodejs | NPM_CONFIG_PREFIX | package-manager | cache/tool config; should be managed alongside package managers |
| nodejs | NPM_CONFIG_CACHE | package-manager | cache/tool config; should be managed alongside package managers |
| nodejs | NVM_DIR | compat/discovery | legacy/helper discovery, lower priority under AnyVersion model |
| nodejs | NVM_HOME | compat/discovery | legacy/helper discovery, lower priority under AnyVersion model |
| nodejs | VOLTA_HOME | compat/discovery | legacy/helper discovery, lower priority under AnyVersion model |
| python | PYTHONHOME | core | directly affects SDK discovery/runtime |
| python | PYTHONPATH | compat/discovery | legacy/helper discovery, lower priority under AnyVersion model |
| python | PIP_CACHE_DIR | package-manager | cache/tool config; should be managed alongside package managers |
| python | CONDA_PREFIX | package-manager | cache/tool config; should be managed alongside package managers |
| python | PYENV_ROOT | compat/discovery | legacy/helper discovery, lower priority under AnyVersion model |
| go | GOROOT | core | directly affects SDK discovery/runtime |
| go | GOPATH | core | directly affects SDK discovery/runtime |
| go | GOBIN | core | directly affects SDK discovery/runtime |
| go | GOCACHE | package-manager | cache/tool config; should be managed alongside package managers |
| go | GOPROXY | compat/discovery | legacy/helper discovery, lower priority under AnyVersion model |
| go | GONOSUMDB | compat/discovery | legacy/helper discovery, lower priority under AnyVersion model |
| go | GONOSUMCHECK | compat/discovery | legacy/helper discovery, lower priority under AnyVersion model |
| java | JAVA_HOME | core | directly affects SDK discovery/runtime |
| java | JDK_HOME | compat/discovery | legacy/helper discovery, lower priority under AnyVersion model |
| java | JRE_HOME | compat/discovery | legacy/helper discovery, lower priority under AnyVersion model |
| java | CLASSPATH | compat/discovery | legacy/helper discovery, lower priority under AnyVersion model |
| dotnet | DOTNET_ROOT | core | directly affects SDK discovery/runtime |
| dotnet | DOTNET_CLI_HOME | package-manager | cache/tool config; should be managed alongside package managers |
| dotnet | NUGET_PACKAGES | package-manager | cache/tool config; should be managed alongside package managers |
| rust | CARGO_HOME | core | directly affects SDK discovery/runtime |
| rust | RUSTUP_HOME | core | directly affects SDK discovery/runtime |
| rust | CARGO_TARGET_DIR | package-manager | cache/tool config; should be managed alongside package managers |