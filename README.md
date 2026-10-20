# Manis Road

Rust workspace：

- [`crates/camera-catalog`](crates/camera-catalog/README.md)：独立的上海交通监控公告资料 crate（领域模型、校验、Tokio CLI、候选数据与测试）。
- [`crates/camera-web`](crates/camera-web/README.md)：Rust/WASM 浏览器资料审核预览页，复用 `camera-catalog` 的数据模型与校验。
- [`crates/motu-core`](crates/motu-core/README.md)：摩途 4.1.9 的业务核心重写（接口目录、签名/加密、登录、电子眼扫描、官方点位与详情、车辆、路线审计、本地状态）。
- [`crates/motu-ffi`](crates/motu-ffi/README.md)：`motu-core` 的 JNI JSON 桥。
- [`android/`](android/README.md)：仅含 Java 界面与构建脚本的 Android 工程，业务逻辑全部在 Rust。

现阶段**没有已审核可上图的点位**；历史导航讨论稿在 [`docs/`](docs/) 下，尚非正式 PRD。
摩途重写的证据、协议与限制见 [`docs/motu/`](docs/motu/README.md)。
