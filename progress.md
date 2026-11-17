# 进度日志

## 当前会话
- 已确认工作目录为空且非 Git 仓库；读取了规划技能，建立研究与讨论流程。
- 已检索政府法规、竞品、SDK、开源地图授权；首轮搜索摘要对外牌摩托限行有相互矛盾的说法，不能当做定论。
- fetch_content 读取原文时全部触发环境 fake-IP 198.18.x.x 的 SSRF 拦截；后续改用 web_search/source_check 寻找原文段落并标注未核实处，不修改安全配置。
- 已完成 docs/上海摩托车导航-方案讨论稿.md：比较标准 SDK 在线方案与商业授权离线自研方案，给出 Rust+原生架构、测试指标、数据合规与五个待确认问题。
- 已用 wc/rg 检查文档及关键覆盖点；本轮无代码实现或自动化测试。阻塞：官方原文直取受网络环境限制、数据授权和路线边界待验证。
- 用户已决定不要求离线、仅 Android + 浏览器功能一致、按已知监控空白区优先、缩放显示电子眼照片日期、车型选具体型号。
- 研究了高德/百度摩托车 WebService、浏览器定位限制、上海公开摄像头点位及照片数据可得性；新建 docs/上海摩托车导航-在线方案讨论稿.md 作为第二轮方案。修正了文档中的商店链接。
- 用户暂缓导航/正式 PRD，要求先收集电子眼公开记录且 Rust 尽量模块化。
- 建立 `data/shanghai-cameras.json`：2 个政府公告来源、3 个从检索摘录得到的待核验候选地点；没有精确坐标、照片/拍摄日期或摩托车抓拍结论。存在另一候选地点搜索摘要相互冲突，未录入。
- 新增 Rust 项目：`src/model.rs`、`src/catalog.rs`、`src/main.rs`、`tests/catalog.rs`、`README.md` 和 `.gitignore`；已审核点位过滤对全部候选输出空数组。
- TDD：初始 `cargo test --offline` 因尚未实现 crate 导入失败（预期 RED）；实现后测试通过。两次 `cargo fmt --check` 发现新增测试格式差异，均用 `cargo fmt` 修复。
- 用户指出模块应独立 crate，且要求 Tokio + WASM 网页；已将项目改为 workspace：`crates/camera-catalog`（纯 Rust 领域模型+Tokio 原生异步读取/CLI）、`crates/camera-web`（共享领域模块，wasm-bindgen Rust 生成安全转义的审核预览，JS 只加载）；数据、测试与 README 搬入独立 crate。
- 验证：workspace 9 项测试、clippy -D warnings、WASM target check 与 wasm-pack release 编译通过；Chrome headless 页面显示 3 候选/0 已审核和日期未知。首次 Chrome 在 macOS 输出 DOM 后未退出，改为有超时保护的 `browser_smoke.py` 测试并通过。
- 已完成首次 commit `8e6d414`。后续用户明确不再每轮提交，且要求将 task_plan.md/findings.md/progress.md 从 Git 跟踪去除；已 git rm --cached（本地保留）并加 .gitignore，未提交。
- 用户追问库 crate 中为何有 main：将二进制入口移至 `crates/camera-cli`，Tokio 异步 I/O 归 CLI；`camera-web` WASM 仍共享库。验证 workspace 9 项测试、clippy、wasm32 check、Chrome 冒烟均通过；未提交。
- 用户要求在同一 catalog crate 内实现采集，两个文件夹即可：已建 `src/catalog/`（模型、校验、审计事件）和 `src/collector/`（政府 URL 白名单下载、HTML 地点表格提取、原始快照 SHA-256、来源变更/候选发现/人工复核事件与去重）；`camera-cli collect-file/collect-url` 保存原文快照与单一 JSON 事件目录；WASM 可显示事件。当前网络 fake-IP 阻断，尚未获取官方页面原始 HTML，**种子真实线上采集事件仍是 0**；仅测试夹具生成模拟事件，不宣称真实点位已核验。
- 意外中断后恢复上下文，补了 URL 白名单单测与空表人工复核测试；曾遇 `clippy::items_after_test_module`，已把测试模块移到文件末尾。最终 fmt、Rust 16 项测试、Clippy -D warnings、wasm32 check、wasm-pack 构建和 Chrome 冒烟均通过。
