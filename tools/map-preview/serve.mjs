#!/usr/bin/env node
/**
 * 静态文件服务，带 HTTP Range 支持——MapLibre 的 pmtiles:// 协议靠 Range 请求
 * 只取需要的字节，缺了 Range 就只会拿到整文件或 200，预览会失败。
 *
 *   node tools/map-preview/serve.mjs <dir> [port]
 *   打开 http://127.0.0.1:<port>/index.html?pmtiles=./roads-od.pmtiles
 */
import { createServer } from 'node:http';
import { createReadStream, statSync } from 'node:fs';
import { extname, join, normalize } from 'node:path';

const dir = normalize(process.argv[2] ?? '.');
const port = Number(process.argv[3] ?? 8089);
const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.pmtiles': 'application/octet-stream',
  '.mvt': 'application/vnd.mapbox-vector-tile',
};

createServer((req, res) => {
  const url = new URL(req.url, 'http://localhost');
  const path = join(dir, normalize(decodeURIComponent(url.pathname)).replace(/^(\.\.[/\\])+/, ''));
  let stat;
  try {
    stat = statSync(path);
  } catch {
    res.writeHead(404).end('not found');
    return;
  }
  if (!stat.isFile()) {
    res.writeHead(404).end('not a file');
    return;
  }
  const type = TYPES[extname(path)] ?? 'application/octet-stream';
  const range = req.headers.range;
  if (range) {
    const match = /bytes=(\d*)-(\d*)/.exec(range);
    const start = match?.[1] ? Number(match[1]) : 0;
    const end = match?.[2] ? Number(match[2]) : stat.size - 1;
    if (start >= stat.size || end >= stat.size || start > end) {
      res.writeHead(416, { 'Content-Range': `bytes */${stat.size}` }).end();
      return;
    }
    res.writeHead(206, {
      'Content-Type': type,
      'Content-Range': `bytes ${start}-${end}/${stat.size}`,
      'Accept-Ranges': 'bytes',
      'Content-Length': end - start + 1,
      'Access-Control-Allow-Origin': '*',
    });
    createReadStream(path, { start, end }).pipe(res);
    return;
  }
  res.writeHead(200, {
    'Content-Type': type,
    'Content-Length': stat.size,
    'Accept-Ranges': 'bytes',
    'Access-Control-Allow-Origin': '*',
  });
  createReadStream(path).pipe(res);
}).listen(port, '127.0.0.1', () => {
  console.log(`serving ${dir} on http://127.0.0.1:${port}`);
});
