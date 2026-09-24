# Manis Road

Rust workspace：

- [`crates/camera-catalog`](crates/camera-catalog/README.md)：独立的上海交通监控公告资料 crate（领域模型、校验、Tokio CLI、候选数据与测试）。
- [`crates/camera-web`](crates/camera-web/README.md)：Rust/WASM 浏览器资料审核预览页，复用 `camera-catalog` 的数据模型与校验。

现阶段**没有已审核可上图的点位**；历史导航讨论稿在 [`docs/`](docs/) 下，尚非正式 PRD。
