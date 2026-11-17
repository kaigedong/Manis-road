# motu-routing

离线 OSM 路网图与摩托车 A* 路由。运行时零网络：图在准备阶段构建成紧凑二进制，
启动时载入内存。

## 成本模型

```text
edge cost = duration_s × class_weight(highway)   // 等级偏好
          + turn_penalty(angle)                  // 路口转向代价
          + signal_penalty(node)                 // 到达信号灯路口（v2 图才有）
```

权重全部 ≥ 1.0，这样 A* 的启发函数（`distance / 最大速度`）保持可采纳，搜索仍是最优解；
“偏好小路”通过**惩罚大路**表达，而不是给小路打折。

| 道路等级 | 默认权重 |
|---|---|
| `trunk` / `trunk_link` | 3.6 |
| `primary` / `primary_link` | 2.4 |
| `secondary` / `secondary_link` | 1.5 |
| `tertiary` / `tertiary_link` | 1.1 |
| `residential` / `unclassified` / `living_street` / `service` | 1.0 |

车道与路面（v3 图才有）：干道每多一条车道（>2）成本 +6%；未铺装路面 ×1.6。
`surface` 缺失时保持 `Unknown` 且不加罚——不猜“应该是柏油”。

| 转向（航向变化） | 惩罚 |
|---|---|
| < 25°（直行） | 0 s |
| 25–60° | 1.5 s |
| 60–120° | 5 s |
| 120–170° | 9 s |
| ≥ 170°（调头） | 25 s |

信号灯代价默认 **12 s/个**：一个灯平均等 10–20 s，旧模型完全忽略它，所以会毫不犹豫地把骑行
者送上每 200 m 一个灯的干道。

搜索状态是**有向边**而不是节点：经过路口的花费取决于从哪条路进来，纯节点状态无法表达
转向代价。

## 质量指标

`RouteMetrics` 把验收标准变成数字：

- `arterial_share`：`trunk` + `primary` 里程占比
- `side_street_share`：`tertiary` 及更小道路占比
- `signals_per_km`：每公里信号灯数；**图为 v1 时返回 `None`**（没有信号灯数据，
  报 0 会让阈值假通过）。v2 图逐节点带一个信号灯标记，才返回 `Some`
- `turn_count` / `u_turn_count`

## 用法

```bash
# 构建（准备阶段，需联网取 OSM）
cargo run -p manis-motu-routing --bin motu-graph -- build var/offline/osm-shanghai.json var/offline/graph.bin
cargo run -p manis-motu-routing --bin motu-graph -- info  var/offline/graph.bin

# 规划
cargo run -p manis-motu-routing --bin motu-graph -- route   var/offline/graph.bin 31.2304,121.4737 31.2397,121.4998

# 质量指标（带改造前对照、可按权重实验）
cargo run -p manis-motu-routing --bin motu-graph -- metrics var/offline/graph.bin 31.2304,121.4737 31.2397,121.4998
cargo run -p manis-motu-routing --bin motu-graph -- metrics var/offline/graph.bin 31.2304,121.4737 31.2397,121.4998 --plain
cargo run -p manis-motu-routing --bin motu-graph -- metrics var/offline/graph.bin 31.2304,121.4737 31.2397,121.4998 --primary 2.8
```

实测质量对比见 [`docs/motu/route-quality.md`](../../docs/motu/route-quality.md)。

## 测试

```bash
cargo test -p manis-motu-routing
```

`tests/route_quality.rs` 锁定两个行为：小程序绕行优先于干道捷径、同等代价下不绕路口。

## 时段先验

`traffic.rs` 按时间桶给每类道路乘一个成本系数，`source` 字段区分
`measured` / `assumption`——占位先验不能冒充实测数据。

```bash
motu-graph metrics var/offline/graph-od.bin 31.1951,121.4368 31.3030,121.5150 \
  --hour 8 --prior crates/motu-routing/examples/traffic-prior.json
```

## 禁摩区域

`RouteContext.avoid` 接收多边形，规划期把这些区域内的节点标为不可进入，A* 直接绕开
（不是事后审计）。

```bash
motu-graph metrics var/offline/graph-od.bin 31.1951,121.4368 31.3030,121.5150 \
  --avoid var/offline/avoid-demo.json    # [[[lat,lon], ...]]
```

## 实测先验

`motu-prior build --graph <graph.bin> --trace <ride.jsonl> --out <prior.json>`
把本机记录的骑行轨迹聚合成 `source: "measured"` 的时段先验；样本不足的格子保持
1.0 并列入报告的「丢弃」清单，不从小样本编权重。`--explain` 逐跳说明为什么被采纳
或被丢（时间间隔、速度异常、偏离路网、方向不匹配等）。

## 基准对比

`motu-parity compare --graph <graph.bin> --baseline <baseline.json>` 用本机图重算
官方基线里的每条路线，检查绕行是否超过 +20%，并输出重合度、干道/支路占比与灯/km。
基线文件只存本地，不进仓库。

## 连续重算

`replan.rs` 只做“要不要重算”的决策：偏航 > 50 m 立即重算（3 s 冷却），或候选路线
省 ≥ 8% 且重合度 ≤ 0.7 且距上次切换 ≥ 15 s。`route_overlap()` 用来判断候选是否
只是“同一条路的另一种折线”。

## 图格式

`MOTUGRPH` magic，当前 **v2**：

| 版本 | 内容 | 加载 |
|---|---|---|
| v1 | 节点无信号灯标记 | 仍可加载，`signals` 为空 → 指标报 `None` |
| v2 | 节点后追加每节点 1 字节信号灯标记 | 支持 |
| v3 | way 追加 `lanes` 与 `surface` 各 1 字节 | 完整支持 |

重建 v3 图需要带节点标签的 Overpass 抽取（`out body;>;out body qt;`，注意第二个
`out` 不是 `skel`）：

```bash
curl -sS -m 900 -X POST https://overpass-api.de/api/interpreter \
  --data-urlencode 'data=[out:json][timeout:600];way["highway"](31.15,121.35,31.32,121.60);out body;>;out body qt;' \
  -o var/offline/osm-central-tagged.json
cargo run -p manis-motu-routing --bin motu-graph -- build var/offline/osm-central-tagged.json var/offline/graph.bin
cargo run -p manis-motu-routing --bin motu-graph -- info var/offline/graph.bin   # 打印 signals 计数
```

## 已知缺口

- 没有按时段的常态车流先验
- 没有禁摩区域的规划期硬约束（目前只有事后的 `RoutePlan::crosses` 审计）
- 没有连续重算控制器（偏航重算 / 更优解切换 / 滞回）
