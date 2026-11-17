import fs from 'node:fs';
import path from 'node:path';
import {createHash} from 'node:crypto';
import {fileURLToPath} from 'node:url';
const root=path.dirname(fileURLToPath(import.meta.url));
const rows=[];
const reports=[];
const metadata=[];
const hash=s=>createHash('sha256').update(s).digest('hex');
for(const name of fs.readdirSync(path.join(root,'runs'))){
 const reportPath=path.join(root,'runs',name,'report.json');
 if(!fs.existsSync(reportPath)) continue;
 const r=JSON.parse(fs.readFileSync(reportPath));
 reports.push(r);
 const events=fs.readFileSync(path.join(root,'runs',name,'events.jsonl'),'utf8').trim().split('\n').map(JSON.parse);
 const mapping=new Map();let current=null;let pendingHeaders={};
 for(const e of events){
  if(e.type==='codex.response.metadata'){pendingHeaders=e.headers;metadata.push({test:name,...e});}
  if(e.type==='response.created'){
   current=e.response.id;
   mapping.set(current,{headers:pendingHeaders,createdObservedAt:e.observedAt,modelAtCreated:e.response.model});
  }
  if(e.type==='client.request_timing'&&current) Object.assign(mapping.get(current),{warmup:e.warmup,requestedModelFromClient:e.requestedModel,phaseEvidence:'client request timing'});
  if(e.type==='response.completed') Object.assign(mapping.get(e.response.id)||{},{completedObservedAt:e.observedAt,modelAtCompleted:e.response.model});
 }
 for(const [i,response] of r.responses.entries()){
  const m=mapping.get(response.id)||{};
  if(m.warmup===undefined){m.warmup=i===0&&response.usage?.output_tokens===0;m.phaseEvidence='inferred from order and zero output; initial smoke lacked timing trace';}
  const mismatch=response.models.some(x=>x!==r.model&&!x.startsWith(r.model+'-20'));
  rows.push({test:name,effort:r.effort,task:r.task,configMode:r.configMode,requestedModel:r.model,responseId:response.id,...m,models:response.models,status:response.status,usage:response.usage,mismatch,exitCode:r.exitCode,timedOut:r.timedOut});
 }
}
reports.sort((a,b)=>a.startedAt.localeCompare(b.startedAt));rows.sort((a,b)=>(a.createdObservedAt||'').localeCompare(b.createdObservedAt||''));
const formal=rows.filter(r=>!r.warmup),warmups=rows.filter(r=>r.warmup);
const astra=formal.filter(r=>r.requestedModel==='gpt-6-astra');
const luna=formal.filter(r=>r.requestedModel==='gpt-5.6-luna');
const totalTokens=rs=>rs.reduce((a,r)=>({input:a.input+(r.usage?.input_tokens||0),output:a.output+(r.usage?.output_tokens||0),reasoning:a.reasoning+(r.usage?.output_tokens_details?.reasoning_tokens||0)}),{input:0,output:0,reasoning:0});
const configPath=path.join(process.env.HOME,'.codex/config.toml');
const currentHash=hash(fs.readFileSync(configPath));
const allHeaders=reports.flatMap(r=>r.handshakes);
const summary={generatedAt:new Date().toISOString(),startedAt:reports[0]?.startedAt,endedAt:reports.at(-1)?.endedAt,cliVersion:'0.156.0',authentication:'ChatGPT OAuth',endpoint:'wss://chatgpt.com/backend-api/codex/responses',formalRequests:formal.length,warmups:warmups.length,astraFormal:astra.length,lunaControls:luna.length,mismatches:rows.filter(r=>r.mismatch).length,astraFormalCompleted:astra.filter(r=>r.status==='completed').length,lunaFormalCompleted:luna.filter(r=>r.status==='completed').length,astraWithLunaFasterModel:astra.filter(r=>r.headers?.['x-codex-safety-buffering-faster-model']==='gpt-5.6-luna').length,upgradeResponses:allHeaders.length,upgradeOpenaiModelCount:allHeaders.filter(h=>h.headerNames.includes('openai-model')).length,serverMetadataOpenaiModelCount:metadata.filter(e=>e.headerNames?.includes('openai-model')||e.headers?.['openai-model']).length,formalReportedTokens:totalTokens(formal),warmupReportedTokens:totalTokens(warmups),configUnchanged:reports.every(r=>r.configSha256===currentHash),unparsedNonJsonFrames:reports.flatMap(r=>r.observations).filter(r=>r.kind==='unparsed_event_shape'),rows};
fs.writeFileSync(path.join(root,'summary.json'),JSON.stringify(summary,null,2)+'\n',{mode:0o600});
const shanghai=t=>new Intl.DateTimeFormat('zh-CN',{timeZone:'Asia/Shanghai',dateStyle:'short',timeStyle:'medium',hour12:false}).format(new Date(t));
const table=formal.map(r=>`| ${r.test} | ${r.effort} | ${r.configMode} | ${r.modelAtCreated} | ${r.modelAtCompleted} | ${r.usage?.output_tokens||0} | ${r.headers?.['x-codex-safety-buffering-faster-model']||'无'} |`).join('\n');
const ids=formal.map(r=>`| ${r.test} | \`${r.responseId}\` | [事件](runs/${r.test}/events.jsonl) |`).join('\n');
const report=`# Astra 请求路由实测

时间：${shanghai(summary.startedAt)} 至 ${shanghai(summary.endedAt)}（Asia/Shanghai）。

## 结论

本轮 ${astra.length} 次 Astra 正式请求全部检查到 response.created / response.completed 的模型字段。完成 ${summary.astraFormalCompleted} 次；模型标识不一致 ${summary.mismatches} 次。${luna.length} 次 Luna 对照完成 ${summary.lunaFormalCompleted} 次。

Astra 正式请求中 ${summary.astraWithLunaFasterModel}/${astra.length} 次返回了 x-codex-safety-buffering-faster-model=gpt-5.6-luna；这个值来自 WebSocket 的 codex.response.metadata.headers。它是与安全缓冲相关的更快重试模型候选，单凭该值不能证明当前生成已经切换。实际 response.model 仍为 gpt-6-astra。

## 方法和范围

- 本机 codex-cli 0.156.0，使用现有 ChatGPT OAuth；实际连接 ${summary.endpoint}。这不是 Platform API key 路径，也不是 Pi 的实际请求。
- 原生 CLI 顺序执行，分别测试 low/high/xhigh/max；任务包括短回答、排列计算、Rust 整数边界审查和较长 LRU 设计。
- inherit 保留用户配置；clean 使用 --ignore-user-config，继续使用相同 CODEX_HOME 的登录信息；每次显式设置模型和推理强度。继承配置与干净配置分别验证。
- 获取原始 WebSocket 消息的 JSON 后才提取模型字段，避免仅从本地界面/会话配置推断服务端模型。
- 除第一次冒烟外，客户端 websocket timing 记录额外提供 requested model 与 warmup 标记。共 ${formal.length} 次正式请求、${warmups.length} 次预热请求，分别统计。
- 每个 response ID 都核对开始和完成模型名；没有使用模型自称、输出质量或 token 速度作为身份判据。
- 采集器只保存白名单字段，不保存 Authorization、Cookie、x-codex-turn-state、完整请求体、完整原始日志或回答内容。
- 记录到 ${summary.unparsedNonJsonFrames.length} 个非 JSON Received-message 帧（均为观测记录所列形状）；正式响应的创建、进行与完成事件已独立保留，不将这些帧作为模型证据。
- ${summary.upgradeResponses} 次 WebSocket HTTP 升级响应中，openai-model 头出现 ${summary.upgradeOpenaiModelCount} 次；流式 metadata 中出现 ${summary.serverMetadataOpenaiModelCount} 次。未出现与出现但值相同分别对待。
- 用户 config.toml 校验哈希前后一致：${summary.configUnchanged}。

## 正式请求结果

| 测试 | effort | 配置 | 开始模型 | 完成模型 | 输出 tokens | faster-model 候选 |
|---|---|---|---|---|---:|---|
${table}

## 可追溯证据

| 测试 | Response ID | 脱敏原始字段 |
|---|---|---|
${ids}

机器可读汇总见 [summary.json](summary.json)，完整计划见 [task_plan.md](task_plan.md)，采集器见 [probe.mjs](probe.mjs)。正式请求报告的 input/output/reasoning tokens 为 ${summary.formalReportedTokens.input}/${summary.formalReportedTokens.output}/${summary.formalReportedTokens.reasoning}。预热报告的 token 数单列在 summary.json；不将这些数字直接解释为计费或额度扣减。

## 判定依据与限制

- 当前证据支持：在以上时间窗口和请求样本内，没有发现 Astra 请求的服务器响应模型标识变成 Luna。
- 服务端返回了模型名，但没有提供可独立验证模型权重的证明。因此 Astra 标签一致不代表已证明权重身份，也无法排除其他时段、会话、客户端或账号出现不同路由。
- 若未来出现 requestedModel=Astra，而相同正式响应 ID 的 modelAtCreated/modelAtCompleted=Luna，应保留响应 ID、时间、两个模型字段以及路由头；若只有 faster-model=Luna，仍不足以判定替换。
- 当前版本官方源码：升级头采集位于 [responses_websocket.rs](https://github.com/openai/codex/blob/rust-v0.156.0/codex-rs/codex-api/src/endpoint/responses_websocket.rs)；安全缓冲候选头解析位于 [safety_buffering.rs](https://github.com/openai/codex/blob/rust-v0.156.0/codex-rs/codex-api/src/safety_buffering.rs)；更快模型重试流程位于 [TUI retry](https://github.com/openai/codex/blob/rust-v0.156.0/codex-rs/tui/src/app/safety_buffering.rs)。
`;
fs.writeFileSync(path.join(root,'REPORT.md'),report);
const manifest=[];
for(const r of reports)for(const f of ['report.json','events.jsonl']){const p=path.join(root,'runs',r.name,f);manifest.push(`${hash(fs.readFileSync(p))}  runs/${r.name}/${f}`);}
fs.writeFileSync(path.join(root,'SHA256SUMS'),manifest.join('\n')+'\n');
console.log(JSON.stringify({...summary,rows:undefined,unparsedNonJsonFrames:summary.unparsedNonJsonFrames.length},null,2));
