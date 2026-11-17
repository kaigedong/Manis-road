import fs from 'node:fs';
import path from 'node:path';
import {spawn} from 'node:child_process';
import {createHash} from 'node:crypto';
import {fileURLToPath} from 'node:url';

const root = path.dirname(fileURLToPath(import.meta.url));
const [name = 'smoke', model = 'gpt-6-astra', effort = 'high', task = 'pong', configMode = 'inherit'] = process.argv.slice(2);
const prompts = {
  pong: 'Reply with exactly the single word: pong. Do not call tools or read files.',
  reasoning: 'Without using tools or reading files: count the permutations of 1,2,...,8 in which no two even numbers are adjacent. Give the result and a short derivation.',
  coding: 'Without using tools or reading files: review this Rust function for all arithmetic and boundary errors, then provide a corrected implementation and four test cases: fn midpoint(a: i64, b: i64) -> i64 { (a + b) / 2 }. The intended result is floor((a+b)/2) for every possible i64 input. Explain correctness briefly.',
  long: 'Do not use tools or read files. Design an in-memory LRU cache in Rust using only the standard library, with O(1) average get/put. Explain ownership, eviction and invariants, then give a compact implementation and test examples. Aim for 700-1000 words.',
};
if (!prompts[task] || !/^[a-zA-Z0-9_-]+$/.test(name)) throw new Error('Invalid test parameters');
const dest = path.join(root, 'runs', name);
fs.mkdirSync(dest, {recursive:true, mode:0o700});
const eventsFile = path.join(dest, 'events.jsonl');
if (fs.existsSync(eventsFile)) throw new Error('Refusing to overwrite evidence');
const report = {name, model, effort, task, configMode, startedAt:new Date().toISOString(), prompt:prompts[task], cliVersion:'0.156.0', responses:[], handshakes:[], connections:[], eventCounts:{}, errorCodes:[], observations:[], parseErrors:0, rawLogsPersisted:false};
const responseMap = new Map();
const allowHeaders = /^(openai-model|x-openai-model|x-request-id|x-oai-request-id|x-reasoning-included|x-codex-safety-buffering-(enabled|faster-model)|x-codex-(primary|secondary)-(used-percent|window-minutes|reset-after-seconds)|date|server|content-type|sec-websocket-extensions)$/i;
function headers(h) { return Object.fromEntries(Object.entries(h || {}).filter(([k]) => allowHeaders.test(k))); }
function usage(u) {if(!u) return u;const out={};for(const key of ['input_tokens','input_tokens_details','output_tokens','output_tokens_details','total_tokens','codex_rollout_budget_units']) if(u[key]!==undefined) out[key]=u[key];return out;}
function emit(record) {fs.appendFileSync(eventsFile, JSON.stringify({observedAt:new Date().toISOString(),...record})+'\n', {mode:0o600});}
function observeEvent(e, transport) {
  if (!e || typeof e.type !== 'string') return;
  report.eventCounts[e.type] = (report.eventCounts[e.type] || 0)+1;
  const selected = {transport, type:e.type};
  if(e.response_id) selected.response_id=e.response_id;
  if(e.headers) {selected.headers=headers(e.headers);selected.headerNames=Object.keys(e.headers);}
  if(e.safety_buffering) selected.safety_buffering=e.safety_buffering;
  if(e.metadata?.openai_verification_recommendation) selected.verification=e.metadata.openai_verification_recommendation;
  const r=e.response;
  if (r && ['response.created','response.in_progress','response.completed','response.failed','response.incomplete'].includes(e.type)) {
    selected.response={};
    for(const key of ['id','model','created_at','status','service_tier','incomplete_details']) if(r[key]!==undefined) selected.response[key]=r[key];
    if(r.usage!==undefined) selected.response.usage=usage(r.usage);
    if(r.reasoning) selected.response.reasoning={effort:r.reasoning.effort};
    if(r.error) selected.response.error={code:r.error.code,type:r.error.type};
    if(r.headers) selected.response.headers=headers(r.headers);
    const key=r.id || 'unknown';
    const prior=responseMap.get(key) || {id:key,events:[],models:[]};
    prior.events.push(e.type);
    if(r.model && !prior.models.includes(r.model)) prior.models.push(r.model);
    Object.assign(prior,selected.response);
    if(r.output) prior.outputItems=r.output.length;
    responseMap.set(key,prior);
  }
  if(e.type==='error') {selected.error={code:e.error?.code,type:e.error?.type,status:e.status};report.errorCodes.push(selected.error);}
  if(selected.response || selected.headers || selected.safety_buffering || selected.verification || selected.error) emit(selected);
}
function parseObject(s) {
  const begin=s.indexOf('{'), end=s.lastIndexOf('}');
  if(begin<0 || end<begin) return null;
  try {return JSON.parse(s.slice(begin,end+1));} catch {return null;}
}
function line(s,channel) {
  s=s.replace(/\x1b\[[0-9;]*m/g,'');
  const received = s.includes('Received message');
  const sse = s.includes('SSE event:');
  if(received || sse) {
    const value=parseObject(s);
    if(value) observeEvent(value,received?'websocket':'sse');
    else {report.parseErrors++; if(report.observations.length<5) report.observations.push({kind:'unparsed_event_shape',shape:s.replace(/"(?:\\.|[^"\\])*"/g,'"<redacted>"').slice(0,180)});}
    return;
  }
  if(s.includes('successfully connected to websocket:')) {
    const h=parseObject(s.slice(s.indexOf('headers:')+8));
    const safe=headers(h);
    report.handshakes.push({headerNames:h?Object.keys(h):[],headers:safe});
    emit({type:'websocket.upgrade',headers:safe,headerNames:h?Object.keys(h):[]});
  } else if(s.includes('connecting to websocket:')) {
    const m=s.match(/wss:\/\/[^\s,]+/);
    if(m) {const u=new URL(m[0]);const target=u.origin+u.pathname;report.connections.push(target);emit({type:'connection.target',target});}
  }
  if(s.includes('responses websocket timing')) {
    const m=s.match(/\bmodel="([^"]+)"/), w=s.match(/\bwarmup=(true|false)/);
    if(m&&w) emit({type:'client.request_timing',requestedModel:m[1],warmup:w[1]==='true'});
  }
  // Persist only event categories and numeric usage from the CLI JSON output.
  if(channel==='stdout') {try {const e=JSON.parse(s);if(e.type==='turn.completed') {report.cliUsage=e.usage;emit({type:'cli.turn.completed',usage:e.usage});} else if(e.type==='turn.failed') {report.observations.push({kind:'cli.turn.failed'});}} catch {}}
  for(const code of ['server_is_overloaded','usage_limit_reached','model_not_found','invalid_request_error']) if(s.includes(code) && !report.errorCodes.includes(code)) report.errorCodes.push(code);
}
const args=['exec','--ephemeral','-s','read-only','--skip-git-repo-check','--color','never','--json','-C',path.join(root,'work'),'-m',model,'-c',`model_reasoning_effort="${effort}"`,'-c','service_tier="default"','-c','notify=[]'];
if(configMode==='clean') args.push('--ignore-user-config');
args.push(prompts[task]);
report.command={binary:'codex',args};
const configPath=path.join(process.env.HOME,'.codex/config.toml');
report.configSha256=fs.existsSync(configPath)?createHash('sha256').update(fs.readFileSync(configPath)).digest('hex'):null;
const env={...process.env,RUST_LOG:'tungstenite::protocol=trace,tungstenite::protocol::frame=off,codex_api::endpoint::responses_websocket=info,codex_api::responses_websocket_timing=trace,codex_api::sse::responses=trace'};
const child=spawn('codex',args,{env,stdio:['ignore','pipe','pipe'],detached:true});
let timedOut=false;
const timer=setTimeout(()=>{timedOut=true;try{process.kill(-child.pid,'SIGTERM');}catch{}},240000);
for(const channel of ['stdout','stderr']) {let buffer='';child[channel].setEncoding('utf8');child[channel].on('data',chunk=>{buffer+=chunk;let pos;while((pos=buffer.indexOf('\n'))>=0){line(buffer.slice(0,pos),channel);buffer=buffer.slice(pos+1);}});child[channel].on('end',()=>{if(buffer) line(buffer,channel);});}
child.on('error',e=>{report.spawnError=e.code;});
child.on('close',(code,signal)=>{
  clearTimeout(timer);report.endedAt=new Date().toISOString();report.exitCode=code;report.signal=signal;report.timedOut=timedOut;report.responses=[...responseMap.values()];
  report.modelMismatch=report.responses.some(r=>r.models.some(m=>m!==model&&!m.startsWith(model+'-20')));
  report.completedResponses=report.responses.filter(r=>r.events.includes('response.completed')).length;
  fs.writeFileSync(path.join(dest,'report.json'),JSON.stringify(report,null,2)+'\n',{mode:0o600});
  console.log(JSON.stringify({name,requested:model,effort,exitCode:code,timedOut,responses:report.responses.map(r=>({id:r.id,models:r.models,status:r.status,usage:r.usage})),handshakes:report.handshakes,connections:report.connections,parseErrors:report.parseErrors,observations:report.observations}));
});
