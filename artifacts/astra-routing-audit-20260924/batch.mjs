import {spawn} from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const root=path.dirname(fileURLToPath(import.meta.url));
const matrix=[
 ['luna-control-before','gpt-5.6-luna','high','pong','clean'],
 ['astra-low-pong','gpt-6-astra','low','pong','clean'],
 ['astra-high-coding','gpt-6-astra','high','coding','inherit'],
 ['astra-xhigh-coding','gpt-6-astra','xhigh','coding','clean'],
 ['astra-max-reasoning','gpt-6-astra','max','reasoning','clean'],
 ['astra-high-long','gpt-6-astra','high','long','clean'],
 ['astra-high-repeat','gpt-6-astra','high','pong','inherit'],
 ['luna-control-after','gpt-5.6-luna','high','reasoning','clean'],
];
fs.writeFileSync(path.join(root,'matrix.json'),JSON.stringify(matrix,null,2)+'\n');
for(const test of matrix){
 console.log(JSON.stringify({starting:test[0],at:new Date().toISOString()}));
 const code=await new Promise(resolve=>{const c=spawn(process.execPath,[path.join(root,'probe.mjs'),...test],{stdio:'inherit'});c.on('close',resolve);});
 if(code!==0) {console.log(JSON.stringify({stopped:test[0],exitCode:code}));process.exitCode=1;break;}
 const r=JSON.parse(fs.readFileSync(path.join(root,'runs',test[0],'report.json')));
 if(r.timedOut||r.responses.length===0){console.log(JSON.stringify({stopped:test[0],reason:'capture_incomplete'}));break;}
}
