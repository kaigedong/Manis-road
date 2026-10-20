package com.manisroad.app.core;

import android.content.Context;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.pm.Signature;
import android.os.Build;

/**
 * Reads the DER bytes of the first APK signing certificate. Hashing and
 * {@code safe-env-token} construction stay in Rust; this class only accesses
 * the Android platform API.
 */
public final class SigningCertificate {
    private SigningCertificate() {
    }

    /**
     * Returns the APK signing certificate bytes, or an empty array when the
     * certificate cannot be read. The Rust core maps an empty array to an
     * omitted {@code safe-env-token}.
     */
    public static byte[] read(Context context) {
        try {
            PackageManager manager = context.getPackageManager();
            String packageName = context.getPackageName();
            Signature[] signatures;
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                PackageInfo info = manager.getPackageInfo(
                        packageName, PackageManager.GET_SIGNING_CERTIFICATES);
                signatures = info.signingInfo == null
                        ? new Signature[0]
                        : info.signingInfo.getApkContentsSigners();
            } else {
                signatures = legacySignatures(manager, packageName);
            }
            if (signatures.length == 0) {
                return new byte[0];
            }
            return signatures[0].toByteArray();
        } catch (PackageManager.NameNotFoundException exception) {
            return new byte[0];
        }
    }

    @SuppressWarnings("deprecation")
    private static Signature[] legacySignatures(PackageManager manager, String packageName)
            throws PackageManager.NameNotFoundException {
        PackageInfo info = manager.getPackageInfo(packageName, PackageManager.GET_SIGNATURES);
        return info.signatures == null ? new Signature[0] : info.signatures;
    }
}
