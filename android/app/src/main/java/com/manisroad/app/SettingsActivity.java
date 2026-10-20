package com.manisroad.app;

import android.app.Activity;
import android.os.Bundle;
import android.widget.CheckBox;
import android.widget.EditText;
import android.widget.TextView;

import com.manisroad.app.core.CoreResult;
import com.manisroad.app.core.MotuCore;

import org.json.JSONException;
import org.json.JSONObject;

/**
 * Local preferences. The document shape lives in Rust (`Settings`), so this
 * activity only reads and writes the same camelCase fields.
 */
public final class SettingsActivity extends Activity {
    private CheckBox cameraAlerts;
    private CheckBox avoidAreas;
    private EditText cityCode;
    private EditText apiBaseUrl;
    private EditText radiusOverride;
    private TextView message;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        setContentView(R.layout.activity_settings);

        cameraAlerts = findViewById(R.id.check_camera_alerts);
        avoidAreas = findViewById(R.id.check_avoid_areas);
        cityCode = findViewById(R.id.edit_city_code);
        apiBaseUrl = findViewById(R.id.edit_api_base_url);
        radiusOverride = findViewById(R.id.edit_radius_override);
        message = findViewById(R.id.text_settings_message);

        CoreResult init = MotuCore.ensureInit(this);
        if (!init.ok) {
            message.setText(getString(R.string.core_init_failed, init.error));
            return;
        }

        findViewById(R.id.button_save_settings).setOnClickListener(view -> save());
        load();
    }

    private void load() {
        CoreResult result = MotuCore.loadState();
        JSONObject data = result.objectData();
        if (!result.ok || data == null) {
            message.setText(getString(R.string.native_error, result.error));
            return;
        }
        JSONObject settings = data.optJSONObject("settings");
        if (settings == null) {
            settings = new JSONObject();
        }
        cameraAlerts.setChecked(settings.optBoolean("cameraAlertsEnabled", true));
        avoidAreas.setChecked(settings.optBoolean("avoidAreasEnabled", true));
        cityCode.setText(settings.optString("cityCode", "021"));
        apiBaseUrl.setText(settings.optString("apiBaseUrl"));
        if (settings.has("cameraRadiusOverrideM")
                && !settings.isNull("cameraRadiusOverrideM")) {
            double radius = settings.optDouble("cameraRadiusOverrideM");
            radiusOverride.setText(Double.isFinite(radius)
                    ? String.valueOf(radius) : "");
        } else {
            radiusOverride.setText("");
        }
    }

    private void save() {
        CoreResult loaded = MotuCore.loadState();
        JSONObject data = loaded.objectData();
        if (!loaded.ok || data == null) {
            message.setText(getString(R.string.native_error, loaded.error));
            return;
        }

        JSONObject settings = new JSONObject();
        try {
            settings.put("cameraAlertsEnabled", cameraAlerts.isChecked());
            settings.put("avoidAreasEnabled", avoidAreas.isChecked());
            settings.put("cityCode", cityCode.getText().toString().trim());
            settings.put("apiBaseUrl", apiBaseUrl.getText().toString().trim());

            String radiusText = radiusOverride.getText().toString().trim();
            if (!radiusText.isEmpty()) {
                double radius;
                try {
                    radius = Double.parseDouble(radiusText);
                } catch (NumberFormatException exception) {
                    message.setText(R.string.settings_radius_invalid);
                    return;
                }
                if (!Double.isFinite(radius) || radius < 1.0 || radius > 5000.0) {
                    message.setText(R.string.settings_radius_invalid);
                    return;
                }
                settings.put("cameraRadiusOverrideM", radius);
            }

            data.put("settings", settings);
        } catch (JSONException exception) {
            message.setText(getString(R.string.native_error, exception.getMessage()));
            return;
        }

        CoreResult saved = MotuCore.saveState(data);
        if (!saved.ok) {
            message.setText(getString(R.string.native_error, saved.error));
            return;
        }
        message.setText(R.string.settings_saved);
    }
}
