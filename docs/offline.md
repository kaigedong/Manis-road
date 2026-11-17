# 离线模式与离线路由

目标：运行时**不访问任何网络**（GPS/北斗除外），地图、电子眼、限行区域、路径规划和巡航全部走本机数据。只有在“数据准备”阶段允许联网。

## 1. 数据准备（一次性，可联网）

### 路网

来源：OpenStreetMap（ODbL），通过 Overpass 抽取。只抓取指定 bbox 内的 `highway`：

```bash
mkdir -p var/offline
curl -sS -m 2400 \
  -A "manis-road-offline/0.1 (local research)" \
  -H "Accept: application/json" \
  -X POST https://overpass-api.de/api/interpreter \
  --data-urlencode 'data=[out:json][timeout:1800];way["highway"](30.67,120.85,31.88,122.12);out body;>;out skel qt;' \
  -o var/offline/osm-shanghai.json
```

当前抽取的是**上海全域**（`30.67,120.85,31.88,122.12`）：147 MB JSON、260,802 条 way、1,045,733 个节点。内环小范围可用 `31.15,121.35,31.32,121.60` 快速迭代。

构建二进制图（约 3 秒）：

```bash
cargo run -p manis-motu-routing --bin motu-graph -- \
  build var/offline/osm-central.json var/offline/graph.bin
```

实测全市：1,045,733 原始节点 → **780,000** 路网节点、204,556 条路、**1,505,934** 条有向边、**48 MB** 图文件，构建约 8 秒。

查看与试算：

```bash
cargo run -p manis-motu-routing --bin motu-graph -- info var/offline/graph.bin
cargo run -p manis-motu-routing --bin motu-graph -- \
  route var/offline/graph.bin 31.2304,121.4737 31.2397,121.4998
```

### 电子眼、区域、图片

在联网模式下用 Web 调试台登录并同步一次，数据会写进 `var/motu-web/cache.sqlite`；区域图层和点开过的详情、图片同样进缓存。详情（含罚单）默认**永不过期**，需要更新时手动点「重新获取最新详情」。之后可把整个 `var/motu-web` 目录复制到离线机器。

**注意**：官方点位仍然是本机缓存，不随仓库分发；离线机器上只有你账号同步过的数据。

## 2. 离线运行

```bash
MOTU_WEB_OFFLINE=1 cargo run -p manis-motu-web
```

- 所有上游调用（登录、退出、同步、详情刷新、区域刷新、图片下载）直接返回 `503`，提示“离线模式已开启”。
- 已缓存的点位、详情、区域、图片照常返回，响应带 `warning: "离线模式：…来自本地缓存"`。
- `/api/route` 使用 `graph.bin`，不依赖网络。
- 页面显示“离线模式”徽标，隐藏登录、禁用同步与自动刷新。

可用 `MOTU_WEB_DATA_DIR` 指向离线数据目录，`MOTU_WEB_GRAPH` 指向图文件：

```bash
MOTU_WEB_OFFLINE=1 \
MOTU_WEB_DATA_DIR=var/motu-web \
MOTU_WEB_GRAPH=var/offline/graph.bin \
cargo run -p manis-motu-web
```

## 3. 离线底图

断网后不再请求任何瓦片。`/api/basemap?bbox=s,w,n,e&zoom=z` 直接从路网图返回道路线段：

| zoom | 绘制等级 |
| --- | --- |
| ≤ 12 | 主干道（trunk/primary） |
| 13–14 | 加次干道（secondary/tertiary） |
| ≥ 15 | 全部道路（含 residential/service） |

前端按等级分三层绘制（主干道带白色描边），随 `moveend/zoomend` 防抖刷新，单次最多 8000 段。署名按 ODbL 显示 “© OpenStreetMap contributors”。当前只有道路骨架，没有建筑、水系、地名标注——它们不在 `highway` 抽取范围内。

## 4. GPS 巡航

顶栏「GPS 巡航」使用 `navigator.geolocation.watchPosition`：

- 地图跟随定位，蓝色圆点加航向线；
- 每 2 秒查询 `/api/cameras/nearby`（本地缓存），500 m 内显示“前方 X m · 类型”；
- 已规划路线时显示剩余里程；偏离路线超过 80 m 持续 4 秒，自动用当前位置本地重算；
- 全程不访问网络（定位本身除外）。

没有 GPS 的环境可以驱动本地调试事件：

```js
window.dispatchEvent(new CustomEvent("motu:gps", {
  detail: { latitude: 31.2350, longitude: 121.4800, heading: 45 }
}));
```

**边界**：浏览器定位在桌面端可能借助系统 Wi-Fi 定位，不能保证是纯 GNSS；页面切后台后浏览器会暂停定位回调，持续导航需要 Android 前台服务。语音播报、摄像头朝向匹配、转向提示的实时倒计时尚未实现。

## 5. 当前路由能力与边界

- **算法**：A*，启发式按 80 km/h 上限，边权为通行时间。
- **摩托车画像**：
  - 排除 `motorway`/`motorway_link`；
  - 名称含「高架/快速路/高速」、以及 `bridge=yes && layer>=2` 的 trunk 结构（延安高架、南北高架等）；
  - 名称含「隧道/大桥」的跨江结构，除非 OSM 明确 `motorcycle=yes|designated`。因此会走「复兴东路隧道(下层)」，而避开延安东路隧道、南浦大桥等禁摩通道；
  - 尊重 `oneway`、环岛、`access`/`motorcycle`/`motor_vehicle` 拒绝标签，速度取 `maxspeed` 或道路等级默认值。
- **吸附**：取最近路网节点，600 m 内有效；未做“吸附到路段投影”。
- **避开电子眼**是硬约束：`avoidCameras=true` 时先删除距缓存点位 60 m 内的道路
  线段，再跑 A*；如果路网被切断，则退回 50 倍时间惩罚，尽量绕开。仍无法减少
  命中时返回 HTTP 422，不会悄悄回到原路。成功时响应含 `camerasBefore/After`、
  `detourRatio`、`extraSeconds`，前端显示“已知点位 2 → 1，多 1 分钟”。
- **步骤**：按相同路名的连续路段合并，输出左转/右转/直行/掉头/到达，并给出
  `startIndex`/`endIndex`（在 `points` 中的位置）。客户端按当前位置最近点匹配到路段，
  取 `endIndex` 之后的下一步，实时显示「右转进入 X · 120 m」。暂未建模转向限制
  （禁左、禁掉头）。
- **未覆盖**：限行区域避让尚未参与路由权重（区域图层只做展示），坡道/隧道/主辅路区分、实时路况、语音播报都还没有。
- **数据范围**：已覆盖上海全域；OSM 的禁摩标签并不完整，跨江通道按显式标签取舍，仍应以官方通告为准。

## 6. Android 接入

- JNI 新增 `nativeLoadGraph(path)` / `nativeRoute(path, oLat, oLon, dLat, dLon)`，图只加载一次并驻留内存。
- `MapActivity` 新增「离线路线」与「GPS 巡航」：
  - 离线路线用当前坐标规划到陆家嘴（演示目的地），把返回的 `points` 画到 `MapCanvasView`；
  - GPS 巡航由 **前台服务 `CruiseService`** 负责：`GPS_PROVIDER`（仅 GNSS）、持续通知、
    每 1 秒定位并调用本地电子眼扫描；锁屏/切后台后仍运行，结果通过广播回传页面。
- 电子眼数据：Web 端 `GET /api/export/cameras` 导出缓存点位（WGS-84），Android 读取
  `offline/cameras.json` 后调用 `nativeUpdateCameras`，服务启动时也会自行加载。
- 详情缓存：Web 端 `GET /api/export/details` 导出所有已缓存详情（含罚单）；Android
  启动时读入 `offline/details.json`，点击点位优先显示本地详情（无需登录/网络）。
  在线打开的新详情会合并写回该文件，之后离线可看。
- 导航提示：Android 用同样的 `startIndex`/`endIndex` 计算下一转向，在提醒下方显示
  「下一步：… · N m」；后台通知仍只展示电子眼提醒。
- 避开电子眼：Android 地图页有同名复选框，走 `nativeRouteAvoiding`；与 Web 相同的
  80 m / 5 倍代价 / 25% 与 15 分钟上限。模拟器实测注入 GNSS 后显示
  「下一步：出发，沿西藏南路行驶 · 883 m」。

在模拟器上实测（Android 35）：

- 推送 48 MB 图后，「离线路线」显示 **8.5 km · 约 12 分钟**，首步「出发，沿西藏南路行驶」；
- 导出 2399 个点位后，注入 GNSS `31.229742,121.473217`，页面与通知同时显示
  **「提醒：闯限行 · 80 m」**；
- 按 Home 切后台后服务仍为 `isForeground=true`（type=location），通知保留提醒文本；
- 开启飞行模式（`ping 8.8.8.8` 返回 `Network is unreachable`）后复测，离线路线与
  GNSS 提醒均正常。
- 图文件放在应用外部私有目录，便于推送：

```bash
# 首选：应用外部私有目录
adb push var/offline/graph.bin \
  /sdcard/Android/data/com.manisroad.app/files/offline/graph.bin

# 若模拟器/设备上外部目录对应用不可见，可写入内部目录（debug 包）：
adb shell "run-as com.manisroad.app mkdir -p files/offline"
adb shell "run-as com.manisroad.app sh -c 'cat > files/offline/graph.bin'" < var/offline/graph.bin
adb shell "run-as com.manisroad.app sh -c 'cat > files/offline/cameras.json'" < var/offline/cameras.json
# 详情（可选，导出前先在 Web 上点开过目标点位）
curl -s http://127.0.0.1:8787/api/export/details > var/offline/details.json
adb shell "run-as com.manisroad.app sh -c 'cat > files/offline/details.json'" < var/offline/details.json
```

没有图文件时按钮会提示缺失，不会回退到网络。

## 7. 还没做的离线能力

1. **Android 完整巡航**：前台服务、锁屏持续定位、语音播报、偏航提示音。
2. **路由精度**：转向限制、限行区域参与权重、路段投影吸附、主辅路/隧道区分。
3. **底图细节**：建筑、水系、地名标注；可选接入 MBTiles/PMTiles。
4. **导航体验**：实时转向距离、车道指引、屏幕常亮。

## 8. 合规

- OSM 数据遵循 ODbL，使用与再分发需保留署名；本仓库不内置地图数据，`var/` 已被 gitignore。
- 不绕过摩途的登录、付费或 VIP；离线数据来自你自己账号在授权范围内的缓存。
- 高德离线地图是 SDK 内的加密缓存，不能导出给 Leaflet/自有渲染器；本项目不使用其 SDK。
