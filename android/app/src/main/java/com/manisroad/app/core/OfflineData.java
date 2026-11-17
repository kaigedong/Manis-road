package com.manisroad.app.core;

import android.content.Context;
import android.util.Log;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;

/**
 * Location of the offline road graph. The graph is large (tens of MB), so it
 * is not bundled in the repository; push it with
 * {@code adb push graph.bin .../files/offline/graph.bin}. If an APK happens to
 * ship {@code assets/graph.bin}, it is copied on first use.
 */
public final class OfflineData {
    private OfflineData() {
    }

    private static File pick(Context context, String name) {
        File external = context.getExternalFilesDir(null);
        File externalFile = external == null ? null : new File(external, "offline/" + name);
        if (externalFile != null && externalFile.isFile()) {
            return externalFile;
        }
        File internal = new File(context.getFilesDir(), "offline/" + name);
        if (internal.isFile()) {
            return internal;
        }
        return externalFile != null ? externalFile : internal;
    }

    /** `.mrdp` 里解出来的离线瓦片归档。 */
    public static File tilesFile(Context context) {
        return pick(context, "tiles.pmtiles");
    }

    public static File camerasFile(Context context) {
        return pick(context, "cameras.json");
    }

    /**
     * Details are always internal: the app also appends newly opened details,
     * and the internal path is reliably writable on every device.
     */
    public static File detailsFile(Context context) {
        return new File(context.getFilesDir(), "offline/details.json");
    }

    public static boolean writeText(File file, String text) {
        try {
            File parent = file.getParentFile();
            if (parent != null && !parent.exists() && !parent.mkdirs()) {
                return false;
            }
            try (java.io.FileOutputStream output = new java.io.FileOutputStream(file)) {
                output.write(text.getBytes(java.nio.charset.StandardCharsets.UTF_8));
            }
            return true;
        } catch (IOException error) {
            Log.w("MotuOffline", "write " + file + " failed: " + error);
            return false;
        }
    }

    /** Accept both a bare array and the web export envelope. */
    public static org.json.JSONArray parseCameras(String json) {
        if (json == null) {
            return null;
        }
        try {
            String trimmed = json.trim();
            if (trimmed.startsWith("[")) {
                return new org.json.JSONArray(trimmed);
            }
            return new org.json.JSONObject(trimmed).optJSONArray("cameras");
        } catch (org.json.JSONException error) {
            Log.w("MotuOffline", "cannot parse cameras: " + error.getMessage());
            return null;
        }
    }

    public static String readText(File file) {
        if (file == null || !file.isFile()) {
            return null;
        }
        try (InputStream input = new java.io.FileInputStream(file)) {
            java.io.ByteArrayOutputStream output = new java.io.ByteArrayOutputStream();
            byte[] buffer = new byte[64 * 1024];
            int read;
            while ((read = input.read(buffer)) != -1) {
                output.write(buffer, 0, read);
            }
            return output.toString("UTF-8");
        } catch (IOException error) {
            Log.w("MotuOffline", "read " + file + " failed: " + error);
            return null;
        }
    }

    public static File graphFile(Context context) {
        // Prefer the app-specific external directory: it is reachable with
        // `adb push /sdcard/Android/data/<pkg>/files/offline/graph.bin`.
        // Some devices/emulators do not expose shell-written files to the app
        // through FUSE, so an internal copy is also accepted:
        // `adb shell run-as <pkg> ... files/offline/graph.bin`.
        File external = context.getExternalFilesDir(null);
        File internal = new File(context.getFilesDir(), "offline/graph.bin");
        if (external != null && new File(external, "offline/graph.bin").isFile()) {
            return new File(external, "offline/graph.bin");
        }
        if (internal.isFile()) {
            return internal;
        }
        File base = external == null ? context.getFilesDir() : external;
        return new File(base, "offline/graph.bin");
    }

    public static File ensureGraph(Context context) {
        File target = graphFile(context);
        if (target.isFile()) {
            return target;
        }
        File parent = target.getParentFile();
        if (parent != null && !parent.exists() && !parent.mkdirs()) {
            return target;
        }
        try (InputStream input = context.getAssets().open("graph.bin");
             OutputStream output = new FileOutputStream(target)) {
            byte[] buffer = new byte[64 * 1024];
            int read;
            while ((read = input.read(buffer)) != -1) {
                output.write(buffer, 0, read);
            }
        } catch (IOException ignored) {
            // No bundled asset; the UI tells the user how to push the file.
        }
        return target;
    }
}
