30模块隔离验收入口（Q-0662）

地址：http://127.0.0.1:5196/reports/module-ui-2026-11-02/preview/index.html
本目录使用实际 moduleRegistry、组件和 App.css，不加载真实 Tauri 桥接或用户数据库。
复用实际默认全局主题预置和 React Activity 挂起语义；保留固定30模块顺序。
与完整桌面App的导航栏、原生窗口、系统集成仍有边界，不能用这些截图证明原生业务全部成功。

启动（项目根目录）：
  npx vite --config reports/module-ui-2026-11-02/preview/vite.config.mjs
端口严格为5196；已有服务时不要重复启动。HMR已关闭，修改夹具后需重新加载页面。

类型检查：
  npx tsc --project reports/module-ui-2026-11-02/preview/tsconfig.json --noEmit

Playwright使用现有npm缓存，不是项目新增依赖。本次缓存位置：
  $env:PLAYWRIGHT_MODULE='D:/any-versions/caches/npm/_npx/e41f203b7505f1fb/node_modules/playwright'
  $env:PW_TEST_SCREENSHOT_NO_FONTS_READY='1'
  node reports/module-ui-2026-11-02/preview/smoke.cjs
  node reports/module-ui-2026-11-02/preview/interactions.cjs
使用独立 headless Edge，不打开或关闭用户浏览器profile；finally关闭本次创建的上下文和浏览器。
界面语言zh-CN，时区Asia/Shanghai；宽1280×800，窄800×900。
可用 $env:ACCEPT_MODULES='logViewer,jsonBrowser' 只复测指定模块；部分复测会合并其余已有结果。
全量复测前 Remove-Item Env:ACCEPT_MODULES -ErrorAction SilentlyContinue。

模拟契约：
- tauri.ts只显式提供已读类型的初始化/已测操作；数组与分页对象分开，未知命令记录到__fixtureMissing并抛错。
- 类型来自实际AiConfig、音乐、收藏等契约；调用参数用structuredClone捕获当时快照。
- __fixtureCalls是调用记录；__fixtureResponses是本页的可注入返回值；__fixtureOverrides可通过addInitScript在初始化前覆盖。
- __fixtureFaults[command]={error,delay}注入错误和延迟；不会穿透到真实IPC。
- __fixtureEmit(event,payload)按Tauri事件外壳投递，listen返回真实取消函数。
- __fixtureDialogs队列模拟open/save选择结果；openPath/openUrl/自启/更新等插件也不操作真实系统。
- CSP及Playwright路由限制到当前5196源，外网和其他loopback端口均不允许。
- 每个交互用例单独浏览器上下文：虚拟时钟在共享上下文中会影响Monaco Performance测量，曾可复现duration异常；分开后不修改产品代码即可通过。

输出与含义：
- ../ui-evidence/smoke.json：30页初始化、页面/控制台错误、IPC、缺失契约及页面宽度。
- ../ui-evidence/interactions.json：30个具名操作断言与IPC记录；预期错误会通过界面断言，不能解释为真实外部业务成功。
- <module>.png / <module>-narrow.png：宽/窄入口截图。
- <module>-interaction.png：操作后的界面；rtspServer-corrupt.png是损坏存储保护证据。
- contact-*.png是目视核对概览，详细内容以各模块原图为准。
- 截图成功本身不等于验收通过；最终状态与真实环境边界见 ../FINAL-REPORT.txt。

日志用例使用可刷新的File句柄样例，覆盖追加/截断/同长度改写、读取失败后恢复、隐藏暂停、停止跟随及Activity返回后新快照。
JSON用例真实键盘输入Monaco并观察结构树；原生二进制日志IO另由临时文件Rust测试覆盖。
RTSP用例写入隔离浏览器自己的损坏localStorage值，确认错误可见且原值未覆盖。
服务用例用有数据卡片、安装失败、npm-log事件和模拟剪贴板失败/恢复；不会真实安装、启停服务或修改系统剪贴板。
