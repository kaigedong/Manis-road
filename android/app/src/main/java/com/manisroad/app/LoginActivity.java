package com.manisroad.app;

import android.app.Activity;
import android.os.Bundle;
import android.text.TextUtils;
import android.widget.Button;
import android.widget.EditText;
import android.widget.TextView;

import com.manisroad.app.core.CoreResult;
import com.manisroad.app.core.MotuCore;
import com.manisroad.app.core.SigningCertificate;

/**
 * Official password login. Network I/O and all request construction run in the
 * Rust core; this activity only collects input and renders the result.
 */
public final class LoginActivity extends Activity {
    private EditText phone;
    private EditText password;
    private Button submit;
    private TextView status;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        setContentView(R.layout.activity_login);

        phone = findViewById(R.id.edit_phone);
        password = findViewById(R.id.edit_password);
        submit = findViewById(R.id.button_login);
        status = findViewById(R.id.text_login_status);

        findViewById(R.id.button_guest).setOnClickListener(view -> finish());

        CoreResult init = MotuCore.ensureInit(this);
        if (!init.ok) {
            status.setText(getString(R.string.core_init_failed, init.error));
            submit.setEnabled(false);
            return;
        }

        CoreResult session = MotuCore.session();
        if (session.ok && session.objectData() != null
                && !TextUtils.isEmpty(session.objectData().optString("token"))) {
            status.setText(R.string.login_success);
            submit.setEnabled(false);
        }

        submit.setOnClickListener(view -> submit());
    }

    private void submit() {
        String phoneText = phone.getText().toString().trim();
        String passwordText = password.getText().toString();
        if (phoneText.isEmpty() || passwordText.isEmpty()) {
            status.setText(R.string.login_required);
            return;
        }

        byte[] signingCertificate = SigningCertificate.read(this);
        submit.setEnabled(false);
        status.setText(R.string.login_in_progress);
        password.setText("");

        new Thread(() -> {
            CoreResult result = MotuCore.login(phoneText, passwordText, signingCertificate);
            runOnUiThread(() -> {
                if (isFinishing()) {
                    return;
                }
                if (result.ok) {
                    status.setText(R.string.login_success);
                    setResult(RESULT_OK);
                    finish();
                } else {
                    status.setText(getString(R.string.login_failed, result.error));
                    submit.setEnabled(true);
                }
            });
        }, "motu-login").start();
    }
}
