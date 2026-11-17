package com.manisroad.app.core;

import android.content.Context;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.File;

/**
 * Thin Java declaration of the Rust bridge. No business logic lives here:
 * every method forwards to {@code libmotu_ffi.so} and returns a JSON envelope.
 */
public final class MotuCore {
    static {
        System.loadLibrary("motu_ffi");
    }

    private static CoreResult initResult;

    private MotuCore() {
    }

    private static native String nativeInit(String dataDir);

    private static native String nativeLoadState();

    private static native String nativeSaveState(String stateJson);

    private static native String nativeDefaultVehicle();

    private static native String nativeUpdateCameras(String camerasJson);

    private static native String nativeScan(double latitude, double longitude, double bearing,
                                            boolean allowScan);

    private static native void nativePause();

    private static native void nativeResetScan();

    private static native String nativeBuildRequest(String endpoint, String queryJson,
                                                    String bodyJson, String headersJson,
                                                    long timestampMs);

    private static native String nativeSafeEnvToken(String deviceId, long seconds,
                                                    byte[] signingCertificate);

    private static native String nativeLogin(String phone, String password,
                                             byte[] signingCertificate);

    private static native String nativeSession();

    private static native String nativeDeviceId();

    private static native String nativeLogout();

    private static native String nativeSyncOfficialCameras(double latitude, double longitude,
                                                           String cityCode,
                                                           byte[] signingCertificate);

    private static native String nativeLoadCameraDetail(String cameraId,
                                                        byte[] signingCertificate);

    private static native String nativeVehicleValidate(String vehicleJson);

    private static native String nativeRouteAudit(String pointsJson, String areasJson,
                                                  String camerasJson, double bufferMeters);

    private static native String nativeLoadGraph(String graphPath);

    private static native String nativeRoute(String graphPath, double originLat, double originLon,
                                             double destinationLat, double destinationLon);

    private static native String nativeRouteAvoiding(String graphPath, double originLat,
                                                     double originLon, double destinationLat,
                                                     double destinationLon, String camerasJson,
                                                     double radiusM, double maxDetourRatio,
                                                     double maxDetourMinutes);

    private static native String nativePlanStraightLine(String requestJson);

    private static native String nativeEncodePolyline(String pointsJson);

    private static native String nativeDecodePolyline(String encoded);

    private static native String nativeParseAmapPositions(String positions);

    private static native String nativeEndpoints();

    private static native String nativeConstants();

    public static CoreResult init(Context context) {
        File directory = new File(context.getFilesDir(), "motu-core");
        return CoreResult.parse(nativeInit(directory.getAbsolutePath()));
    }

    /**
     * Initialise the native store once per process. Activities may be recreated
     * directly by the system, so each one calls this before using the core.
     */
    public static synchronized CoreResult ensureInit(Context context) {
        if (initResult == null || !initResult.ok) {
            initResult = init(context);
        }
        return initResult;
    }

    public static CoreResult loadState() {
        return CoreResult.parse(nativeLoadState());
    }

    public static CoreResult saveState(JSONObject state) {
        return CoreResult.parse(nativeSaveState(state.toString()));
    }

    public static CoreResult defaultVehicle() {
        return CoreResult.parse(nativeDefaultVehicle());
    }

    public static CoreResult updateCameras(JSONArray cameras) {
        return CoreResult.parse(nativeUpdateCameras(cameras.toString()));
    }

    public static CoreResult scan(double latitude, double longitude, double bearing,
                                  boolean allowScan) {
        return CoreResult.parse(nativeScan(latitude, longitude, bearing, allowScan));
    }

    public static void pause() {
        nativePause();
    }

    public static void resetScan() {
        nativeResetScan();
    }

    public static CoreResult buildRequest(String endpoint, JSONObject query, JSONObject body,
                                          JSONObject headers) {
        return CoreResult.parse(nativeBuildRequest(
                endpoint,
                query == null ? "{}" : query.toString(),
                body == null ? "null" : body.toString(),
                headers == null ? "{}" : headers.toString(),
                System.currentTimeMillis()));
    }

    public static CoreResult safeEnvToken(String deviceId, long seconds, byte[] signingCertificate) {
        return CoreResult.parse(nativeSafeEnvToken(deviceId, seconds, signingCertificate));
    }

    /**
     * Blocking official login. Call from a worker thread, never from the UI
     * thread. The Rust core hashes the signing certificate and the password,
     * signs the request and stores the returned session.
     */
    public static CoreResult login(String phone, String password, byte[] signingCertificate) {
        return CoreResult.parse(nativeLogin(phone, password, signingCertificate));
    }

    public static CoreResult session() {
        return CoreResult.parse(nativeSession());
    }

    public static CoreResult deviceId() {
        return CoreResult.parse(nativeDeviceId());
    }

    public static CoreResult logout() {
        return CoreResult.parse(nativeLogout());
    }

    /**
     * Blocking official camera sync for a signed-in account. Call from a
     * worker thread. The Rust core fetches the security config, calls
     * {@code v1/poi/searchRulePoiSec}, and replaces the in-memory scan-engine
     * cameras; the returned points are not persisted.
     */
    public static CoreResult syncOfficialCameras(double latitude, double longitude,
                                                 String cityCode, byte[] signingCertificate) {
        return CoreResult.parse(nativeSyncOfficialCameras(
                latitude, longitude, cityCode, signingCertificate));
    }

    /**
     * Blocking camera detail lookup. Call from a worker thread. The Rust core
     * calls the encrypted detail and ticket endpoints and returns one merged
     * JSON object; nothing is persisted.
     */
    public static CoreResult loadCameraDetail(String cameraId, byte[] signingCertificate) {
        return CoreResult.parse(nativeLoadCameraDetail(cameraId, signingCertificate));
    }

    public static CoreResult validateVehicle(JSONObject vehicle) {
        return CoreResult.parse(nativeVehicleValidate(vehicle.toString()));
    }

    public static CoreResult auditRoute(JSONArray points, JSONArray areas, JSONArray cameras,
                                        double bufferMeters) {
        return CoreResult.parse(nativeRouteAudit(
                points.toString(),
                areas == null ? "[]" : areas.toString(),
                cameras == null ? "[]" : cameras.toString(),
                bufferMeters));
    }

    /** Load (and cache) the offline road graph, returning node/edge counts. */
    public static CoreResult loadGraph(String graphPath) {
        return CoreResult.parse(nativeLoadGraph(graphPath));
    }

    /** Offline A* route; the first call loads {@code graphPath} into memory. */
    public static CoreResult route(String graphPath, double originLat, double originLon,
                                   double destinationLat, double destinationLon) {
        return CoreResult.parse(nativeRoute(
                graphPath, originLat, originLon, destinationLat, destinationLon));
    }

    /** Offline A* with camera penalties; returns {plan, avoidance}. */
    public static CoreResult routeAvoiding(String graphPath, double originLat, double originLon,
                                           double destinationLat, double destinationLon,
                                           JSONArray cameras, double radiusM,
                                           double maxDetourRatio, double maxDetourMinutes) {
        return CoreResult.parse(nativeRouteAvoiding(
                graphPath, originLat, originLon, destinationLat, destinationLon,
                cameras == null ? "[]" : cameras.toString(),
                radiusM, maxDetourRatio, maxDetourMinutes));
    }

    public static CoreResult planStraightLine(JSONObject request) {
        return CoreResult.parse(nativePlanStraightLine(request.toString()));
    }

    public static CoreResult encodePolyline(JSONArray points) {
        return CoreResult.parse(nativeEncodePolyline(points.toString()));
    }

    public static CoreResult decodePolyline(String encoded) {
        return CoreResult.parse(nativeDecodePolyline(encoded));
    }

    public static CoreResult parseAmapPositions(String positions) {
        return CoreResult.parse(nativeParseAmapPositions(positions));
    }

    public static CoreResult endpoints() {
        return CoreResult.parse(nativeEndpoints());
    }

    public static CoreResult constants() {
        return CoreResult.parse(nativeConstants());
    }
}
