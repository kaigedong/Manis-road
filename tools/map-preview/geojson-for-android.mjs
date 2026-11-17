#!/usr/bin/env node
/**
 * 把 roads-od.geojson 压成 App 内置底图：只留 highway 属性、坐标 5 位小数。
 * 10.5 MB 左右，随 APK 打包（assets 会被压缩）。
 *
 *   node tools/map-preview/geojson-for-android.mjs \
 *     var/offline/roads-od.geojson android/app/src/main/assets/roads.geojson
 */
import { readFileSync, writeFileSync, statSync } from 'node:fs';

const [input, output] = process.argv.slice(2);
if (!input || !output) {
  console.error('usage: geojson-for-android.mjs <in.geojson> <out.geojson>');
  process.exit(1);
}
const round = (value) => Math.round(value * 1e5) / 1e5;
const source = JSON.parse(readFileSync(input, 'utf8'));
const features = source.features
  .map((feature) => ({
    type: 'Feature',
    properties: { highway: feature.properties.highway ?? '' },
    geometry: {
      type: 'LineString',
      coordinates: feature.geometry.coordinates.map(([lon, lat]) => [round(lon), round(lat)]),
    },
  }))
  .filter((feature) => feature.geometry.coordinates.length >= 2);

writeFileSync(output, JSON.stringify({ type: 'FeatureCollection', features }));
console.log(
  `features=${features.length} size=${(statSync(output).size / 1e6).toFixed(1)}MB -> ${output}`,
);
