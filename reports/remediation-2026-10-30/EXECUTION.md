# 整改执行归档

最终状态：原26项已验证，原76个超长文件已清零；以FINAL-REPORT.txt、audit-results.json和verification-summary.json为准。

用户已授权多agent全部修复并逐批QA、检查、完善、提交；主agent串行验收与Git，未push。测试采用临时fixture；保留已有qa.db修改。最终分工与授权规则见PLAN.md。

## 验收

- 前端tsc、50文件397测试、生产构建通过。
- Rust1459沙箱+18精确隔离测试通过，6原ignored；all-targets通过。
- 共享部署14、receiver10、QR1通过；依赖漏洞条目0，维护/非Windows警告单独说明。
- 全量1084源文件无达到1000行者；关键公共层依赖守卫通过。
- 最终复核补齐utils/config反向依赖，76处导出按内部/crate/旧祖先范围保留可见性，全部重新编译与回归。

## 代码批次

详见commit-index.tsv。

- 92269b7 refactor: #Q-0520 Extract shared UI capabilities and module metadata (#Q-0521)
- 23c2c84 fix: #Q-0508 Unify credential master key creation and preserve damaged keys
- e75471d fix: #Q-0513 Constrain HTTP file resolution and preserve encoded filenames (#Q-0528)
- 8d453d5 fix: #Q-0527 Correct Markdown association registry rights and error propagation
- a3c98e3 test: #Q-0529 Check and test the standalone certificate receiver in CI
- aa7ad24 docs: #Q-0531 Align development prerequisites and port documentation with CI
- 9a6ec1b fix: #Q-0507 Preserve unrelated configuration updates across stale saves
- 354b5f5 fix: #Q-0509 Apply field-level application settings and truthful feedback (#Q-0519 #Q-0523)
- 6dfc03f fix: #Q-0517 Bound load test resources and release completed sample state (#Q-0518)
- 3a3c3af refactor: #Q-0532 Separate panel components and preserve module state ownership
- e33aa7c fix: #Q-0525 Harden certificate push validation and transactional reloads (#Q-0524 #Q-0526)
- 2f7a760 refactor: #Q-0533 Separate project subpages and environment conflict wizard views
- 603eabd refactor: #Q-0534 Extract typed serial RTSP MCP and provider editor views
- 3afc495 refactor: #Q-0535 Separate workspace settings project and Markdown feature responsibilities
- da802d6 fix: #Q-0510 Coordinate storage transactions module lifecycles and shared certificate deployment (#Q-0511 #Q-0512 #Q-0514 #Q-0515 #Q-0516 #Q-0524 #Q-0525 #Q-0526 #Q-0530)
- d8bfa8f refactor: #Q-0536 Extract skill library operations and typed management views
- 01e0738 refactor: #Q-0537 Isolate package storage workflow from manager workspace
- 10414db refactor: #Q-0539 Separate certificate models deployment adapters monitoring and command entry points
- a9e0b4f refactor: #Q-0540 Separate configuration models persistence backups and settings commands
- 3f8ce97 refactor: #Q-0541 Isolate credential restore fixtures and move pure UTF8 helpers to foundation
- 5eda7f7 refactor: #Q-0522 Invert AI transport dependencies through public contracts and application hooks
- 61eebfb fix: #Q-0538 Upgrade vulnerable archive and TLS dependencies while retaining Rust 1.88 support
- 9123505 fix: #Q-0542 Query Windows TCP listener ownership through the native IP Helper API
- 511f9a3 refactor: #Q-0543 Separate API format parsers and domain command responsibilities
- 5c904ca refactor: #Q-0544 Separate proxy transport and protocol implementations by responsibility
- fbe8064 refactor: #Q-0545 Separate AI tool launch model marketplace session and operation features
- ccb3a85 refactor: #Q-0546 Separate favorites category AI and import task features
- 71ed789 refactor: #Q-0547 Split env.rs into cohesive private responsibility modules
- 6a095ab refactor: #Q-0548 Split node_manager.rs into cohesive private responsibility modules
- 26c4cb2 refactor: #Q-0549 Split project/commands.rs into cohesive private responsibility modules
- 6a41d01 refactor: #Q-0550 Split project/versions.rs into cohesive private responsibility modules
- 6e20c2f refactor: #Q-0551 Split project/scanner.rs into cohesive private responsibility modules
- 6c0ce7a refactor: #Q-0552 Split network.rs into cohesive private responsibility modules
- 8daa799 refactor: #Q-0553 Extract port detection responsibilities from shared utilities
- 3647320 refactor: #Q-0554 Separate AI orchestration and tool management by responsibility
- 4d86587 refactor: #Q-0555 Separate designer generation repair parsing and reverse database readers
- a47302c refactor: #Q-0556 Separate mindmap orchestration tools project scans and import graph analysis
- d3327c6 refactor: #Q-0557 Separate launcher database categories items backup and regression tests
- 01d1917 refactor: #Q-0558 Separate API request load test project and editor responsibilities
- 9230a7b refactor: #Q-0560 Split music-player into cohesive responsibility modules
- ed206e7 refactor: #Q-0561 Split music-plugin-host into cohesive responsibility modules
- b0e0d95 refactor: #Q-0562 Split mihomo into cohesive responsibility modules
- 0becc49 refactor: #Q-0563 Split picky into cohesive responsibility modules
- 6f201d5 refactor: #Q-0564 Split suno into cohesive responsibility modules
- dd46cd2 refactor: #Q-0565 Split service into cohesive responsibility modules
- f151a54 refactor: #Q-0566 Separate Buddy account APIs and scheduling responsibilities
- 20e39f2 refactor: #Q-0567 Separate favorite storage import and classification responsibilities
- 8f6d7a4 refactor: #Q-0568 Separate launcher native integration and file search responsibilities
- 0103b0d refactor: #Q-0569 Separate tool registry configuration contracts from loading
- 89cf0be refactor: #Q-0570 Separate IPC registration from application startup composition
- d7cbe6b refactor: #Q-0571 Isolate database design document AI session canvas and editor features
- ed0fd21 refactor: #Q-0572 Separate mindmap document library history layout and position persistence
- e0733c8 refactor: #Q-0575 Complete Buddy session responsibility boundaries
- 0649c0a refactor: #Q-0576 Complete Buddy session responsibility boundaries
- 6381dfc refactor: #Q-0577 Complete Buddy session responsibility boundaries
- f6d33f0 refactor: #Q-0578 Complete frontend feature responsibility boundaries
- 24c8ab6 refactor: #Q-0579 Complete frontend feature responsibility boundaries
- 9e34e6f refactor: #Q-0515 Complete one-way foundation and configuration storage boundaries
- ad81c11 refactor: #Q-0574 Match private module reexports to their actual visibility scopes
- 8e02d65 fix: #Q-0573 Patch sanitizer math renderer and Vite dependencies (#Q-0559)
