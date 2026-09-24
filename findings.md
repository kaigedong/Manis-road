# 研究记录：上海摩托车导航

## 已知需求
- 丝滑导航体验（参考 Google Maps）；Rust 后端、Android Kotlin 原生端 + 功能一致的浏览器端；浏览器+App 测试；按监控空白区优先规划，缩放到一定级别展示电子眼点位及照片日期；参考摩途；不强求离线导航，支持在线数据更新；首期上海；车型按摩托车具体型号选择。

## 工作边界
- 仅研究与讨论，不先替用户敲定未确认需求或声称已写定稿 PRD。
- 现有工作目录为空；非 Git 仓库。

## 来源与事实（初步，2026-09-07 检索；以原文和现场标志为准）
- 上海官方《本市道路禁止通行、限制通行规定》按号牌、车型、区域、时段细分，并包含高架/越江限制；不能以简单的“内环内禁摩”代替：[上海市政府原文](https://www.shanghai.gov.cn/nw12344/20200814/0001-12344_28765.html)。搜索摘要对外牌限制互有矛盾，须复核原文。
- 竞品可能指「摩途」（非魔途）；官网展示导航、限行与电子眼等卖点：[摩途](https://www.motumap.com/)。不能据此推定支持全离线导航。
- [高德离线地图](https://lbs.amap.com/api/android-sdk/guide/create-map/offline-map)是底图；[高德官方 FAQ](https://lbs.amap.com/faq/advisory/others/39843)称算路仍联网；[摩托车路线](https://lbs.amap.com/faq/android/navi-sdk/route-plan/44318)需商业开通。百度摩托车导航亦需授权：[百度文档](https://lbsyun.baidu.com/docs/android?title=android-navsdk%2Fguide%2FMotorcycle-nav)。
- [MapLibre Native](https://maplibre.org/maplibre-native/ios/latest/documentation/maplibre/)具有离线区域加载 API，仅提供地图渲染，不自动提供离线路网算路/定位/更新服务。
- 国内互联网地图涉及[地图管理条例](https://www.gov.cn/zhengce/zhengceku/2015-12/14/content_10403.htm)及审图、测绘资质；OSM [ODbL](https://opendatacommons.org/licenses/odbl/1-0/)和署名义务不等于可绕过国内地图合规。自建数据和面向公众发布前须律师/合规评估。
- 进一步查询到[上海市政府外来机动车规定](https://www.shanghai.gov.cn/nw4338/20200813/0001-4338_247.html)提及外牌二轮等外环内禁行；[上海公安现行通告](https://gaj.sh.gov.cn/shga/wzXxfbGj/detail?pa=1b89c2c9c1e7a00e4fcbf784730c1c7f7455ef763f297bf2b757918cdd578f0e2a1ce29182a443324dbb851656a139dbf89cd8d0bb43e938)描述围合区域和边界例外。**不能仅凭摘要判定二者适用次序/精确边界，须法务及交警数据核对。**
- [UniFFI](https://mozilla.github.io/uniffi-rs/next/tutorial/foreign_language_bindings.html)可把 Rust 核心暴露给 Kotlin、Swift；[MapLibre GL JS](https://maplibre.org/maplibre-gl-js/docs/)实现 Web 地图；[MapLibre Native](https://maplibre.org/maplibre-native-ffi/guides/work-offline/)可下载离线区域，但与路由数据不同步需产品自行解决。
- [Playwright 截图对比](https://playwright.dev/docs/test-snapshots)可验证 Web 地图页面；[Android UI Automator](https://developer.android.com/training/testing/other-components/ui-automator)用于 Android 端到端测试；iOS XCTest/XCUITest 需再核对。位置轨迹回放、禁行表、离线断网与更新回滚需作为专门测试。

- [四维图新导航产品](https://lbs.navinfo.com/products/navi.html)宣称具备在线/离线导航，但是否向上海摩托车手机 App 授权**可导出的路网和离线算路**未知，需询价/PoC；[高德协议](https://developer.amap.com/pages/terms/)对存储、缓存、混用数据有限制，不能把高德 SDK 离线包当作自研路由数据源。
- [Apple XCUITest 定位模拟](https://developer.apple.com/documentation/xcode/simulating-location-in-tests)、[Android 模拟器路径回放](https://developer.android.com/studio/run/emulator-extended-controls)可配合真实骑行路段回放测试；手机 UI 测试和 Web 共享的是规则/路线接口与固定夹具，不等同于用 Web 测试证明 App 性能。

## 在线优先方案追加研究（本轮）
- [高德 Web 服务摩托车路线 API](https://lbs.amap.com/api/logistic-service/guide/cycling-service/direction-motor) `/v5/direction/motor` 有摩托算路、多备选路线、避让多边形（需书面授权和商务开通）；Web 地图 JS 路线组件的 Riding 不是摩托车。Android 端导航 SDK 可用但与服务端 Web 算路需核对路线一致性；建议 Rust 服务端统一向同一供应商调用路线，再在 Android/Web 渲染，导航语音/偏航需额外实现或验证 SDK 是否支持导入路线。
- [百度 WebService 摩托车规划](https://lbsyun.baidu.com/docs/webapi?title=directionv2%2Fwebservice-direction%2Fmotorcycle)亦需高级授权，可作为竞品报价。不能混用高德 Web 路由和其他地图商底图而不审核合同：[高德 Web 条款](https://developer.amap.com/pages/law-web-service)。
- [上海公安公开监控地点](https://gaj.sh.gov.cn/shga/wzXxfbGj/detail?pa=1b89c2c9c1e7a00e4fcbf784730c1c7f705158668007a20059e9fbc3ef498663fa3dff9ed46ea3c2b6446ee45deff24ff89cd8d0bb43e938)的查询入口与[区级设备公告样例](https://www.shanghai.gov.cn/nw17239/20260107/e808dce2edfb485f8e6ba4f0ede18fef.html)可用于事实核对，但未发现可再分发的官方完整点位 API/数据集；点位位置与是否针对摩托车抓拍需分别标注。高德导航 SDK [camera info 回调](https://lbs.amap.com/api/android-navi-sdk/guide/navigation-map/navi-info)并非可任意导出的摄像头数据库；照片需合法来源、上传审核/时间证据。
- [摩途商店页](https://apps.apple.com/cn/app/%E6%91%A9%E9%80%94-%E6%91%A9%E6%89%98%E8%BD%A6%E5%AF%BC%E8%88%AA%E6%91%A9%E6%97%85%E8%B7%AF%E4%B9%A6%E8%BD%A8%E8%BF%B9%E5%9C%B0%E5%9B%BE/id1187159082)提到摩友贡献的电子眼照片，**未证实**其显示照片拍摄日期；拍摄日期/上传日期/公告日期应区别展示，无法验证拍摄时间时应显示“未知”。
- [W3C Geolocation](https://www.w3.org/TR/geolocation/)在隐藏页面不提供连续定位；浏览器与 Android 可做到前台功能一致，但息屏/后台导航不能保证浏览器一致。需要用户确认“功能一致”的边界和是否要 Android 后台持续语音。

## 免费使用与收费接口复核
- 「必须商业授权」需限定场景：不能笼统地说所有地图都收费。[高德基础服务配额](https://lbs.amap.com/pages/base_service_price)和 [JS 地图概述](https://lbs.amap.com/api/javascript-api-v2/summary)有按身份/用途的配额或引擎免费说明，但其[摩托车 Web 路线接口](https://lbs.amap.com/api/logistic-service/guide/cycling-service/direction-motor)明确单列收费、试用需工单；[Android 摩托导航](https://lbs.amap.com/api/android-navi-sdk/guide/route-plan/motorcycle-route-plan)同样需商务开通；普通骑行路线不等于摩托路线。
- [百度摩托 Web 路线](https://lbsyun.baidu.com/docs/webapi?title=directionv2%2Fwebservice-direction%2Fmotorcycle)是高级服务、需申请开通；不能把普通地图免费配额当成摩托车授权。
- [OSM 数据许可](https://www.openstreetmap.org/copyright)允许免费商用但要求署名并遵守 ODbL，底图瓦片服务/服务器非免费保障，上海路权数据与[国内地图合规](https://www.gov.cn/zhengce/zhengceku/2015-12/14/content_10403.htm)仍待核查。自行开发 Rust 算路、采集监控点、供图并非零成本。

## 不依赖付费接口的方向（用户明确倾向开发后不付地图/数据商费用）
- [Google 官方地图构建介绍](https://blog.google/products-and-platforms/products/maps/google-maps-101-how-we-map-world/)说明其融合街景/航片、外部权威数据及用户贡献；其自有地图 App 与第三方接入 API 计费不同，[Google Maps Platform](https://developers.google.com/maps/billing-and-pricing)对开发者按 API/SKU 计费。不能据此推断 Google 自身构建、采购、审核与运营成本为零。
- 若禁止长期第三方接口费，可评估 OSM ODbL 开放路网 + 自建底图服务 + Rust 路由/地图与用户投稿监控点；但 [OSMF 公共瓦片政策](https://operations.osmfoundation.org/policies/tiles/)不允许把其免费服务当作生产级无限免费底图，需自托管/合法免费供应并承担服务器带宽；OSM [版权许可](https://www.openstreetmap.org/copyright)与中国[地图管理条例](https://www.gov.cn/zhengce/zhengceku/2015-12/14/content_10403.htm)分别核验。官方摄像头点位公告非带照片日期的完整、可再分发数据库。
- “不付第三方接口费”可作为方案方向；“部署公开地图后没有任何持续开支”不现实。零付费方案上线可行性以法规、数据覆盖质量与运营成本验证为门槛，不承诺 Google 级画质/准确率或电子眼无遗漏。

## 用户提出“高德导航 + 额外 URL 避开道路”研究
- [高德通用导航 URI](https://lbs.amap.com/api/uri-api/guide/travel/route)公开模式 car/bus/walk/ride，不含 motorcycle，也无任意避让道路参数；驾车途经点至多一个。Android [高德 App Scheme](https://developer.amap.com/api/amap-mobile/guide/android/route)也未公开摩托模式/自有避让规则的 URI 参数。调起高德 App 后无法继续在其原生界面插入我方摄像头详情图层。
- [摩托车 Web 路线 API](https://lbs.amap.com/api/logistic-service/guide/cycling-service/direction-motor)确实支持 `avoidroad`（按道路名称仅 1 条）或 `avoidpolygons`（一定数量/面积限制），可拉取多路线；两者同时传时仅 avoidroad 生效，且仍为收费接口，不是可免费加在跳转 URL 上的配置。
- [Android 摩托导航 SDK](https://lbs.amap.com/api/android-navi-sdk/guide/route-plan/motorcycle-route-plan)需收费开通，其公开页面未列出摩托 SDK 直接上传自有避让道路/点位的接口。地图图层可展示合法自有摄像头资料，不等于路由引擎按它重算。途经点等引导技巧不保证摩托路权、避开每个点，需安全检查且不可把驾车/自行车当摩托路线。

## 官方公告试点候选数据（不能视作已审核点位）
- [2025-08-28 徐汇区公告](https://www.shanghai.gov.cn/nw17239/20250828/094eed2f24c54f1b8184b81031f80d15.html)搜索摘录列出罗秀路出龙州路东约160米、龙川北路柳南路南约5米、田林东路柳州路东约280米；关于东安路点位搜索摘录出现**岳阳路/零陵路互相矛盾**，暂不录入。官网原文直取在当前环境遭 fake-IP 安全拦截，以上只做待人工审核候选，坐标/照片日期留空。
- [2026-01-07 宝山区公告](https://www.shanghai.gov.cn/nw17239/20260107/e808dce2edfb485f8e6ba4f0ede18fef.html)提 94 套并在图片中列点位；不依图片未核对内容批量推断精确点位。可先作为待处理来源条目。

## 研究障碍
- 本地 fetch_content 因 fake-IP 198.18.x.x 被安全拦截；搜索返回了源链接及部分摘录，但未直接核对政府原文整篇。法律条款及精确几何边界属待验证，不直接视为确定性实施数据。
