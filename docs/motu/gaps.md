# 未重写部分与限制

本页列出与摩途真实行为仍有差距、或刻意不做的部分。

## 原生 `requireSalt`（已确认）

`SecurityIntercepter` 调用 native `requireSalt` 生成 AES-GCM 的 AAD。对官方
4.1.9 运行时调用确认：非空 salt 返回其 UTF-8 字节，空字符串抛
`IllegalArgumentException("salt 不能为空")`。Rust 默认 `Aad::SaltUtf8` 就是
该行为，`Aad::Custom(bytes)` 仅保留给测试/未来变体。

## RSA-OAEP 参数

Java 使用 `RSA/ECB/OAEPWithSHA-256AndMGF1Padding`。平台默认把 MGF1 留在
SHA-1，因此 Rust 必须用 `Oaep::new_with_mgf_hash::<Sha256, Sha1>()`。若用
`Oaep::new::<Sha256>()`，服务端会解出错误的 AES key，表现为
`{"code":400,"msg":"Tag mismatch!"}`。

## 登录、支付与授权资源

- 密码登录协议已恢复；Rust 核心、JNI 与 Android 登录页均已接通。无效测试账号
  能在模拟器上得到官方服务端的业务错误，证明 TLS/签名/JSON 链路可用。
- 官方点位同步已接入加密的 `searchRulePoiSec`；未登录/假 token 已验证到服务端
  业务错误。Android 端结果只进入内存扫描集合；Web 调试台则缓存到本机 SQLite
  （`var/motu-web/cache.sqlite`，git-ignored）：点位按 `memberByString` 合并、
  详情按 TTL 复用、图片只下载一次，刷新失败时回退旧缓存并标注 `warning`。
  当前仍是手动同步，但 Web 端已按视野用 `bbox` 读本地缓存，并按记录范围跳过
  已覆盖的重复同步（Shift 可强制）；「自动刷新」开关会在进入未覆盖视野时自动
  同步（带 `model: "CRUISE"`、15 秒冷却），但仍是**视野驱动**，没有连续定位流
  （浏览器端未做 GPS watch），也没有原生导航式的自动前移。
- 官方限行区域图层 `mapShowAreaSec` 已接入：Rust 解码 AMap 1e6 polyline，Web
  `/api/areas` 缓存整层并转 WGS-84，Leaflet 叠加多边形/路段（实测 445 条）。
  区域数据为全国范围，当前不做按城市裁剪。
- 电子眼详情已接入 `trafficEyesDetailSec` 与 `trafficTicketListSec`：Rust 合并
  类型、限行规则、抓拍说明、上报者字段和罚单列表，Android 点击官方标记后弹窗
  展示；Web 端另外缓存详情并代理罚单照片。真实账号
  返回内容尚未验证；评论、白名单/避让操作、罚单照片与上报入口未重写。
- 真实账号登录、token 刷新、用户信息、VIP 商品/订单与真实点位返回尚未在本仓库
  中执行；需要使用者用自己的账号验证。
- web 调试台的 `safe-env-token` 现在默认带上：从本机 `var/motu-web/signing-cert.der`
  （自行从自有 APK 提取的签名证书 DER）取 MD5，`/api/config` 只暴露来源不暴露值。
  证书与 MD5 均不进入仓库。服务端是否对用户接口强制校验该头，仍需真实账号登录
  后才能定论。
- 支付必须走官方下单与支付渠道。未移植支付宝/微信支付 SDK，也不伪造订单、
  支付回调、设备证明或 VIP 权益。

## AMap 与第三方 SDK

- 路径规划、地图渲染、导航播报属于 AMap SDK，未重写。Rust 侧只提供 `RoutePlanner`
  端口与离线 `StraightLinePlanner`；Android 用 `MapCanvasView` 画示意图。
- 没有 AMap 逆地理编码，官方点位同步的城市代码需在设置中手动填写（默认上海
  `021`，留空回退北京 `010`）。
- 广告、推送、统计等 SDK 不移植。

## 加固与清单

- 原始 APK 带加固（`com.Proxy`、`com.Install`、`com.appsec`），其清单元素索引为
  `0xFFFFFFFF`，常规解码会失败；已恢复的业务来自解包后的真实 DEX，而非壳代码。
- 原版 APK 已在本地模拟器上启动，并通过运行时调用确认了 `requireSalt`；线上
  加密接口已做端到端验证。未产生针对用户账号的抓包。

## 离线（2026-10-21）

- 新增 `crates/motu-routing`：Overpass JSON → 二进制路网图 → A*；摩托车画像排除
  `motorway`、尊重单行与 access/motorcycle 标签。内环 bbox 实测 120k 节点、
  211k 边，跨江 6.1 km 约 8 分钟。
- `motu-web` 增加 `MOTU_WEB_OFFLINE=1`：登录/同步/详情刷新/区域刷新/图片下载
  全部 503，缓存与 `/api/route` 正常；已用独立数据目录 + 浏览器全外网阻断验证。
- 离线底图已接入：`/api/basemap` 从同一张路网图返回分级道路线段，页面在离线模式
  下完全不创建瓦片图层；GPS 巡航已接入 `watchPosition`、本地电子眼提醒和偏航
  重算（80 m/4 s）。已用 Playwright 阻断全部外网验证：瓦片请求 0、外部请求 0。
- 全市路网已构建：1.05M 原始节点 → 780k 路网节点 / 1.5M 边 / 48 MB；摩托画像排除
  高架、快速路与未明确允许的跨江隧道/大桥（走复兴东路隧道下层）。JNI 增加
  `nativeLoadGraph` / `nativeRoute`，Android 增加离线路线与 GPS 巡航按钮。
- Android 前台服务已接入 `CruiseService`（location FGS），模拟器实测锁屏/切后台
  仍持续定位；Web 导出 2399 个点位后，注入 GNSS 可得到「闯限行 · 80 m」提醒。
- 实时下一转向：`RouteStep` 增加 `startIndex`/`endIndex`，Web 巡航横幅与 Android
  地图页都按最近路网点显示「下一步：… · N m」。
- Android 详情缓存已接入：Web `/api/export/details` 导出，Android 本地优先展示，
  在线新详情写回；模拟器未登录/无网络下成功打开「延安高架北侧…」详情。
- Web/Android 路由支持“避开电子眼”：先硬删除 60 m 内道路线段，断开时用 50 倍
  惩罚，仍无法减少命中则 422；不再静默回退原路。
- 未完成：语音播报、屏幕常亮、后台重算路线、转向限制与限行区域参与权重、
  底图建筑/水系/地名、路段投影吸附。

## 验证边界

- 已在本地 arm64 模拟器验证：APK 构建、`libmotu_ffi.so` 加载、仪表盘、
  模拟扫描/释放、路线审计、设置开关、车辆 upsert 与 JSON 持久化。
- 已在线验证公开加密接口 `v1/poi/mapShowAreaSec` 的完整
  AES-GCM/RSA-OAEP/HMAC 流程；最小请求头也能通过。
- 已在模拟器验证登录页、Rust/rustls 官方登录请求、错误展示与本地
  `deviceId`/session JSON 持久化；无效测试账号返回服务端业务错误。
- 已用未登录和假 token 验证官方点位同步的加密请求、服务端 token 错误与 UI
  展示；真实账号返回点位尚未验证。
- 未在真机验证；真实账号登录、token 刷新、支付和真实点位接口尚未做线上
  端到端验证。
- 仓库不含摩途的电子眼/上报数据；演示点位为虚构坐标。

## 数据政策

- 不内置、不随仓库分发摩途电子眼或用户上报数据。
- 官方点位只在用户自有账号与授权范围内按需同步。Android 端只放内存；Web 调试台
  为降低官方服务压力，会把点位、详情和图片缓存在本机 `var/motu-web/cache.sqlite`
  （git-ignored），不批量抓取、不转发、不写入仓库，可随时清空。
- 缓存只服务于当前使用者自己的账号；不做跨账号共享，也不作为数据分发来源。
- 支付必须走官方下单/支付渠道；不伪造订单、不绕过 VIP 或支付。
- 使用者需自行确认所在地对相关数据与接口的法律与授权要求。
