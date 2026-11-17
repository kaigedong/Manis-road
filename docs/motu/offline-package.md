# 离线数据包（`.mrdp`）

运行时零网络的前提是：**所有数据以一个文件到达设备**，并且设备能判断它有没有坏。

## 内容

| 条目 | 必需 | 作用 |
|---|---|---|
| `graph.bin` | 是 | 摩托路网图（v3：节点信号灯 + way 的车道/路面） |
| `traffic-prior.json` | 否 | 时段车流先验（`source` 区分 measured / assumption） |
| `avoid.json` | 否 | 禁摩/限行多边形，规划期硬约束 |
| `cameras.json` | 否 | 电子眼点位（本机自用，不分发） |
| `areas.json` | 否 | 限行区域图层 |
| `tiles.pmtiles` | 待实现 | 离线底图瓦片 |

manifest 里记录 OSM 署名（`© OpenStreetMap contributors (ODbL)`）、bbox 与备注，
署名随数据一起分发。

## 打包与校验

```bash
cargo run -p manis-motu-pack --bin motu-pack -- build \
  --out var/offline/shanghai-od.mrdp \
  --graph var/offline/graph-od.bin \
  --prior crates/motu-routing/examples/traffic-prior.json \
  --avoid var/offline/avoid-demo.json \
  --cameras var/offline/cameras.json \
  --bbox 31.13,121.30,31.33,121.62

cargo run -p manis-motu-pack --bin motu-pack -- verify var/offline/shanghai-od.mrdp
```

实测输出：

```text
package format v1  created 1794887622  entries 4
attribution: © OpenStreetMap contributors (ODbL)
bbox: 31.13000,121.30000 .. 31.33000,121.62000
  graph.bin               7897788 bytes  crc32 915e25a6
  traffic-prior.json          796 bytes  crc32 4326fcf8
  avoid.json                  117 bytes  crc32 b657ff86
  cameras.json             454127 bytes  crc32 67c5ff16
ok: 4 entries verified, 8352828 bytes
```

篡改任意一个字节后：

```text
motu-pack: entry cameras.json is corrupt (crc mismatch)
```

错误会指出**是哪一条坏了**，而不是笼统地说“包坏了”。

## App 侧

```rust
let package = motu_pack::PackReader::open(path)?;
package.verify()?;
let graph = motu_routing::RoadGraph::from_bytes(package.entry("graph.bin")?)?;
```

图直接从包内字节加载，不需要先解包到磁盘——`RoadGraph::from_bytes` 就是为此加的。
端到端链路由 `crates/motu-pack/tests/package_graph.rs` 覆盖：OSM JSON → 图 → 打包
→ 读回 → 规划成功。

## 边界

- CRC32 只防损坏，不防篡改：要防篡改需要签名，当前没有。
- 没有增量更新，换数据是整包替换。
- 包内不含任何摩途账号数据；`cameras.json` 若来自本机缓存，只允许自用、不得分发。
