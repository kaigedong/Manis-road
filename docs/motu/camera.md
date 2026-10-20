# 电子眼、车辆与路线审计

来源：`com.motu.motumap.navi.camera.CameraScanEngine`、
`com.motu.api.poi.CameraTabType`、车辆表单枚举、`calPathAvoidSec` 的避让逻辑。
实现见 [`crates/motu-core/src/camera.rs`](../../crates/motu-core/src/camera.rs)、
[`vehicle.rs`](../../crates/motu-core/src/vehicle.rs)、
[`route.rs`](../../crates/motu-core/src/route.rs)。

## 电子眼类型（`CameraTabType`）

| id | 枚举 | 含义 |
|----|------|------|
| 0 | `Unknown` | 未知 |
| 1 | `RestrictedEntry` | 闯限行 |
| 2 | `EmergencyLane` | 走应急车道 |
| 3 | `NonMotorLane` | 走非机动车道 |
| 4 | `RedLight` | 闯红灯 |
| 5 | `Speeding` | 超速 |
| 6 | `BusLane` | 公交车道 |
| 7 | `LaneViolation` | 未按车道行驶 |
| 8 | `ReverseDriving` | 逆行 |
| 9 | `TailNumberRestriction` | 尾号限行 |
| 10 | `NoHelmet` | 不带头盔 |
| 11 | `Noise` | 噪音 |
| 12 | `OnSiteTicket` | 现场罚单 |
| 13 | `LinePressing` | 压线 |
| 14 | `ImproperLighting` | 违规用灯 |

`from_id` 对未识别值回退到 `Unknown`（与 Java 一致）。

## 扫描算法

每个导航定位点调用一次 `scan`，顺序如下：

1. **释放**先执行：已提醒的电子眼若「距离比上次增加超过 25 m」或「相对方位
   变化超过 60°」，则退出提醒集合。
2. **扫描节流**：距上次扫描点不足 100 m（`scanIntervalM`）时不扫描；上层
   `allow_scan = false`（例如关闭提醒）同样直接返回。
3. **候选过滤**：只在 0.05° 包围盒内、距离在提醒半径内的电子眼参与；普通
   类型 300 m，`RestrictedEntry` 500 m。
4. **前向锥**：电子眼方位与车头方向夹角不超过 7.5°。
5. **按类型去重**：每种类型只保留最近的一个；同类型相距不超过 50 m 的一并
   标记为已提醒。
6. **结果**：`cameras`、`cameraTypes`、`primaryDistanceM`（最近距离，向上取整
   至少 1）、`clustered`（最远与最近的类型间距 ≤ 50 m，单个电子眼也算「密集」，
   复刻 Java 的 `(farthest - nearest) <= 50`）。
7. **暂停**：`pause` 清空扫描闸门与提醒集合，但保留电子眼列表；`reset` 全部
   清空。

提醒键优先用 `memberByString`，否则用 `lat,lon:typeId`。

## 官方点位同步（`official`）

登录后，地图页的「同步官方点位」按钮调用
`POST v1/poi/searchRulePoiSec`（`Security-Level: Encrypt`，需要 `token`）。
请求体来自 `CameraSearchRuleBody`：

| 字段 | 取值 |
|------|------|
| `lon` / `lat` | 当前地图中心或模拟位置 |
| `cityCode` / `toCityCode` | 设置中的 AMap 城市代码（默认上海 `021`）；空值回退北京 `010` |
| `type` | 城市代码完整时 `"2"`，否则 `"1"` |
| `tabTypes` | 默认 `1..11,13..16`，带尾逗号 |
| `color` | 默认车辆的黄/蓝牌，未知回退 `YELLOW` |
| `plateVal` | 默认车辆号牌，可为空 |
| `source` | 地图图层为 `1` |
| `model` | 巡航时为 `"CRUISE"`，普通地图不带该字段 |

响应 `data` 是 `TrafficCamerasPoiResponse` 数组：`coordinate`、`distance`、
`member`、`memberByString`、`newFlag`、`tabType[]`、`white`。Rust 转换成
`TrafficCamera`，用 `tabType[0]` 作为主类型；坐标无效的项会被丢弃。扫描引擎
仍按 `memberByString`（否则 `lat,lon:typeId`）去重提醒。

官方点位只保留在 Rust 的 `CameraScanEngine` 内存中，地图最多绘制 200 个标记，
其余仍参与扫描；不写入 `motu-core-state.json`，也不随应用分发。重启应用后需要
重新同步。同步前必须登录：未登录时服务端返回
`{"code":20105,"message":"token校验异常：access_token非法 Token无效"}`。

当前是手动一次性同步；原版会随地图视野/巡航自动刷新，自动刷新与巡航模式尚未
重写。

## Web 调试台

[`crates/motu-web`](../../crates/motu-web/README.md) 把同一条链路暴露给浏览器：
Leaflet 使用 OpenStreetMap（WGS-84）瓦片，Rust 在 `/api/sync` 里把地图中心
转换回 GCJ-02 再请求 `searchRulePoiSec`，并把返回点位和详情坐标转换为 WGS-84
后交给页面。登录令牌只保存在本机 Rust 服务，浏览器拿不到；官方点位仍只存在
进程内存中。演示点位同样是虚构坐标，未登录时只能查看演示说明。

## 电子眼详情（`load_camera_detail`）

点击地图上的官方电子眼后，原版 `MotuMapViewModel.loadTrafficCameraDetail`
会并发调用两个加密接口，Rust 侧串行调用并合并结果：

| 用途 | 接口 | 请求体 |
|------|------|--------|
| 基本信息/限行规则 | `POST v1/poi/trafficEyesDetailSec` | `{"trafficEyesId":"<memberByString>"}` |
| 罚单列表 | `POST v1/poi/trafficTicketListSec` | `{"trafficEyesId":"<memberByString>"}` |

两个接口都需要 `Security-Level: Encrypt` 和登录 `token`。详情响应中的
`eyePoiTabs[]`（类型，如「闯限行」）、`ruleInfoList[][]`（抓拍牌照、时间段、
尾号）、`detail`（含抓拍方向等说明）会被扁平化为 `CameraDetail`；罚单响应
`tickets[]` 合并到同一对象的 `tickets` 字段。

Android 侧只负责点击命中、调用 `MotuCore.loadCameraDetail(cameraId, cert)` 和
渲染底部弹窗；未登录或演示点位不会发起请求。当前实现为一次性加载，未做评论、
白名单/避让操作或图片加载。

## 车辆（`Vehicle`）

| 字段 | 取值 |
|------|------|
| 号牌颜色 `plateColor` | 黄牌 `1`、蓝牌 `2`、未知 `0` |
| 动力 `powerType` | 汽油 `1`、电动 `2`、混动 `3`、未知 `0` |
| 变速 `transType` | 自动 `1`、手动 `2`、半自动 `3`、未知 `0` |

号牌校验：先去掉空格、`-`、`·` 并转大写；长度 7 或 8；首位为省份简称（汉字）、
第二位为 ASCII 字母、其余为字母数字。未知颜色/动力/变速和非法排量给出警告而非
错误。`normalize_default` 保证列表最多一辆 `isDefault`。

Android 车辆页只编辑默认车辆：保存时替换原默认项（没有则新增），不追加重复项。

## 路线审计（`audit_route`）

输入路线折线、避让区、电子眼与电子眼缓冲距离：

- 避让区：路线顶点落在多边形内、线段与边相交，或边界距离 ≤ 0.5 m，记为命中。
  `kind` 为 `User(0)`、`Platform(1)`、`White(2)`；白色区域不算阻断。
- 电子眼：到路线折线的最近距离 ≤ 缓冲距离则命中，返回最近顶点下标。
- `StraightLinePlanner` 是离线兜底 `RoutePlanner`，按 30 km/h 估算时长，并在
  直线穿过非白色避让区时标记 `degraded`。线上路径规划属于 AMap，未重写。
