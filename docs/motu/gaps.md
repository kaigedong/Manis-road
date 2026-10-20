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
  业务错误，结果只进入内存扫描集合，不写本地状态。当前为手动一次性同步，未做
  按地图视野自动刷新/巡航模式；官方限行区域图层 `mapShowAreaSec` 已有模型和
  加密请求验证，但尚未接入地图显示。
- 电子眼详情已接入 `trafficEyesDetailSec` 与 `trafficTicketListSec`：Rust 合并
  类型、限行规则、抓拍说明和罚单列表，Android 点击官方标记后弹窗展示。真实账号
  返回内容尚未验证；评论、白名单/避让操作、罚单照片与上报入口未重写。
- 真实账号登录、token 刷新、用户信息、VIP 商品/订单与真实点位返回尚未在本仓库
  中执行；需要使用者用自己的账号验证。
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
- 官方点位只在用户自有账号与授权范围内按需同步到内存，不批量抓取、不转发、
  不写入仓库；重启后需重新同步。
- 支付必须走官方下单/支付渠道；不伪造订单、不绕过 VIP 或支付。
- 使用者需自行确认所在地对相关数据与接口的法律与授权要求。
