# Env Tier Matrix

## nodejs
- NODE_PATH: compat
- NPM_CONFIG_PREFIX: pkg
- NPM_CONFIG_CACHE: pkg
- NVM_DIR: compat
- NVM_HOME: compat
- VOLTA_HOME: compat

## python
- PYTHONHOME: core
- PYTHONPATH: compat
- PIP_CACHE_DIR: pkg
- CONDA_PREFIX: pkg
- PYENV_ROOT: compat

## go
- GOROOT: core
- GOPATH: core
- GOBIN: core
- GOCACHE: pkg
- GOPROXY: compat
- GONOSUMDB: compat
- GONOSUMCHECK: compat

## java
- JAVA_HOME: core
- JDK_HOME: compat
- JRE_HOME: compat
- CLASSPATH: compat

## dotnet
- DOTNET_ROOT: core
- DOTNET_CLI_HOME: pkg
- NUGET_PACKAGES: pkg

## rust
- CARGO_HOME: core
- RUSTUP_HOME: core
- CARGO_TARGET_DIR: pkg
