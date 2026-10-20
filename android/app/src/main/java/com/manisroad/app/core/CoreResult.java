package com.manisroad.app.core;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

/** Result envelope returned by every native method. */
public final class CoreResult {
    public final boolean ok;
    /** JSONObject, JSONArray, String, Number, Boolean, or null. */
    public final Object data;
    public final String error;

    private CoreResult(boolean ok, Object data, String error) {
        this.ok = ok;
        this.data = data;
        this.error = error;
    }

    public static CoreResult parse(String raw) {
        if (raw == null || raw.isEmpty()) {
            return new CoreResult(false, null, "native bridge returned no data");
        }
        try {
            JSONObject object = new JSONObject(raw);
            boolean ok = object.optBoolean("ok", false);
            Object data = object.opt("data");
            if (data == JSONObject.NULL) {
                data = null;
            }
            String error = object.optString("error", "");
            return new CoreResult(ok, data, error);
        } catch (JSONException exception) {
            return new CoreResult(false, null, "invalid native response: " + exception.getMessage());
        }
    }

    public JSONObject objectData() {
        return data instanceof JSONObject ? (JSONObject) data : null;
    }

    public JSONArray arrayData() {
        return data instanceof JSONArray ? (JSONArray) data : null;
    }

    public String stringData() {
        return data == null ? "" : String.valueOf(data);
    }
}
