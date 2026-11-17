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

## 图格式

`MOTUGRPH` magic，当前 **v2**：

| 版本 | 内容 | 加载 |
|---|---|---|
| v1 | 节点无信号灯标记 | 仍可加载，`signals` 为空 → 指标报 `None` |
| v2 | 节点后追加每节点 1 字节信号灯标记 | 完整支持 |

重建 v2 图需要带节点标签的 Overpass 抽取（`out body;>;out body qt;`，注意第二个
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
