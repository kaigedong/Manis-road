# 上海电子眼记录：资料收集试点

现阶段**只做资料目录，不做导航、自动绕行或公开电子眼地图**。独立 Rust crate：`src/model.rs` 描述来源、点位、坐标与照片；`src/catalog.rs` 做纯 Rust JSON 解析与规则校验、并通过 Tokio 异步读取原生文件；`src/main.rs` 是 Tokio 命令行检查工具。浏览器 WASM crate 复用纯解析模块。以后可以在这些模块上添加来源适配器、人工审核与 API，不需要先绑定某家地图商。

## 已收集到什么

`crates/camera-catalog/data/shanghai-cameras.json` 有 **2 条政府公告来源、3 条从徐汇区公告搜索摘要提取的候选地点描述、0 条已审核精确点位**。宝山区公告的 94 套设备仅记为**待逐条核对的来源**，没有凭图片标题虚构 94 条记录。所有坐标、现场照片、**照片拍摄日期**、是否针对摩托车均为未知，不能因为没显示点位就判断路上没有监控。

- [徐汇区政府公告（2025-08-28）](https://www.shanghai.gov.cn/nw17239/20250828/094eed2f24c54f1b8184b81031f80d15.html)
- [宝山区政府公告（2026-01-07）](https://www.shanghai.gov.cn/nw17239/20260107/e808dce2edfb485f8e6ba4f0ede18fef.html)

本地环境访问上海政府网站原文时被 fake-IP 安全限制拦截；检索摘要曾对同一公告的其他点位给出互相冲突的路名，因此暂未录入该点位。**现存的 3 条也不是原文已核实记录，必须人工核对公告表格**。不可直接用于地图显示或道路决策。

## 使用

```bash
cargo test --offline -p manis-camera-catalog
cargo run --offline -p manis-camera-catalog -- validate crates/camera-catalog/data/shanghai-cameras.json
cargo run --offline -p manis-camera-catalog -- stats crates/camera-catalog/data/shanghai-cameras.json
cargo run --offline -p manis-camera-catalog -- points crates/camera-catalog/data/shanghai-cameras.json
```

`points` 只输出已核对且有精确 GCJ-02 坐标的记录，目前正确结果是 `[]`。`sources[].published_on` 是**公告发布日期**，绝非照片拍摄日期；只有在有权展示的照片与可核对的日期证据时才填写 `photo_url` / `photo_taken_on`。

## 下一步人工核验流程

1. 核对公告原件/图片：逐条确认道路名称、方向、设备类别、发布日期，保留来源链接；未经核验状态维持 `candidate`。
2. 取得地理位置证据，在允许的地图/测绘资料上人工审核 GCJ-02 坐标和 `coordinate.reference`；**不要通过模糊地名自动生成伪精度点位**。
3. 分别核对是否针对摩托车抓拍；未知则 `applies_to_motorcycle: null`。若有自有/获许可现场照片，审核隐私、展示权和实际拍摄日期，勿拿公告日期代替照片日期。
4. 审核完成再置 `evidence_level: reviewed`，记录审核说明；确认公开展示和再利用的法律条件之后才能接入产品。后续需要补审核人/审核时间与撤销、纠错工作流。

初期目录只是样本，不代表上海全部监控设备。数据应用于公开地图前，仍须核对地图服务合规及来源的再利用条件。
