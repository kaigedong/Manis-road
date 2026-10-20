package com.manisroad.app;

import android.app.Activity;
import android.os.Bundle;
import android.widget.ArrayAdapter;
import android.widget.EditText;
import android.widget.Spinner;
import android.widget.TextView;

import com.manisroad.app.core.CoreResult;
import com.manisroad.app.core.MotuCore;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

/** Motorcycle profile editor. Validation and persistence are Rust-side. */
public final class VehicleActivity extends Activity {
    private EditText plate;
    private EditText displacement;
    private Spinner color;
    private Spinner power;
    private Spinner transmission;
    private TextView message;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        setContentView(R.layout.activity_vehicle);

        plate = findViewById(R.id.edit_plate);
        displacement = findViewById(R.id.edit_displacement);
        color = findViewById(R.id.spinner_color);
        power = findViewById(R.id.spinner_power);
        transmission = findViewById(R.id.spinner_transmission);
        message = findViewById(R.id.text_vehicle_message);

        CoreResult init = MotuCore.ensureInit(this);
        if (!init.ok) {
            message.setText(getString(R.string.core_init_failed, init.error));
            return;
        }

        color.setAdapter(new ArrayAdapter<>(this, android.R.layout.simple_spinner_dropdown_item,
                new String[]{"蓝牌", "黄牌"}));
        power.setAdapter(new ArrayAdapter<>(this, android.R.layout.simple_spinner_dropdown_item,
                new String[]{"汽油", "电动", "混动"}));
        transmission.setAdapter(new ArrayAdapter<>(this, android.R.layout.simple_spinner_dropdown_item,
                new String[]{"自动", "手动", "半自动"}));

        findViewById(R.id.button_save_vehicle).setOnClickListener(view -> save());
        loadDefault();
    }

    private void loadDefault() {
        CoreResult state = MotuCore.loadState();
        JSONObject data = state.objectData();
        if (!state.ok || data == null) {
            message.setText(getString(R.string.native_error, state.error));
            return;
        }
        JSONArray vehicles = data.optJSONArray("vehicles");
        if (vehicles == null || vehicles.length() == 0) {
            message.setText(R.string.vehicle_empty);
            return;
        }
        JSONObject vehicle = null;
        for (int i = 0; i < vehicles.length(); i++) {
            JSONObject candidate = vehicles.optJSONObject(i);
            if (candidate != null && candidate.optBoolean("isDefault")) {
                vehicle = candidate;
                break;
            }
        }
        if (vehicle == null) {
            vehicle = vehicles.optJSONObject(0);
        }
        if (vehicle == null) {
            return;
        }
        plate.setText(vehicle.optString("numberPlate"));
        displacement.setText(vehicle.optString("displacement"));
        color.setSelection(Math.max(0, vehicle.optInt("plateColor") - 1));
        power.setSelection(Math.max(0, vehicle.optInt("powerType") - 1));
        transmission.setSelection(Math.max(0, vehicle.optInt("transType") - 1));
    }

    private void save() {
        try {
            JSONObject vehicle = new JSONObject();
            vehicle.put("numberPlate", plate.getText().toString());
            vehicle.put("plateColor", color.getSelectedItemPosition() + 1);
            vehicle.put("powerType", power.getSelectedItemPosition() + 1);
            vehicle.put("transType", transmission.getSelectedItemPosition() + 1);
            vehicle.put("displacement", displacement.getText().toString());
            vehicle.put("isDefault", true);

            CoreResult validation = MotuCore.validateVehicle(vehicle);
            JSONObject report = validation.objectData();
            if (!validation.ok || report == null) {
                message.setText(getString(R.string.native_error, validation.error));
                return;
            }
            if (!report.optBoolean("valid")) {
                message.setText(report.optJSONArray("errors") == null
                        ? getString(R.string.vehicle_invalid)
                        : join(report.optJSONArray("errors"), "；"));
                return;
            }

            CoreResult state = MotuCore.loadState();
            JSONObject data = state.objectData();
            if (!state.ok || data == null) {
                message.setText(getString(R.string.native_error, state.error));
                return;
            }
            JSONArray vehicles = data.optJSONArray("vehicles");
            JSONArray updated = new JSONArray();
            boolean replaced = false;
            if (vehicles != null) {
                for (int i = 0; i < vehicles.length(); i++) {
                    JSONObject existing = vehicles.optJSONObject(i);
                    if (existing == null) {
                        continue;
                    }
                    if (!replaced && existing.optBoolean("isDefault")) {
                        updated.put(vehicle);
                        replaced = true;
                        continue;
                    }
                    existing.put("isDefault", false);
                    updated.put(existing);
                }
            }
            if (!replaced) {
                updated.put(vehicle);
            }
            data.put("vehicles", updated);

            CoreResult saved = MotuCore.saveState(data);
            if (!saved.ok) {
                message.setText(getString(R.string.native_error, saved.error));
                return;
            }
            JSONArray warnings = report.optJSONArray("warnings");
            String suffix = warnings == null || warnings.length() == 0
                    ? ""
                    : "\n注意：" + join(warnings, "；");
            message.setText(getString(R.string.vehicle_saved) + suffix);
        } catch (JSONException exception) {
            message.setText(getString(R.string.native_error, exception.getMessage()));
        }
    }

    /** `JSONArray.join` is API 26+; keep the UI usable on older devices. */
    private static String join(JSONArray array, String separator) {
        StringBuilder builder = new StringBuilder();
        for (int i = 0; i < array.length(); i++) {
            if (i > 0) {
                builder.append(separator);
            }
            builder.append(array.optString(i));
        }
        return builder.toString();
    }
}
