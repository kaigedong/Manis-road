# 电子眼资料审核预览（Rust/WASM）

独立 WASM crate：复用 `manis-camera-catalog` 的 JSON 解析与来源校验，使用 Rust 生成转义后的页面内容；JavaScript 仅负责 `fetch` JSON、加载 wasm 和挂载 DOM。此页不是导航或完整电子眼地图：未经人工审核的记录只展示为候选列表。

在仓库根目录运行：

```bash
cargo test --offline --workspace
wasm-pack build crates/camera-web --target web --out-dir pkg --release --offline
python3 crates/camera-web/tests/browser_smoke.py  # 本机安装 Chrome 时运行
python3 -m http.server 8000
```

打开 `http://localhost:8000/crates/camera-web/www/`。请从 HTTP 服务访问，勿直接用 `file://`；页面优先读取 `var/camera/catalog.json`（若已用 `camera-cli collect-file/collect-url` 采集），否则退回同一仓库下的 `crates/camera-catalog/data/shanghai-cameras.json`。展示资料采集审计事件；这些不是实际交通抓拍事件。打包结果 `crates/camera-web/pkg/` 不提交版本库。

后续加入地图/定位前需先有人审过精确坐标和数据再利用权利。目前页面只显示公告、地点描述和未知照片拍摄日期，不把 94 套的公告总数当作 94 条已录入点位。
