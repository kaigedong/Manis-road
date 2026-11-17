package com.manisroad.app.core;

import android.util.Log;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStreamReader;
import java.io.OutputStream;
import java.net.InetAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.charset.StandardCharsets;

/**
 * Serve `tiles.pmtiles` to MapLibre over loopback.
 *
 * <p>MapLibre Android has no {@code pmtiles://} protocol — its vector sources take
 * an HTTP tile template (or an offline region pack). Rather than re-implement a
 * tile provider, the app runs a tiny server bound to {@code 127.0.0.1} that pulls
 * raw gzip tiles out of the archive through JNI. Nothing leaves the device: the
 * socket is loopback-only, and the data comes from the offline package.</p>
 */
public final class TileServer {

    private static final String TAG = "MotuTiles";

    private final ServerSocket socket;
    private final String pmtilesPath;
    private final Thread worker;
    private volatile boolean running = true;

    private TileServer(ServerSocket socket, String pmtilesPath) {
        this.socket = socket;
        this.pmtilesPath = pmtilesPath;
        this.worker = new Thread(this::serve, "motu-tiles");
        this.worker.setDaemon(true);
    }

    /** Start on an ephemeral loopback port. */
    public static TileServer start(String pmtilesPath) throws IOException {
        ServerSocket socket = new ServerSocket(0, 16, InetAddress.getByName("127.0.0.1"));
        TileServer server = new TileServer(socket, pmtilesPath);
        server.worker.start();
        Log.i(TAG, "tile server on 127.0.0.1:" + socket.getLocalPort());
        return server;
    }

    public int port() {
        return socket.getLocalPort();
    }

    /** URL template MapLibre expects. */
    public String tileUrlTemplate() {
        return "http://127.0.0.1:" + port() + "/tiles/{z}/{x}/{y}.mvt";
    }

    public void stop() {
        running = false;
        try {
            socket.close();
        } catch (IOException ignored) {
            // 关闭失败没有补救动作。
        }
    }

    private void serve() {
        while (running) {
            try (Socket client = socket.accept()) {
                handle(client);
            } catch (IOException error) {
                if (!running) {
                    return;
                }
                Log.w(TAG, "accept failed: " + error);
            }
        }
    }

    private void handle(Socket client) throws IOException {
        BufferedReader reader = new BufferedReader(
                new InputStreamReader(client.getInputStream(), StandardCharsets.US_ASCII));
        String request = reader.readLine();
        // Drain headers so the client does not see a reset.
        String line;
        while ((line = reader.readLine()) != null && !line.isEmpty()) {
            // 请求头内容用不到，只需要读干净。
        }
        OutputStream out = client.getOutputStream();
        int[] zxy = parseTilePath(request);
        if (zxy == null) {
            write(out, "HTTP/1.1 400 Bad Request", null, null);
            return;
        }
        byte[] tile;
        try {
            tile = MotuCore.tile(pmtilesPath, zxy[0], zxy[1], zxy[2]);
        } catch (RuntimeException error) {
            Log.w(TAG, "tile " + zxy[0] + "/" + zxy[1] + "/" + zxy[2] + " failed: " + error);
            write(out, "HTTP/1.1 500 Internal Server Error", null, null);
            return;
        }
        if (tile == null) {
            write(out, "HTTP/1.1 204 No Content", null, null);
            return;
        }
        write(out, "HTTP/1.1 200 OK", tile, "application/vnd.mapbox-vector-tile");
    }

    /** `GET /tiles/{z}/{x}/{y}.mvt` → `{z, x, y}`. */
    private static int[] parseTilePath(String request) {
        if (request == null || !request.startsWith("GET ")) {
            return null;
        }
        String path = request.substring(4).split(" ")[0];
        String[] parts = path.split("/");
        if (parts.length != 5 || !"tiles".equals(parts[1])) {
            return null;
        }
        String y = parts[4].endsWith(".mvt")
                ? parts[4].substring(0, parts[4].length() - 4)
                : parts[4];
        try {
            return new int[]{Integer.parseInt(parts[2]), Integer.parseInt(parts[3]),
                    Integer.parseInt(y)};
        } catch (NumberFormatException error) {
            return null;
        }
    }

    private static void write(OutputStream out, String status, byte[] body, String contentType)
            throws IOException {
        StringBuilder head = new StringBuilder(status).append("\r\n");
        if (contentType != null && body != null) {
            head.append("Content-Type: ").append(contentType).append("\r\n");
            // 瓦片在归档里就是 gzip 的，直接声明让客户端解压，不重复压缩。
            head.append("Content-Encoding: gzip\r\n");
            head.append("Content-Length: ").append(body.length).append("\r\n");
        }
        head.append("Access-Control-Allow-Origin: *\r\n");
        head.append("Connection: close\r\n\r\n");
        out.write(head.toString().getBytes(StandardCharsets.US_ASCII));
        if (body != null) {
            out.write(body);
        }
        out.flush();
    }
}
