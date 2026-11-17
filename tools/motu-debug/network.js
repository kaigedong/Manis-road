import Java from 'frida-java-bridge';

// Local logcat only. Each record is chunked so Android's log size limit cannot
// silently discard a large JSON response. The collector reassembles by id.
let serial = 0;
const MAX_BODY = 1024 * 1024;
const secret = /authorization|cookie|password|passwd|token|secret|api[-_]?key|session|credential|phone|mobile|idcard/i;
let Log;
function redact(value) {
  if (Array.isArray(value)) return value.map(redact);
  if (value && typeof value === 'object') {
    const out = {};
    for (const [key, val] of Object.entries(value)) out[key] = secret.test(key) ? '[REDACTED]' : redact(val);
    return out;
  }
  return value;
}
function safeUrl(value) {
  return String(value).replace(/([?&])([^=&]+)=([^&]*)/g, (all, sep, key) => {
    try { return secret.test(decodeURIComponent(key)) ? `${sep}${key}=[REDACTED]` : all; }
    catch (_) { return `${sep}${key}=[UNDECODABLE]`; }
  }).replace(/\/\/[^/@]+:[^/@]+@/, '//[REDACTED]@');
}
function headers(h) {
  const result = [];
  for (let i = 0; i < h.size(); i++) {
    const name = String(h.name(i));
    result.push([name, secret.test(name) ? '[REDACTED]' : String(h.value(i))]);
  }
  return result;
}
function bodyText(text, type) {
  try { return {format: 'json', value: redact(JSON.parse(text))}; } catch (_) {}
  if (/x-www-form-urlencoded/i.test(type || '')) {
    return {format: 'form', value: safeUrl('https://local/?' + text).split('?')[1]};
  }
  // Unknown text/binary could contain credentials. Report sizes, omit payload.
  return {format: 'omitted', reason: 'non-JSON/non-form body', characters: text.length};
}
function emit(record) {
  try {
    record.time = new Date().toISOString();
    const id = `${Process.id}-${++serial}`;
    const text = JSON.stringify(record);
    // Escape to ASCII before splitting: 1800 chars fits even on small log buffers.
    const ascii = text.replace(/[\u007f-\uffff]/g, c => '\\u' + c.charCodeAt(0).toString(16).padStart(4, '0'));
    const count = Math.ceil(ascii.length / 1800);
    for (let i = 0; i < count; i++) Log.i('MotuHTTPS', `${id} ${i + 1}/${count} ${ascii.slice(i * 1800, (i + 1) * 1800)}`);
  } catch (_) {}
}

Java.perform(() => {
  Log = Java.use('android.util.Log');
  emit({event: 'logger_ready', package: 'com.motu.debug', max_body_bytes: MAX_BODY, sensitive_fields: 'redacted'});
  const installed = new Set();
  function installOkHttp(factory) {
    if (installed.has('okhttp')) return;
    try {
      const Chain = factory.use('okhttp3.internal.http.RealInterceptorChain');
      const proceed = Chain.proceed.overload('okhttp3.Request');
      const Buffer = factory.use('okio.Buffer');
      proceed.implementation = function(request) {
        // Only log the outermost invocation per thread: retries still show the
        // final response; network details may also appear in TLS metadata.
        const tid = Process.getCurrentThreadId();
        if (active.has(tid)) return proceed.call(this, request);
        active.add(tid);
        const exchange = `${Process.id}-http-${++serial}`;
        try {
          const req = {event: 'request', exchange, method: String(request.method()), url: safeUrl(request.url()), headers: headers(request.headers())};
          try {
            const body = request.body();
            if (body) {
              const size = Number(body.contentLength());
              let repeatable = false;
              // OkHttp 4+ explicitly describes streaming/one-shot bodies.
              if (body.isOneShot && body.isDuplex) repeatable = !body.isOneShot() && !body.isDuplex();
              const type = String(body.contentType() || '');
              if (repeatable && size >= 0 && size <= MAX_BODY && /json|x-www-form-urlencoded/i.test(type)) {
                const buffer = Buffer.$new();
                body.writeTo(buffer);
                req.body = bodyText(String(buffer.readUtf8()), type);
              } else req.body = {format: 'omitted', reason: 'streaming, unknown length/type, or limit', bytes: size};
            }
          } catch (e) { req.body_error = String(e); }
          emit(req);
          const response = proceed.call(this, request);
          const record = {event: 'response', exchange, url: safeUrl(response.request().url()), status: Number(response.code()), headers: headers(response.headers())};
          try {
            const body = response.body();
            if (body) {
              const type = String(body.contentType() || '');
              const size = Number(body.contentLength());
              // Never pre-read unknown/chunked or server-sent streams.
              if (/json|x-www-form-urlencoded/i.test(type) && size >= 0 && size <= MAX_BODY) {
                record.body = bodyText(String(response.peekBody(MAX_BODY).string()), type);
              } else record.body = {format: 'omitted', reason: 'streaming, unknown length/type, or limit', bytes: size};
            }
          } catch (e) { record.body_error = String(e); }
          emit(record);
          return response;
        } catch (e) {
          emit({event: 'request_failed', exchange, error: String(e)});
          throw e;
        } finally { active.delete(tid); }
      };
      installed.add('okhttp');
      emit({event: 'hook_installed', hook: 'okhttp3.RealInterceptorChain.proceed'});
    } catch (_) {}
  }
  const active = new Set();
  installOkHttp(Java.classFactory);
  Java.enumerateClassLoaders({
    onMatch(loader) { try { installOkHttp(Java.ClassFactory.get(loader)); } catch (_) {} },
    onComplete() {},
  });
  let attempts = 0;
  const retry = setInterval(() => {
    Java.perform(() => installOkHttp(Java.classFactory));
    if (installed.has('okhttp') || ++attempts >= 60) {
      clearInterval(retry);
      if (!installed.has('okhttp')) emit({event: 'coverage_gap', detail: 'OkHttp class not found after 60 seconds'});
    }
  }, 1000);
  try {
    const URL = Java.use('java.net.URL');
    const open = URL.openConnection.overload();
    open.implementation = function() {
      emit({event: 'url_open', url: safeUrl(this.toString()), coverage: 'URL only'});
      return open.call(this);
    };
    emit({event: 'hook_installed', hook: 'java.net.URL.openConnection'});
  } catch (e) { emit({event: 'hook_error', hook: 'URL', error: String(e)}); }
});
