# camera-cli

`#[tokio::main]` **仅**存在于此二进制 crate；它用 `tokio::fs` 异步读写目录、公告快照，调用同一 `manis-camera-catalog` 库内的 `catalog` 与 `collector` 模块。库 crate 无二进制入口。

从 workspace 根目录运行：

```bash
cargo run --offline -p camera-cli -- validate crates/camera-catalog/data/shanghai-cameras.json
cargo run --offline -p camera-cli -- stats crates/camera-catalog/data/shanghai-cameras.json
cargo run --offline -p camera-cli -- points crates/camera-catalog/data/shanghai-cameras.json
cargo run --offline -p camera-cli -- collect-file crates/camera-catalog/data/shanghai-cameras.json xuhui-20250828 /path/to/notice.html var/camera
cargo run --offline -p camera-cli -- collect-url crates/camera-catalog/data/shanghai-cameras.json xuhui-20250828 var/camera
```
