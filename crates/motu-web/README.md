# manis-motu-web

Local web debugger for the Motu electronic-eye client. It serves a Leaflet map
of Shanghai and proxies login, official camera sync and camera-detail calls
through the Rust core in `manis-motu-core`.

The browser only renders JSON returned by this server. Signing, encryption,
session handling and `GCJ-02` ↔ WGS-84 conversion stay in Rust.

## Run

```bash
cargo run -p manis-motu-web
```

Then open <http://127.0.0.1:8787>.

The server binds to localhost by default and keeps its session in
`var/motu-web/motu-core-state.json` (git-ignored).

Official camera points, camera details and images are cached on disk in
`var/motu-web/cache.sqlite` (git-ignored). The cache is cache-first: a request
that can be answered from SQLite never reaches Motu, which keeps both latency
and official service load down. `POST /api/cache/clear` drops every row and the
UI footer has a "清空缓存" button.

The cache is personal debugging data, not bundled content: it is never
committed, never redistributed, and only exists under your own account.

## Environment

| Variable | Default | Purpose |
| --- | --- | --- |
| `MOTU_WEB_ADDR` | `127.0.0.1:8787` | Listen address. Keep localhost for personal debugging. |
| `MOTU_WEB_DATA_DIR` | `var/motu-web` | Directory for the JSON state/session file and `cache.sqlite`. |
| `MOTU_WEB_CACHE_TTL_SECS` | `0` (never expires) | How long a cached detail/area layer is served without a refresh. `0` keeps ticket history forever; set a positive number to expire. |
| `MOTU_WEB_SYNC_TTL_SECS` | `604800` (7 days) | How long a recorded sync area covers later viewports; `0` always queries Motu. |
| `MOTU_WEB_OFFLINE` | unset | `1`/`true`/`yes`/`on` disables every upstream request; only local caches and the road graph are used. |
| `MOTU_WEB_GRAPH` | `var/offline/graph.bin` | Binary road graph built by `motu-graph build`; missing means `/api/route` returns 503. |
| `MOTU_WEB_API_BASE_URL` | core default | Override the Motu API base URL. |
| `MOTU_WEB_TILE_URL` | OpenStreetMap | WGS-84 raster tile template. |
| `MOTU_WEB_TILE_ATTRIBUTION` | OpenStreetMap | Attribution HTML shown on the map. |
| `MOTU_SIGNATURE_MD5` | empty | Uppercase MD5 of the client signing certificate. |
| `MOTU_CERT_DER` | empty | Path to a DER certificate; its MD5 is used when `MOTU_SIGNATURE_MD5` is unset. |

`MOTU_SIGNATURE_MD5` / `MOTU_CERT_DER` feed the `safe-env-token` header. Some
official endpoints reject requests without it.

Resolution order (first match wins):

1. `MOTU_SIGNATURE_MD5` (32 hex chars, any case; an explicitly empty value
   disables the header);
2. `MOTU_CERT_DER`, a DER certificate whose MD5 is computed;
3. `<data dir>/signing-cert.der`, the same DER fallback, so a locally extracted
   certificate survives restarts without an `export`. `var/` is git-ignored;
   never commit a certificate or its MD5.

`GET /api/config` reports `signatureConfigured` and `signatureSource` (the key
or file the MD5 came from, never the MD5 itself).

For the Android debug keystore:

```bash
keytool -exportcert -alias androiddebugkey \
  -keystore ~/.android/debug.keystore \
  -storepass android -keypass android -file /tmp/debug.cer
export MOTU_CERT_DER=/tmp/debug.cer
# or, equivalently:
# export MOTU_SIGNATURE_MD5=$(md5 -q /tmp/debug.cer | tr '[:lower:]' '[:upper:]')
```

Never commit a real certificate or MD5.

## HTTP API

All endpoints are local and return JSON. `POST` bodies use camelCase.

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/api/health` | Liveness probe. |
| `GET` | `/api/config` | Map center, tile URL/attribution and signature status. |
| `GET` | `/api/state` | Login state, city code and cache counters. |
| `GET` | `/api/cameras` | Cached camera snapshot (`demo` when the cache is empty, otherwise `cache`). Optional `?bbox=south,west,north,east` in WGS-84. |
| `POST` | `/api/login` | `{ "phone", "password" }`; persists the session, never the password. |
| `POST` | `/api/logout` | Best-effort server logout, then clears the local session. The cache is kept. |
| `POST` | `/api/sync` | `{ "latitude", "longitude", "cityCode", "south"?, "west"?, "north"?, "east"?, "force"? }`; WGS-84 center, converted to `GCJ-02` before the Motu request. New rows are merged into the cache. When the viewport is inside a fresh recorded sync area and `force` is false, answers from cache with `skipped: true`. |
| `POST` | `/api/detail` | `{ "cameraId", "refresh" }`; `memberByString` from the camera snapshot. Served from cache while fresh; a failed refresh returns the stale row with a `warning`. |
| `GET` | `/api/cache` | Row counts, image bytes, TTL and the database path. |
| `POST` | `/api/cache/clear` | Delete every cached row. |
| `GET` | `/api/areas?refresh=true` | Restricted-area layer (polygons + road segments), cache-first with stale fallback. |
| `POST` | `/api/route` | `{ origin, destination, avoidCameras?, cameraRadiusM?, maxDetourRatio?, maxDetourMinutes? }`; offline A* over the local graph, WGS-84, returns points/distance/duration/steps. With `avoidCameras` it also returns `baseline` and `avoidance` (hits before/after, detour, rejection reason). |
| `GET` | `/api/image?url=...` | Image proxy with an on-disk cache (8 MiB cap, `image/*` only). |

Login tokens stay in the Rust server; the browser never receives them.

## Map and coordinates

Motu (`AMap`) returns `GCJ-02`. OpenStreetMap/Leaflet use WGS-84. The server
converts official camera positions and the detail position to WGS-84 before
sending them to the browser, and converts the map center back to `GCJ-02` for
`/api/sync`. Conversion lives in `motu_core::geo`.

If you point `MOTU_WEB_TILE_URL` at a `GCJ-02` provider such as AMap, markers
will be offset; this debugger assumes WGS-84 tiles.

## Cache behaviour

- **Camera points** are keyed by `memberByString` (falling back to the
  `lat,lon:type` id) and merged across syncs, so dragging the map and syncing
  again accumulates every point you have seen.
- **Sync coverage**: each successful sync records the padded extent of the
  points the server returned in a `sync_regions` row. A later sync whose
  viewport is fully inside that fresh region is answered locally
  (`skipped: true`); `force` or Shift+click bypasses it. Note that the official
  query is centre based: syncing from a different centre can still surface a
  few points the previous area did not include, which is why coverage is stored
  as an extent rather than "city already synced".
- **Offline basemap**: when `MOTU_WEB_OFFLINE` is set the page never creates a
  tile layer. It draws `/api/basemap` segments instead (arterials at low zoom,
  everything at ≥ 15), with ODbL attribution.
- **Camera avoidance** is a hard constraint, not a soft weight:
  `avoidCameras` first removes every road segment passing within 60 m of a
  cached camera (`cameraRadiusM` overrides). If that disconnects the network, a
  50× time penalty is used so the route still avoids as many cameras as
  possible. If even that cannot reduce the known hits, the request fails with
  HTTP 422 instead of silently returning the original road. On success
  `avoidance` reports `camerasBefore/After`, `detourRatio`, `extraSeconds` and a
  reason when some cameras could not be avoided.
- **Turn guidance**: `/api/route` steps carry `startIndex`/`endIndex` into
  `points`; the cruise banner matches the nearest route point and shows the next
  instruction with its remaining distance.
- **GPS cruise**: the "GPS 巡航" toggle follows `watchPosition`, queries
  `/api/cameras/nearby` every 2 s for a 500 m alert, and reroutes locally when
  the position stays > 80 m off the planned route for 4 s. A debug hook lets a
  desktop replay a track:
  `window.dispatchEvent(new CustomEvent("motu:gps", { detail: { latitude, longitude, heading } }))`.
- **Viewport loading**: the page fetches `/api/cameras?bbox=` on `moveend` and
  `zoomend` (debounced 160 ms), so only visible markers are drawn; markers
  outside the viewport stay in SQLite and reappear when you pan back. The badge
  shows the viewport count and the total cache size.
- **Restricted areas** are cached as a single `map_areas` row and refreshed when
  stale or when the UI sends `?refresh=true` (Shift+click on the layer button).
  `mapShowAreaSec` has no request body and does not require a login, but the
  session token is forwarded when present.
- **Cruise / auto refresh**: the "自动刷新" toggle (persisted in
  `localStorage`) sends `/api/sync` with `cruise: true` when the viewport moves
  into an area not yet covered. The backend's coverage check answers locally in
  that case; a real upstream request only happens for uncovered areas, with a
  15 s client-side cooldown. The request body then carries
  `model: "CRUISE"`, matching the app's cruise query. This is viewport-driven,
  not GPS-driven: there is no continuous position streaming in the browser yet.
  Viewports with no camera data answer with the official business error
  `未匹配到对应电子眼数据`; the client treats that as "nothing here", backs off
  for 60 s and does not toast on every pan (a manual sync still reports it).

## Restricted-area layer

`GET /api/areas` returns `{ areas, version, cached, fetchedAtMs }`, where each
area is `{ id, name, shapeType, points }` and `points` are WGS-84 for Leaflet.
The official `posEncode` uses AMap's polyline encoding with **six** decimal
places (`PolylineUtil`, `1e6`), not the Google/route `1e5`; the core exposes
`decode_polyline_scaled` for that. Shapes are `polygon` or `linestring`; unknown
or malformed shapes are skipped without dropping the rest of the layer. The
live layer currently returns 445 shapes (166 polygons, 279 road segments), which
the map draws in red dashed style under the camera markers.
- **Details** are keyed by the wire id and stored with their `GCJ-02` position.
  They are served without a login, converted to WGS-84 on the way out, and by
  default **never expire** (`MOTU_WEB_CACHE_TTL_SECS=0`): ticket history is
  historical data. The UI's "重新获取最新详情" or `refresh: true` still forces a
  request; a positive TTL restores automatic expiry.
- **Images** are fetched at most once per URL and served from SQLite afterwards.
- A failed refresh falls back to stale data with `cached: true` and a `warning`,
  so the panel still renders when the official API is down.
- `cache.sqlite` uses WAL mode; each request opens a short-lived connection.
  The file is created with mode `0600` because ticket photos and reporter
  contact details can contain third-party personal data. Reporter phone numbers
  are masked in the UI; the detail text itself is stored verbatim.

## Demo data

Before the first successful sync the cache is empty and the server returns eight
invented Shanghai markers. The UI labels them as demo data and does not request
official details for them. A successful sync writes rows to the cache; from then
on the map is served from disk, including after a restart.

## Offline mode

`MOTU_WEB_OFFLINE=1` makes the server reject login, logout, sync, detail refreshes,
area refreshes and uncached image downloads with HTTP 503. Cached cameras,
details, areas and images keep working (details/areas add a `warning`), and
`/api/route` runs entirely on the local graph. The page shows an "离线模式" badge,
hides login and disables sync/auto refresh.

Prepare the graph with `cargo run -p manis-motu-routing --bin motu-graph -- build …`;
see [`docs/offline.md`](../../docs/offline.md) for the Overpass query, data scope
and current routing limits.

## Limitations

- Real login, official sync and camera detail require the user's own Motu
  account. This project does not bypass payment, VIP checks or entitlements.
- Official electronic-eye data is not bundled or redistributed. It is only
  cached on this machine under the git-ignored `var/` directory and can be
  deleted at any time.
- Leaflet is vendored under `web/vendor/leaflet` (BSD-2-Clause); see its
  `LICENSE`.
- The web debugger does not implement payment, AMap routing or push.
- Offline basemap tiles are not bundled yet: with no network the Leaflet tile
  layer stays blank (markers, areas and routes still render). The next step is
  drawing roads from the same OSM graph or reading MBTiles/PMTiles.
- Browser geolocation can use OS/Wi-Fi positioning and pauses in the
  background; continuous GNSS navigation still needs the Android foreground
  service.
- The offline basemap has roads only (no buildings, water or place labels) and
  currently covers the extracted bbox (inner Shanghai).
