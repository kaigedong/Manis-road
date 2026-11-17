package com.manisroad.app;

import android.app.Activity;
import android.content.Intent;
import android.os.Bundle;
import android.widget.Button;
import android.widget.TextView;
import android.widget.Toast;

import com.manisroad.app.core.CoreResult;
import com.manisroad.app.core.MotuCore;

import org.json.JSONArray;
import org.json.JSONObject;

/** Entry dashboard. All behaviour is delegated to the Rust core. */
public final class MainActivity extends Activity {
    private TextView status;
    private TextView account;
    private Button accountButton;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        setContentView(R.layout.activity_main);

        status = findViewById(R.id.text_status);
        account = findViewById(R.id.text_account);
        accountButton = findViewById(R.id.button_account);
        accountButton.setOnClickListener(view -> onAccountClicked());
        findViewById(R.id.button_map).setOnClickListener(view -> startActivity(
                new Intent(this, MapActivity.class)));
        findViewById(R.id.button_vehicle).setOnClickListener(view -> startActivity(
                new Intent(this, VehicleActivity.class)));
        findViewById(R.id.button_settings).setOnClickListener(view -> startActivity(
                new Intent(this, SettingsActivity.class)));
        findViewById(R.id.button_base_map).setOnClickListener(view -> startActivity(
                new Intent(this, BaseMapActivity.class)));

        CoreResult init = MotuCore.ensureInit(this);
        if (!init.ok) {
            status.setText(getString(R.string.core_init_failed, init.error));
            return;
        }
        renderStatus();
    }

    @Override
    protected void onResume() {
        super.onResume();
        renderStatus();
    }

    private void onAccountClicked() {
        CoreResult session = MotuCore.session();
        JSONObject data = session.objectData();
        boolean loggedIn = data != null && !data.optString("token").isEmpty();
        if (!loggedIn) {
            startActivity(new Intent(this, LoginActivity.class));
            return;
        }

        accountButton.setEnabled(false);
        account.setText(R.string.logout_in_progress);
        new Thread(() -> {
            CoreResult result = MotuCore.logout();
            runOnUiThread(() -> {
                accountButton.setEnabled(true);
                if (!result.ok) {
                    Toast.makeText(this, getString(R.string.logout_failed, result.error),
                            Toast.LENGTH_LONG).show();
                }
                renderStatus();
            });
        }, "motu-logout").start();
    }

    private void renderStatus() {
        CoreResult constants = MotuCore.constants();
        CoreResult endpoints = MotuCore.endpoints();
        CoreResult state = MotuCore.loadState();
        CoreResult session = MotuCore.session();

        StringBuilder builder = new StringBuilder();
        if (constants.objectData() != null) {
            JSONObject data = constants.objectData();
            builder.append("核心 ").append(data.optString("appVersion"))
                    .append(" · 平台 ").append(data.optString("platform"))
                    .append('\n');
            builder.append("扫描 ").append(data.optInt("scanIntervalM"))
                    .append(" m · 锥形 ").append(data.optDouble("forwardConeDeg"))
                    .append("°\n");
            builder.append("提醒半径 ").append(data.optInt("ordinaryRadiusM"))
                    .append(" m / 限行 ").append(data.optInt("restrictedRadiusM"))
                    .append(" m\n");
        }
        JSONArray endpointList = endpoints.arrayData();
        builder.append("已恢复接口 ").append(endpointList == null ? 0 : endpointList.length())
                .append(" 个\n");
        JSONObject stateData = state.objectData();
        if (stateData != null) {
            builder.append("车辆 ").append(stateData.optJSONArray("vehicles") == null
                    ? 0 : stateData.optJSONArray("vehicles").length()).append(" 辆");
        }
        status.setText(builder.toString());

        JSONObject sessionData = session.objectData();
        String token = sessionData == null ? "" : sessionData.optString("token");
        if (token.isEmpty()) {
            account.setText(R.string.account_guest);
            accountButton.setText(R.string.login_button);
            return;
        }
        String name = sessionData.optString("nickName");
        if (name.isEmpty()) {
            name = sessionData.optString("realName");
        }
        if (name.isEmpty()) {
            name = sessionData.optString("phone");
        }
        String level = sessionData.optBoolean("isPro")
                ? getString(R.string.account_vip)
                : getString(R.string.account_normal);
        account.setText(getString(R.string.account_logged_in, name + " · " + level));
        accountButton.setText(R.string.logout_button);
    }
}
