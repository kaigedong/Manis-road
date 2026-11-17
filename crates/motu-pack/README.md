# motu-pack

离线数据包 `.mrdp` 的读写实现与命令行工具。App 运行时只读这个包，不做任何联网。

## 为什么需要它

“完全离线”不是一句承诺，而是一个交付物：路网图、时段先验、禁摩区域、电子眼点位
（以及后续的地图瓦片）必须以**一个可拷贝的文件**到达手机，并且能判断它是否完整。

## 包格式

```text
magic           8 bytes  "MRDPKG01"
format version  u32 LE
manifest length u32 LE
manifest        JSON
blobs           concatenated
```

manifest 里每条记录带 `name / offset / length / crc32`，`source` 里带 OSM 署名、
bbox 与备注——署名跟着数据走，而不是留在没人会拷贝的 README 里。

CRC32 是**损坏检测，不是签名**：它证明字节与打包时一致，不证明是谁打的包。

## 用法

```bash
# 打包（准备阶段，可联网抓数据；打包本身不联网）
cargo run -p manis-motu-pack --bin motu-pack -- build \
  --out var/offline/shanghai.mrdp \
  --graph var/offline/graph-od.bin \
  --prior crates/motu-routing/examples/traffic-prior.json \
  --avoid var/offline/avoid.json \
  --cameras var/offline/cameras.json \
  --bbox 31.13,121.30,31.33,121.62 \
  --note "上海中心城区"

cargo run -p manis-motu-pack --bin motu-pack -- info    var/offline/shanghai.mrdp
cargo run -p manis-motu-pack --bin motu-pack -- verify  var/offline/shanghai.mrdp
cargo run -p manis-motu-pack --bin motu-pack -- extract var/offline/shanghai.mrdp --out /tmp/unpacked
```

条目名固定：`graph.bin`、`traffic-prior.json`、`avoid.json`、`cameras.json`、
`areas.json`、`tiles.pmtiles`。

## App 侧读取

```rust
let package = motu_pack::PackReader::open(path)?;
package.verify()?;                                  // 损坏就在启动时报错
let graph = motu_routing::RoadGraph::from_bytes(package.entry("graph.bin")?)?;
```

图不再需要先落盘：`RoadGraph::from_bytes` 直接吃包里的字节。端到端链路由
`tests/package_graph.rs` 覆盖（OSM JSON → 图 → 打包 → 读回 → 规划）。

## 校验行为

| 情况 | 结果 |
|---|---|
| 正常包 | `verify` 通过，`info` 列出条目与 CRC |
| 任意字节被改 | `verify` 报 `entry X is corrupt (crc mismatch)`，指出是哪一条 |
| 文件被截断 | 条目越界或 CRC 不符，打开/校验失败 |
| magic 不对 | `not an .mrdp package` |
| 版本高于实现 | `unsupported package format version N` |
| 重名条目 | 打包时拒绝 |

## 还没做

- `tiles.pmtiles` 只是预留条目名，PMTiles 生成与 MapLibre 渲染尚未实现
- 没有包间增量更新（现在整包替换）
