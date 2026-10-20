package com.manisroad.app.ui;

import android.content.Context;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.Paint;
import android.graphics.Path;
import android.util.AttributeSet;
import android.view.MotionEvent;
import android.view.View;
import android.view.ViewConfiguration;

import java.util.ArrayList;
import java.util.List;

/**
 * Dependency-free schematic map. It deliberately is not a street map: the
 * proprietary AMap SDK is not bundled, so the view shows route geometry,
 * cameras and the simulated vehicle in a local equirectangular projection.
 */
public final class MapCanvasView extends View {
    public interface OnMarkerClickListener {
        void onMarkerClick(Marker marker);
    }

    public static final class Marker {
        public final String id;
        public final double latitude;
        public final double longitude;
        public final String label;
        public final boolean restricted;
        /** True when the marker has an official id that can be queried. */
        public final boolean official;

        public Marker(double latitude, double longitude, String label, boolean restricted) {
            this("", latitude, longitude, label, restricted, false);
        }

        public Marker(String id, double latitude, double longitude, String label,
                      boolean restricted, boolean official) {
            this.id = id;
            this.latitude = latitude;
            this.longitude = longitude;
            this.label = label;
            this.restricted = restricted;
            this.official = official;
        }
    }

    private final Paint background = new Paint();
    private final Paint grid = new Paint();
    private final Paint route = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Paint camera = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Paint restrictedCamera = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Paint vehicle = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Paint text = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Path routePath = new Path();
    private final Path heading = new Path();

    private final List<double[]> points = new ArrayList<>();
    private final List<Marker> markers = new ArrayList<>();
    private double vehicleLatitude = 31.2304;
    private double vehicleLongitude = 121.4737;
    private float vehicleBearing;
    private Projection lastProjection;
    private OnMarkerClickListener markerClickListener;
    private float touchDownX;
    private float touchDownY;

    public MapCanvasView(Context context) {
        this(context, null);
    }

    public MapCanvasView(Context context, AttributeSet attrs) {
        super(context, attrs);
        background.setColor(Color.rgb(14, 20, 27));
        grid.setColor(Color.rgb(32, 44, 57));
        grid.setStrokeWidth(1f);
        route.setColor(Color.rgb(88, 196, 221));
        route.setStrokeWidth(5f);
        route.setStyle(Paint.Style.STROKE);
        route.setStrokeCap(Paint.Cap.ROUND);
        camera.setColor(Color.rgb(88, 196, 221));
        restrictedCamera.setColor(Color.rgb(238, 108, 77));
        vehicle.setColor(Color.rgb(255, 214, 102));
        text.setColor(Color.WHITE);
        text.setTextSize(24f);
    }

    public void setOnMarkerClickListener(OnMarkerClickListener listener) {
        markerClickListener = listener;
    }

    public void setData(List<double[]> routePoints, List<Marker> cameraMarkers,
                        double latitude, double longitude, float bearing) {
        points.clear();
        if (routePoints != null) {
            points.addAll(routePoints);
        }
        markers.clear();
        if (cameraMarkers != null) {
            markers.addAll(cameraMarkers);
        }
        vehicleLatitude = latitude;
        vehicleLongitude = longitude;
        vehicleBearing = bearing;
        lastProjection = null;
        invalidate();
    }

    @Override
    protected void onDraw(Canvas canvas) {
        super.onDraw(canvas);
        canvas.drawRect(0, 0, getWidth(), getHeight(), background);

        Bounds bounds = computeBounds();
        if (bounds == null) {
            return;
        }
        Projection projection = new Projection(bounds, getWidth(), getHeight(), 72f);
        lastProjection = projection;

        for (int i = 0; i <= 10; i++) {
            float x = getWidth() * i / 10f;
            float y = getHeight() * i / 10f;
            canvas.drawLine(x, 0, x, getHeight(), grid);
            canvas.drawLine(0, y, getWidth(), y, grid);
        }

        routePath.reset();
        for (int i = 0; i < points.size(); i++) {
            float[] xy = projection.project(points.get(i)[0], points.get(i)[1]);
            if (i == 0) {
                routePath.moveTo(xy[0], xy[1]);
            } else {
                routePath.lineTo(xy[0], xy[1]);
            }
        }
        if (points.size() > 1) {
            canvas.drawPath(routePath, route);
        }

        for (Marker marker : markers) {
            float[] xy = projection.project(marker.latitude, marker.longitude);
            canvas.drawCircle(xy[0], xy[1], 12f,
                    marker.restricted ? restrictedCamera : camera);
            canvas.drawText(marker.label, xy[0] + 18f, xy[1] - 14f, text);
        }

        float[] vehicleXy = projection.project(vehicleLatitude, vehicleLongitude);
        canvas.drawCircle(vehicleXy[0], vehicleXy[1], 18f, vehicle);
        heading.reset();
        double radians = Math.toRadians(vehicleBearing);
        float tipX = vehicleXy[0] + (float) (Math.sin(radians) * 38f);
        float tipY = vehicleXy[1] - (float) (Math.cos(radians) * 38f);
        float leftX = vehicleXy[0] + (float) (Math.sin(radians + 2.5) * 24f);
        float leftY = vehicleXy[1] - (float) (Math.cos(radians + 2.5) * 24f);
        float rightX = vehicleXy[0] + (float) (Math.sin(radians - 2.5) * 24f);
        float rightY = vehicleXy[1] - (float) (Math.cos(radians - 2.5) * 24f);
        heading.moveTo(tipX, tipY);
        heading.lineTo(leftX, leftY);
        heading.lineTo(rightX, rightY);
        heading.close();
        canvas.drawPath(heading, vehicle);
    }

    @Override
    public boolean onTouchEvent(MotionEvent event) {
        switch (event.getActionMasked()) {
            case MotionEvent.ACTION_DOWN:
                touchDownX = event.getX();
                touchDownY = event.getY();
                return true;
            case MotionEvent.ACTION_UP:
                float dx = event.getX() - touchDownX;
                float dy = event.getY() - touchDownY;
                int slop = ViewConfiguration.get(getContext()).getScaledTouchSlop();
                if (dx * dx + dy * dy <= slop * slop) {
                    Marker hit = findMarker(event.getX(), event.getY());
                    if (hit != null) {
                        performClick();
                        if (markerClickListener != null) {
                            markerClickListener.onMarkerClick(hit);
                        }
                        return true;
                    }
                }
                break;
            default:
                break;
        }
        return super.onTouchEvent(event);
    }

    @Override
    public boolean performClick() {
        super.performClick();
        return true;
    }

    private Marker findMarker(float x, float y) {
        if (lastProjection == null) {
            return null;
        }
        float radius = 32f * getResources().getDisplayMetrics().density;
        float bestDistance = radius * radius;
        Marker best = null;
        for (Marker marker : markers) {
            float[] xy = lastProjection.project(marker.latitude, marker.longitude);
            float dx = xy[0] - x;
            float dy = xy[1] - y;
            float distance = dx * dx + dy * dy;
            if (distance <= bestDistance) {
                bestDistance = distance;
                best = marker;
            }
        }
        return best;
    }

    private Bounds computeBounds() {
        Bounds bounds = new Bounds();
        bounds.include(vehicleLatitude, vehicleLongitude);
        for (double[] point : points) {
            bounds.include(point[0], point[1]);
        }
        for (Marker marker : markers) {
            bounds.include(marker.latitude, marker.longitude);
        }
        return bounds.valid ? bounds : null;
    }

    private static final class Bounds {
        double minLat = Double.MAX_VALUE;
        double maxLat = -Double.MAX_VALUE;
        double minLon = Double.MAX_VALUE;
        double maxLon = -Double.MAX_VALUE;
        boolean valid;

        void include(double latitude, double longitude) {
            minLat = Math.min(minLat, latitude);
            maxLat = Math.max(maxLat, latitude);
            minLon = Math.min(minLon, longitude);
            maxLon = Math.max(maxLon, longitude);
            valid = true;
        }
    }

    private static final class Projection {
        private final double centerLat;
        private final double centerLon;
        private final double latitudeSpan;
        private final double longitudeSpan;
        private final float width;
        private final float height;
        private final float padding;

        Projection(Bounds bounds, float width, float height, float padding) {
            centerLat = (bounds.minLat + bounds.maxLat) / 2.0;
            centerLon = (bounds.minLon + bounds.maxLon) / 2.0;
            latitudeSpan = Math.max(0.0004, bounds.maxLat - bounds.minLat);
            longitudeSpan = Math.max(0.0004, bounds.maxLon - bounds.minLon);
            this.width = width;
            this.height = height;
            this.padding = padding;
        }

        float[] project(double latitude, double longitude) {
            double availableWidth = Math.max(1f, width - padding * 2f);
            double availableHeight = Math.max(1f, height - padding * 2f);
            double scaleX = availableWidth / longitudeSpan;
            double scaleY = availableHeight / latitudeSpan;
            double scale = Math.min(scaleX, scaleY);
            float x = (float) (width / 2.0 + (longitude - centerLon) * scale);
            float y = (float) (height / 2.0 - (latitude - centerLat) * scale);
            return new float[]{x, y};
        }
    }
}
