// OSM Overpass JSON (ways + node coords) -> GeoJSON FeatureCollection of LineStrings
import { readFileSync, writeFileSync } from 'node:fs';
const [input, output] = process.argv.slice(2);
const data = JSON.parse(readFileSync(input, 'utf8'));
const coords = new Map();
const ways = [];
for (const el of data.elements) {
  if (el.type === 'node') coords.set(el.id, [el.lon, el.lat]);
  else if (el.type === 'way' && el.tags) ways.push(el);
}
const features = [];
for (const way of ways) {
  const line = way.nodes.map((id) => coords.get(id)).filter(Boolean);
  if (line.length < 2) continue;
  const tags = way.tags;
  features.push({
    type: 'Feature',
    properties: {
      highway: tags.highway ?? '',
      name: tags.name ?? '',
      lanes: Number.parseInt(tags.lanes ?? '0', 10) || 0,
      surface: tags.surface ?? '',
      oneway: tags.oneway === 'yes' ? 1 : tags.oneway === '-1' ? -1 : 0,
      bridge: tags.bridge && tags.bridge !== 'no' ? 1 : 0,
      tunnel: tags.tunnel && tags.tunnel !== 'no' ? 1 : 0,
    },
    geometry: { type: 'LineString', coordinates: line },
  });
}
writeFileSync(output, JSON.stringify({ type: 'FeatureCollection', features }));
console.log(`ways=${ways.length} features=${features.length} -> ${output}`);
