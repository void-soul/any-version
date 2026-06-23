# Env Audit

## nodejs
- NODE_PATH (path) ? 全局模块搜索路径
- NPM_CONFIG_PREFIX (path) ? npm 全局安装前缀
- NPM_CONFIG_CACHE (path) ? npm 缓存目录
- NVM_DIR (path) ? nvm-windows 安装目录
- NVM_HOME (path) ? nvm-windows 根目录
- VOLTA_HOME (path) ? Volta 安装目录

## python
- PYTHONHOME (path) ? Python 解释器根目录
- PYTHONPATH (path) ? Python 模块搜索路径
- PIP_CACHE_DIR (path) ? pip 缓存目录
- CONDA_PREFIX (path) ? conda 环境目录
- PYENV_ROOT (path) ? pyenv-win 根目录

## go
- GOROOT (path) ? Go 安装根目录
- GOPATH (path) ? Go 工作区路径
- GOBIN (path) ? Go 二进制安装目录
- GOCACHE (path) ? Go 构建缓存目录
- GOPROXY (nonempty) ? Go 模块代理地址
- GONOSUMDB (nonempty) ? 跳过 sum 校验模块列表
- GONOSUMCHECK (nonempty) ? 跳过校验模块列表

## java
- JAVA_HOME (path) ? JDK 安装根目录
- JDK_HOME (path) ? JDK 根目录（替代变量）
- JRE_HOME (path) ? JRE 根目录
- CLASSPATH (nonempty) ? Java 类库搜索路径

## dotnet
- DOTNET_ROOT (path) ? .NET SDK root
- DOTNET_CLI_HOME (path) ? CLI home directory
- NUGET_PACKAGES (path) ? global packages cache

## rust
- CARGO_HOME (path) ? Cargo 包管理器目录
- RUSTUP_HOME (path) ? Rustup 工具链目录
- CARGO_TARGET_DIR (path) ? Cargo 构建输出目录
