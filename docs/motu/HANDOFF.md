# 交接：换机器继续做

## 仓库里有什么

```
crates/motu-core      上游接口客户端、状态、加密、相机模型
crates/motu-routing   图、A*、成本模型（risk.rs）、时段先验、重算、基准对比
crates/motu-ffi       JNI 桥
crates/motu-pack      .mrdp 离线包 + PMTiles 读取
crates/motu-web       数据准备/调试台
android/              Android App（MapLibre + 导航）
tools/map-preview     瓦片流水线与预览
docs/motu/            设计、实测数据、踩坑记录
```

## 仓库里**没有**什么（都在 `var/`，已 gitignore）

| 文件 | 怎么来 |
|---|---|
| `var/offline/graph-od.bin` | Overpass 抓 OSM → `motu-graph build`（约 12 万节点 / 22 万边 / 3,809 信号灯） |
| `var/offline/roads-od.pmtiles` | `tippecanoe` → `pmtiles convert`（见 `offline-map-tiles.md`） |
| `var/offline/cameras.json` | 官方接口抓的本机缓存（2,399 条） |
| `var/offline/traffic-prior*.json` | 假设版在 `crates/motu-routing/examples/`；实测版用 `motu-prior` 从轨迹生成 |
| `var/offline/*.mrdp` | `motu-pack build` 打包 |

工具依赖（macOS）：`brew install tippecanoe pmtiles`；Android 需要 SDK 35 + NDK 27 + `cargo-ndk`。

## 装到机器上

```bash
./android/build-rust.sh debug          # 生成 libmotu_ffi.so
cd android && ./gradlew assembleDebug  # 产出 app-debug.apk
adb push var/offline/graph-od.bin /sdcard/Android/data/com.manisroad.app/files/offline/graph.bin
adb push var/offline/roads-od.pmtiles .../files/offline/tiles.pmtiles
adb push var/offline/cameras.json     .../files/offline/cameras.json
# 可选：traffic-prior.json / avoid.json
```

App 入口：仪表盘 →「离线底图（MapLibre）」。

## 当前状态（2026-11-17）

已验证：
- 路线质量：干道 2%、支路 58%（跨江 OD 拖低）、灯 1.6/km、绕行中位 +4%
- 偏航重算：真机 5/5 次、最慢 101 ms、越界切换 0 次
- 离线包、PMTiles 渲染、电子眼图层、真实 GPS 导航、实测先验链路

未完成 / 已知问题：
1. **电子眼避让验收未达标**：中山公园→豫园 0 → 1 个。命中在起终点连接段，见 route-quality 第 19 轮
2. `route_report_with_context` 已收 risk，但旧的 `CameraDeterrent` 仍留在 `route_with` 老路径里，两套逻辑待合并
3. Android 侧本轮改动**未在设备复验**（手机拔线）
4. 没有 release 构建（debug APK 30 MB）
5. 转向文案按顶点索引改过，未真机复验
6. calPath 官方基线未采集（需要摩途账号）——「绕行 ≤20%」目前只能自证

## 下一步建议顺序

1. 修电子眼验收口径（连接段计价 or 换对照组），把 5 条 OD 全部达标
2. release 构建 + 装机复验（含转向文案）
3. 合并两套相机逻辑，删掉 `CameraDeterrent` 老路径
4. 用 `motu-prior` 把实测先验填满（需要真实骑行轨迹）
