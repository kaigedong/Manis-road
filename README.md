# Manis Road

Rust workspace：

- [`crates/camera-catalog`](crates/camera-catalog/README.md)：单一库 crate，下设 `src/catalog/` 与 `src/collector/`；前者是跨平台资料/事件模型及校验，后者在原生端提取公告候选并生成采集审计事件。无 `main`。
- [`crates/camera-cli`](crates/camera-cli/README.md)：使用 Tokio 异步读取文件/保存快照，调用 catalog 内的 collector；可离线导入公告或按白名单在线采集。
- [`crates/camera-web`](crates/camera-web/README.md)：Rust/WASM 浏览器资料审核预览页，复用 `camera-catalog` 的数据模型与校验。
- [`crates/motu-core`](crates/motu-core/README.md)：摩途 4.1.9 的业务核心重写（接口目录、签名/加密、登录、电子眼扫描、官方点位与详情、车辆、路线审计、本地状态）。
- [`crates/motu-ffi`](crates/motu-ffi/README.md)：`motu-core` 的 JNI JSON 桥。
- [`crates/motu-routing`](crates/motu-routing/src/lib.rs)：离线 OSM 路网图与 A* 摩托车路由，构建一次后运行时零网络。
- [`crates/motu-pack`](crates/motu-pack/README.md)：离线数据包 `.mrdp` 的读写与 CLI，把图、先验、禁区、电子眼打成单一可分发文件。
- [`crates/motu-web`](crates/motu-web/README.md)：本机 Web 调试台；Leaflet 展示上海电子眼，登录/同步/详情全部经由 `motu-core`，点位/详情/图片缓存在本机 SQLite（git-ignored）；支持 `MOTU_WEB_OFFLINE=1` 零网络运行与本地路线规划。
- [`android/`](android/README.md)：仅含 Java 界面与构建脚本的 Android 工程，业务逻辑全部在 Rust；地图页支持离线路线（JNI `nativeRoute`）与 GPS 巡航。

现阶段**没有已审核可上图的点位**；历史导航讨论稿在 [`docs/`](docs/) 下，尚非正式 PRD。
摩途重写的证据、协议与限制见 [`docs/motu/`](docs/motu/README.md)。
离线模式、数据准备与离线路由见 [`docs/offline.md`](docs/offline.md)。
