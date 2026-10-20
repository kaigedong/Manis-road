# 接口目录

来源：`com.motu.api.service.*` 的 Retrofit 声明。Rust 侧 `ALL_ENDPOINTS` 与
`endpoint_by_name` 保持不变顺序与命名。

公共头：`version: 4.1.9`、`platForm: 2`，可选 `token`、`deviceId`、
`safe-env-token`；加密接口额外加 `Security-Level: Encrypt`。POST 的
`Content-Type: application/json; charset=utf-8`。

| 方法名 | HTTP | 路径 | 加密 |
|--------|------|------|------|
| `calPathAreaSec` | POST | `v2/poi/calPathByAreaSec` | 是 |
| `calPathAvoidSec` | POST | `v11/poi/calPathAvoidSec` | 是 |
| `calPathQuick` | POST | `v1/poi/calPathQuick` | 否 |
| `loadCameraTabType` | GET | `v2/poi/getTabType` | 否 |
| `loadCameraTabTypeGroup` | GET | `v2/poi/getTabTypeGroup` | 否 |
| `loadCityTabList` | GET | `v2/poi/getTab` | 否 |
| `loadPoiDetailSec` | POST | `v1/poi/poiDetailSec` | 是 |
| `loadPoiShopDetailSec` | POST | `v1/poi/shopDetailSec` | 是 |
| `loadTrafficCameraDetailSec` | POST | `v1/poi/trafficEyesDetailSec` | 是 |
| `loadTrafficTicketListSec` | POST | `v1/poi/trafficTicketListSec` | 是 |
| `mapShowAreaSec` | POST | `v1/poi/mapShowAreaSec` | 是 |
| `searchPoiSec` | POST | `v1/poi/searchPoiSec` | 是 |
| `searchRulePoiSec` | POST | `v1/poi/searchRulePoiSec` | 是 |
| `searchShopSec` | POST | `v1/poi/searchShopSec` | 是 |
| `addTrafficEyes` | POST | `v2/poi/addTrafficEyes` | 否 |
| `addEyesCheck` | POST | `v2/poi/addEyesCheck` | 否 |
| `updateTrafficEyes` | POST | `v1/poi/updateTrafficEyes` | 否 |
| `addUserAvoid` | POST | `v1/poi/addUserAvoid` | 否 |
| `updateUserAvoid` | POST | `v1/poi/updateUserAvoid` | 否 |
| `deleteUserAvoid` | POST | `v1/poi/deleteUserAvoid` | 否 |
| `getUserAvoidList` | GET | `v1/poi/user/userAvoidList` | 否 |
| `getUserAvoidByMap` | GET | `v1/poi/getUserAvoidByMap` | 否 |
| `getUserAvoidDetail` | GET | `v1/poi/userAvoidDetail` | 否 |
| `addAvoidWhite` | POST | `v1/poi/addAvoidWhite` | 否 |
| `updateAvoidWhite` | POST | `v1/poi/updateAvoidWhite` | 否 |
| `deleteAvoidWhite` | POST | `v1/poi/deleteAvoidWhite` | 否 |
| `getAvoidWhiteList` | GET | `v1/poi/user/avoidWhiteList` | 否 |
| `userEyesList` | GET | `v2/poi/user/eyesList` | 否 |
| `userShopList` | GET | `v2/poi/user/shopList` | 否 |
| `userGasList` | GET | `v2/poi/user/gasList` | 否 |
| `getNavigationStatus` | GET | `v2/poi/getNavigationStatus` | 否 |
| `getAllCityTab` | GET | `v2/poi/getAllCityTab` | 否 |
| `getTabTypeMapIcon` | GET | `v2/poi/getTabTypeMapIcon` | 否 |
| `recentTicketsStatus` | POST | `v1/poi/recentTicketsStatus` | 否 |
| `nearbyShopSearch` | POST | `v1/poi/nearbyShopSearch` | 否 |
| `nearbyShopMapSearch` | POST | `v1/poi/nearbyShopMapSearch` | 否 |
| `login` | POST | `v4/user/login` | 否 |
| `register` | POST | `v2/user/register` | 否 |
| `logout` | POST | `v4/user/logout` | 否 |
| `getUserInfoSec` | POST | `v1/user/infoSec` | 是 |
| `getUserCarListSec` | POST | `v1/user/userCarListSec` | 是 |
| `addUserCar` | POST | `v1/user/addUserCar` | 否 |
| `updateUserCar` | POST | `v1/user/updateUserCar` | 否 |
| `delUserCar` | POST | `v1/user/delUserCar` | 否 |
| `setDefaultCar` | POST | `v1/user/setDefaultCar` | 否 |
| `visitorToken` | GET | `v3/user/vistorToken` | 否 |
| `fastLoginToken` | GET | `v1/user/fastLoginToken` | 否 |
| `getLevelInfo` | GET | `v1/user/levelInfo` | 否 |
| `buyVipProduct` | POST | `v2/mem/buy/product` | 否 |
| `getVipProducts` | POST | `v3/mem/query/product/list` | 否 |
| `blackList` | GET | `v1/user/blackList` | 否 |

## 免签名

路径包含下列子串时跳过签名拦截器：

- `poi/getPoiTabs`
- `https://tsapi.amap.com`
