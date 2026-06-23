# Node.js 环境变量参考

> 仅列出 Node.js 运行时自身的环境变量，不含 npm/pnpm/yarn 相关配置。

## 核心运行时

| 变量 | 说明 | 示例值 |
|------|------|--------|
| `NODE_ENV` | 运行环境标识，大量库据此切换行为（压缩、缓存、日志等） | `development` / `production` / `test` |
| `NODE_OPTIONS` | 传递 V8 / Node CLI 标志，无需改启动脚本 | `--max-old-space-size=4096 --inspect=0.0.0.0:9229` |
| `NODE_PATH` | 模块查找的额外搜索路径（`;` 分隔） | `D:\lib\modules;D:\shared\node_modules` |
| `NODE_NO_WARNINGS` | 设为 `1` 静默所有进程警告 | `1` |

## V8 引擎与内存

| 变量 | 说明 | 示例值 |
|------|------|--------|
| `--max-old-space-size` | Old Space 堆上限 (MB)，通过 `NODE_OPTIONS` 设置 | `NODE_OPTIONS=--max-old-space-size=8192` |
| `--max-semi-space-size` | Semi Space (新生代) 上限 (MB) | `NODE_OPTIONS=--max-semi-space-size=128` |
| `UV_THREADPOOL_SIZE` | libuv 线程池大小，影响文件 I/O / DNS 并发 | `16`（默认 4，上限 1024） |
| `NODE_HEAP_SIZE_LIMIT` | 等效 `--max-old-space-size`，部分工具链识别 | `4096` |

## 调试与诊断

| 变量 | 说明 | 示例值 |
|------|------|--------|
| `NODE_DEBUG` | 启用内置模块调试输出（`,` 分隔模块名） | `http,net,fs,module` |
| `DEBUG` | `debug` 库通配符过滤，生态系统广泛支持 | `app:*` / `express:*` / `*` |
| `NODE_V8_COVERAGE` | 输出 V8 代码覆盖率 JSON 到指定目录 | `./coverage` |
| `NODE_EXTRA_CA_CERTS` | 追加可信 CA 证书文件 (PEM) | `/etc/ssl/my-ca.pem` |
| `NODE_INSPECT_RESUME_ON_START` | Node 22+ 调试器在断点处是否暂停等待连接 | `1`（暂停） / `0`（立即运行） |

## 网络 / TLS

| 变量 | 说明 | 示例值 |
|------|------|--------|
| `HTTP_PROXY` / `HTTPS_PROXY` | 出站 HTTP/HTTPS 代理（需配合 `globalAgent` 或代理库生效） | `http://127.0.0.1:7890` |
| `NO_PROXY` | 不走代理的域名/IP（`,` 分隔） | `localhost,127.0.0.1,.internal` |
| `NODE_TLS_REJECT_UNAUTHORIZED` | 设 `0` 禁用 TLS 证书校验（**仅开发用，安全风险**） | `0` |
| `PORT` / `HOST` | Web 框架约定监听端口和地址 | `PORT=3000 HOST=0.0.0.0` |

## 终端 / 显示

| 变量 | 说明 | 示例值 |
|------|------|--------|
| `FORCE_COLOR` | 强制启用彩色输出（chalk 等库使用） | `1`（启用） / `0`（禁用） |
| `NO_COLOR` | 标准约定：设此变量禁用所有颜色输出 | `1` |
| `TERM` | 终端类型，影响 ANSI 支持判断 | `xterm-256color` |
| `COLORTERM` | 彩色终端标识 | `truecolor` |

## 模块系统与缓存

| 变量 | 说明 | 示例值 |
|------|------|--------|
| `NODE_COMPILE_CACHE` | Node 22+ 字节码编译缓存目录，加速冷启动 | `./.node-cache` |
| `NODE_DISABLE_COMPILE_CACHE` | Node 22+ 设 `1` 禁用编译缓存 | `1` |
| `NODE_PENDING_DEPRECATION` | 设 `1` 时将 pending deprecation 当作正式警告输出 | `1` |
| `NODE_PENDING_PAUSE_ON_READLINE_INPUT` | Node 22+ 在 `readline` 输入处暂停事件循环 | `1` |

## ESM / 包解析

| 变量 | 说明 | 示例值 |
|------|------|--------|
| `NODE_EXPERIMENTAL_REQUIRE_MODULE` | Node 23+ 允许 `require()` 加载 ESM 模块 | `1` |
| `NODE_OPTIONS=--experimental-default-type=module` | 默认将 `.js` 当作 ESM 解析 | 加入 `NODE_OPTIONS` |
| `NODE_OPTIONS=--experimental-json-modules` | 允许 ESM import JSON | 加入 `NODE_OPTIONS` |
| `NODE_OPTIONS=--experimental-specifier-resolution=node` | ESM 导入可省略扩展名（旧版） | 加入 `NODE_OPTIONS` |

## 其他

| 变量 | 说明 | 示例值 |
|------|------|--------|
| `NODE_ICS` | Node 22+ 启用 ICU（国际化）数据加载 | `/path/to/icu.dat` |
| `NODE_REPL_EXTERNAL_MODULE` | REPL 中默认预加载的模块 | `lodash` |
| `NODE_SKIP_FIPS` | 设 `1` 跳过 FIPS 模式检查 | `1` |
| `OPENSSL_CONF` | OpenSSL 配置文件路径，影响 Node TLS 底层行为 | `/etc/ssl/openssl.cnf` |

---

## 快速速查：常见问题对应变量

| 问题 | 变量 | 一行设置 |
|------|------|----------|
| 构建时 OOM | `NODE_OPTIONS` | `NODE_OPTIONS=--max-old-space-size=4096` |
| 调试第三方库输出 | `DEBUG` | `DEBUG=app:*` |
| 自签名证书报错 | `NODE_EXTRA_CA_CERTS` | `NODE_EXTRA_CA_CERTS=./my-ca.pem` |
| 禁用颜色（CI/管道） | `NO_COLOR` | `NO_COLOR=1` |
| Node 22+ 加速冷启动 | `NODE_COMPILE_CACHE` | `NODE_COMPILE_CACHE=./.node-cache` |
| 禁用进程警告 | `NODE_NO_WARNINGS` | `NODE_NO_WARNINGS=1` |
| DNS/文件 I/O 并发不够 | `UV_THREADPOOL_SIZE` | `UV_THREADPOOL_SIZE=64` |
