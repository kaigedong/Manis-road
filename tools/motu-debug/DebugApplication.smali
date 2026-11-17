.class public Llab/motu/DebugApplication;
.super Lcom/Proxy/ShellApplication;

.method public constructor <init>()V
    .locals 0
    invoke-direct {p0}, Lcom/Proxy/ShellApplication;-><init>()V
    return-void
.end method

.method protected attachBaseContext(Landroid/content/Context;)V
    .locals 8
    invoke-super {p0, p1}, Lcom/Proxy/ShellApplication;->attachBaseContext(Landroid/content/Context;)V
    :try_start
    invoke-virtual {p1}, Landroid/content/Context;->getAssets()Landroid/content/res/AssetManager;
    move-result-object v0
    const-string v1, "motu-network.js"
    invoke-virtual {v0, v1}, Landroid/content/res/AssetManager;->open(Ljava/lang/String;)Ljava/io/InputStream;
    move-result-object v0
    invoke-virtual {p1}, Landroid/content/Context;->getFilesDir()Ljava/io/File;
    move-result-object v2
    new-instance v3, Ljava/io/File;
    invoke-direct {v3, v2, v1}, Ljava/io/File;-><init>(Ljava/io/File;Ljava/lang/String;)V
    new-instance v2, Ljava/io/FileOutputStream;
    invoke-direct {v2, v3}, Ljava/io/FileOutputStream;-><init>(Ljava/io/File;)V
    const/16 v3, 0x2000
    new-array v3, v3, [B
    :copy
    invoke-virtual {v0, v3}, Ljava/io/InputStream;->read([B)I
    move-result v4
    if-lez v4, :done
    const/4 v5, 0x0
    invoke-virtual {v2, v3, v5, v4}, Ljava/io/OutputStream;->write([BII)V
    goto :copy
    :done
    invoke-virtual {v2}, Ljava/io/OutputStream;->close()V
    invoke-virtual {v0}, Ljava/io/InputStream;->close()V
    const-string v0, "motulog"
    invoke-static {v0}, Ljava/lang/System;->loadLibrary(Ljava/lang/String;)V
    :try_end
    .catch Ljava/lang/Throwable; {:try_start .. :try_end} :failure
    return-void
    :failure
    move-exception v0
    const-string v1, "MotuHTTPS"
    const-string v2, "logger bootstrap failed"
    invoke-static {v1, v2, v0}, Landroid/util/Log;->e(Ljava/lang/String;Ljava/lang/String;Ljava/lang/Throwable;)I
    return-void
.end method
