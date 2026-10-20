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
`var/motu-web/motu-core-state.json` (already git-ignored). Official camera rows
are only held in memory and are never written to disk.

## Environment

| Variable | Default | Purpose |
| --- | --- | --- |
| `MOTU_WEB_ADDR` | `127.0.0.1:8787` | Listen address. Keep localhost for personal debugging. |
| `MOTU_WEB_DATA_DIR` | `var/motu-web` | Directory for the JSON state/session file. |
| `MOTU_WEB_API_BASE_URL` | core default | Override the Motu API base URL. |
| `MOTU_WEB_TILE_URL` | OpenStreetMap | WGS-84 raster tile template. |
| `MOTU_WEB_TILE_ATTRIBUTION` | OpenStreetMap | Attribution HTML shown on the map. |
| `MOTU_SIGNATURE_MD5` | empty | Uppercase MD5 of the client signing certificate. |
| `MOTU_CERT_DER` | empty | Path to a DER certificate; its MD5 is used when `MOTU_SIGNATURE_MD5` is unset. |

`MOTU_SIGNATURE_MD5` / `MOTU_CERT_DER` feed the `safe-env-token` header. Some
official endpoints reject requests without it. For the Android debug keystore:

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
| `GET` | `/api/state` | Login state, city code and camera-cache metadata. |
| `GET` | `/api/cameras` | Current in-memory camera snapshot (`demo` or `official`). |
| `POST` | `/api/login` | `{ "phone", "password" }`; persists the session, never the password. |
| `POST` | `/api/logout` | Best-effort server logout, then clears the local session. |
| `POST` | `/api/sync` | `{ "latitude", "longitude", "cityCode" }`; WGS-84 center, converted to `GCJ-02` before the Motu request. |
| `POST` | `/api/detail` | `{ "cameraId" }`; `memberByString` from the camera snapshot. |

Login tokens stay in the Rust server; the browser never receives them.

## Map and coordinates

Motu (`AMap`) returns `GCJ-02`. OpenStreetMap/Leaflet use WGS-84. The server
converts official camera positions and the detail position to WGS-84 before
sending them to the browser, and converts the map center back to `GCJ-02` for
`/api/sync`. Conversion lives in `motu_core::geo`.

If you point `MOTU_WEB_TILE_URL` at a `GCJ-02` provider such as AMap, markers
will be offset; this debugger assumes WGS-84 tiles.

## Demo data

Before the first successful sync the server returns eight invented Shanghai
markers. The UI labels them as demo data and does not request official details
for them. A successful login or logout resets the in-memory cache to demo data;
a successful sync replaces it with official rows.

## Limitations

- Real login, official sync and camera detail require the user's own Motu
  account. This project does not bypass payment, VIP checks or entitlements.
- Official electronic-eye data is not bundled, redistributed or persisted.
- Leaflet is vendored under `web/vendor/leaflet` (BSD-2-Clause); see its
  `LICENSE`.
- The web debugger does not implement payment, AMap routing or push.
