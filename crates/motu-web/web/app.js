(() => {
  "use strict";

  const TYPE_COLORS = {
    1: "#d97706",
    2: "#dc2626",
    3: "#7c3aed",
    4: "#dc2626",
    5: "#ea580c",
    6: "#2563eb",
    7: "#0891b2",
    8: "#b91c1c",
    9: "#ca8a04",
    10: "#9333ea",
    11: "#475569",
    12: "#be123c",
    13: "#e11d48",
    14: "#f59e0b",
    15: "#64748b",
  };

  const TYPE_NAMES = {
    0: "未知类型",
    1: "闯限行",
    2: "走应急车道",
    3: "走非机动车道",
    4: "闯红灯",
    5: "超速",
    6: "公交车道",
    7: "未按车道行驶",
    8: "逆行",
    9: "尾号限行",
    10: "不带头盔",
    11: "噪音",
    12: "现场罚单",
    13: "压线",
    14: "违规用灯",
  };

  const state = {
    config: null,
    app: null,
    snapshot: null,
    map: null,
    markerLayer: null,
    markers: new Map(),
    camerasById: new Map(),
    selectedId: null,
    detailRequest: 0,
    viewportRequest: 0,
    areas: null,
    areasVisible: false,
    cruise: false,
    syncing: false,
    pickMode: null,
    routeOrigin: null,
    routeDestination: null,
    routePlan: null,
    offline: false,
    basemapLayer: null,
    positionLayer: null,
    watchId: null,
    position: null,
    nearestCamera: null,
    lastNearbyAt: 0,
    alertedAt: 0,
    alertedId: null,
    offRouteSince: null,
    lastRerouteAt: 0,
    cruiseTimer: null,
  };

  const dom = {
    sourceBadge: document.getElementById("source-badge"),
    cameraCount: document.getElementById("camera-count"),
    cityCode: document.getElementById("city-code"),
    syncButton: document.getElementById("sync-button"),
    accountStatus: document.getElementById("account-status"),
    loginButton: document.getElementById("login-button"),
    logoutButton: document.getElementById("logout-button"),
    signatureHint: document.getElementById("signature-hint"),
    detailEyebrow: document.getElementById("detail-eyebrow"),
    detailTitle: document.getElementById("detail-title"),
    detailBody: document.getElementById("detail-body"),
    detailClose: document.getElementById("detail-close"),
    loginDialog: document.getElementById("login-dialog"),
    loginForm: document.getElementById("login-form"),
    loginPhone: document.getElementById("login-phone"),
    loginPassword: document.getElementById("login-password"),
    loginError: document.getElementById("login-error"),
    loginSubmit: document.getElementById("login-submit"),
    loginClose: document.getElementById("login-close"),
    loginCancel: document.getElementById("login-cancel"),
    toast: document.getElementById("toast"),
    mapElement: document.getElementById("map"),
    cacheStatus: document.getElementById("cache-status"),
    cacheClear: document.getElementById("cache-clear"),
    areasButton: document.getElementById("areas-button"),
    cruiseButton: document.getElementById("cruise-button"),
    offlineBadge: document.getElementById("offline-badge"),
    routeOrigin: document.getElementById("route-origin"),
    routeDestination: document.getElementById("route-destination"),
    routeGo: document.getElementById("route-go"),
    routeClear: document.getElementById("route-clear"),
    routeHint: document.getElementById("route-hint"),
    routeAvoid: document.getElementById("route-avoid"),
    routeRadius: document.getElementById("route-radius"),
    gpsButton: document.getElementById("gps-button"),
    cruiseAlert: document.getElementById("cruise-alert"),
    cruiseStep: document.getElementById("cruise-step"),
    cruiseCamera: document.getElementById("cruise-camera"),
  };

  let toastTimer = null;

  function toast(message, kind = "info") {
    dom.toast.textContent = message;
    dom.toast.dataset.kind = kind;
    dom.toast.hidden = false;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => {
      dom.toast.hidden = true;
    }, kind === "error" ? 7200 : 4200);
  }

  async function api(path, options = {}) {
    const headers = { ...(options.headers || {}) };
    if (options.body !== undefined && !headers["Content-Type"]) {
      headers["Content-Type"] = "application/json";
    }
    const response = await fetch(path, { ...options, headers });
    const text = await response.text();
    let data = {};
    if (text) {
      try {
        data = JSON.parse(text);
      } catch {
        data = { error: text };
      }
    }
    if (!response.ok) {
      throw new Error(data.error || `HTTP ${response.status}`);
    }
    return data;
  }

  function el(tag, className, text) {
    const node = document.createElement(tag);
    if (className) {
      node.className = className;
    }
    if (text !== undefined && text !== null) {
      node.textContent = String(text);
    }
    return node;
  }

  function typeColor(id) {
    return TYPE_COLORS[id] || "#64748b";
  }

  function typeName(id) {
    return TYPE_NAMES[id] || TYPE_NAMES[0];
  }

  function formatClock(ms) {
    if (!ms) {
      return "";
    }
    const date = new Date(ms);
    if (Number.isNaN(date.getTime())) {
      return "";
    }
    return date.toLocaleTimeString("zh-CN", {
      hour12: false,
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  function formatDateTime(value) {
    if (!value) {
      return "";
    }
    const milliseconds = value < 1e12 ? value * 1000 : value;
    const date = new Date(milliseconds);
    if (Number.isNaN(date.getTime())) {
      return "";
    }
    return date.toLocaleString("zh-CN", {
      hour12: false,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
    });
  }

  function formatAge(ms) {
    if (!ms) {
      return "未知时间";
    }
    const seconds = Math.max(0, Math.round((Date.now() - ms) / 1000));
    if (seconds < 60) {
      return "刚刚";
    }
    const minutes = Math.round(seconds / 60);
    if (minutes < 60) {
      return `${minutes} 分钟前`;
    }
    const hours = Math.round(minutes / 60);
    if (hours < 24) {
      return `${hours} 小时前`;
    }
    return `${Math.round(hours / 24)} 天前`;
  }

  function maskContact(value) {
    const text = String(value || "").trim();
    if (!text) {
      return "";
    }
    if (/^1\d{10}$/.test(text)) {
      return `${text.slice(0, 3)}****${text.slice(-4)}`;
    }
    return text.replace(/(\d{3})\d{4}(\d{4})/g, "$1****$2");
  }

  function imageUrl(url) {
    return `/api/image?url=${encodeURIComponent(url)}`;
  }

  function initMap(config) {
    if (typeof L === "undefined") {
      throw new Error("Leaflet 资源未加载，请检查 web/vendor/leaflet");
    }
    const center = config.mapCenter || { latitude: 31.2304, longitude: 121.4737 };
    state.map = L.map(dom.mapElement, {
      zoomControl: true,
      attributionControl: true,
      preferCanvas: true,
    }).setView([center.latitude, center.longitude], 12);

    state.map.createPane("areasPane");
    state.map.getPane("areasPane").style.zIndex = "350";
    state.basemapLayer = L.layerGroup();
    state.positionLayer = L.layerGroup().addTo(state.map);

    if (state.offline) {
      // Offline: no tile requests at all; roads come from the local OSM graph.
      state.map.attributionControl.addAttribution(
        '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors (ODbL)',
      );
      state.basemapLayer.addTo(state.map);
    } else {
      const tiles = L.tileLayer(config.tileUrl, {
        maxZoom: config.maxZoom || 19,
        attribution: config.tileAttribution || "",
      }).addTo(state.map);

      let tileWarningShown = false;
      tiles.on("tileerror", () => {
        if (tileWarningShown) {
          return;
        }
        tileWarningShown = true;
        toast(
          "地图瓦片加载失败：请检查网络，或用 MOTU_WEB_TILE_URL 指向可用的 WGS-84 瓦片服务。",
          "error",
        );
      });
    }

    state.areasLayer = L.layerGroup();
    state.routeLayer = L.layerGroup().addTo(state.map);
    state.markerLayer = L.layerGroup().addTo(state.map);
    L.control.scale({ imperial: false, maxWidth: 120 }).addTo(state.map);
    state.map.on("click", handleMapClick);
    state.map.on("moveend zoomend", () => {
      scheduleViewportRefresh();
      scheduleBasemapRefresh();
    });
    window.addEventListener("resize", () => {
      state.map.invalidateSize();
      scheduleViewportRefresh();
    });
  }

  function applyAppState(app) {
    state.app = app;
    if (typeof app.cityCode === "string" && app.cityCode.trim()) {
      dom.cityCode.value = app.cityCode.trim();
    }
    dom.signatureHint.hidden = Boolean(app.signatureConfigured);

    const loggedIn = Boolean(app.loggedIn);
    dom.loginButton.hidden = loggedIn;
    dom.logoutButton.hidden = !loggedIn;
    if (loggedIn) {
      const user = app.user || {};
      const name = user.nickName || user.phone || "已登录";
      dom.accountStatus.textContent = user.isPro ? `${name} · VIP` : name;
      dom.accountStatus.title = user.phone || "";
    } else {
      dom.accountStatus.textContent = "未登录";
      dom.accountStatus.title = "";
    }
  }

  function markerStyle(camera, selected = false) {
    return {
      radius: selected ? 10 : 7,
      color: selected ? "#0f172a" : "#ffffff",
      weight: selected ? 4 : 2,
      fillColor: typeColor(camera.primaryType),
      fillOpacity: 0.92,
      opacity: 1,
      bubblingMouseEvents: false,
    };
  }

  function renderSnapshot(snapshot, options = {}) {
    const keepDetail = Boolean(options.keepDetail);
    state.snapshot = snapshot;
    state.markerLayer.clearLayers();
    state.markers.clear();
    state.camerasById.clear();
    if (!keepDetail) {
      state.selectedId = null;
      state.detailRequest += 1;
    }

    for (const camera of snapshot.cameras || []) {
      if (
        !camera.position ||
        !Number.isFinite(camera.position.latitude) ||
        !Number.isFinite(camera.position.longitude)
      ) {
        continue;
      }
      const marker = L.circleMarker(
        [camera.position.latitude, camera.position.longitude],
        markerStyle(camera),
      );
      const tooltip =
        (camera.primaryLabel || typeName(camera.primaryType)) +
        (camera.official ? "" : " · 演示");
      marker.bindTooltip(tooltip, {
        direction: "top",
        offset: [0, -8],
        className: "camera-tooltip",
        opacity: 1,
      });
      marker.on("click", () => {
        if (state.pickMode) {
          applyPickedPoint({
            lat: camera.position.latitude,
            lng: camera.position.longitude,
          });
          return;
        }
        selectCamera(camera);
      });
      marker.addTo(state.markerLayer);
      state.markers.set(camera.id, marker);
      state.camerasById.set(camera.id, camera);
    }

    const official = snapshot.source !== "demo";
    const count = (snapshot.cameras || []).length;
    const total = Number(state.app?.cache?.cameras || 0);
    dom.sourceBadge.textContent = official ? "缓存点位" : "演示数据";
    dom.sourceBadge.className = `badge ${official ? "badge-official" : "badge-demo"}`;
    const age = official && snapshot.fetchedAtMs ? ` · ${formatAge(snapshot.fetchedAtMs)}` : "";
    const totalText = official && total > count ? ` · 缓存共 ${total}` : "";
    dom.cameraCount.textContent = `${count} 个点位${age}${totalText}`;

    if (keepDetail && state.selectedId) {
      const camera = state.camerasById.get(state.selectedId);
      if (!camera) {
        clearDetail();
      } else {
        state.markers.get(state.selectedId).setStyle(markerStyle(camera, true));
      }
    } else if (!keepDetail) {
      clearDetail();
    }
  }

  function setSelectedMarker(selectedId) {
    if (state.selectedId && state.markers.has(state.selectedId)) {
      const previous = state.camerasById.get(state.selectedId);
      if (previous) {
        state.markers.get(state.selectedId).setStyle(markerStyle(previous, false));
      }
    }
    state.selectedId = selectedId;
    if (selectedId && state.markers.has(selectedId)) {
      const current = state.camerasById.get(selectedId);
      if (current) {
        state.markers.get(selectedId).setStyle(markerStyle(current, true));
      }
    }
  }

  function setDetailHeader(eyebrow, title) {
    dom.detailEyebrow.textContent = eyebrow;
    dom.detailTitle.textContent = title;
  }

  function emptyDetailView() {
    const wrapper = el("div", "empty-state");
    wrapper.append(
      el("div", "empty-icon", "◎"),
      el("p", null, "地图上的圆点就是电子眼。"),
      el(
        "p",
        "muted",
        "演示点位可直接点击查看说明；官方点位需要先登录并同步。",
      ),
    );
    return wrapper;
  }

  function clearDetail() {
    state.detailRequest += 1;
    setSelectedMarker(null);
    setDetailHeader("电子眼详情", "选择一个电子眼");
    dom.detailBody.replaceChildren(emptyDetailView());
  }

  function renderLoading(camera) {
    setDetailHeader("官方电子眼", camera.primaryLabel || "电子眼详情");
    const wrapper = el("div", "loading");
    wrapper.append(el("span", "spinner"), el("span", null, "正在请求详情和罚单记录…"));
    dom.detailBody.replaceChildren(wrapper);
  }

  function renderPanelError(message) {
    setDetailHeader("详情加载失败", "暂时无法显示");
    const wrapper = el("div", "panel-error", message);
    dom.detailBody.replaceChildren(wrapper);
  }

  function renderLoginRequired(camera) {
    setDetailHeader("需要登录", camera.primaryLabel || "官方电子眼");
    const wrapper = el("div", "panel-error", "同步官方点位和查看详情需要先登录 Motu 账号。");
    const button = el("button", "button button-primary", "打开登录");
    button.type = "button";
    button.style.marginTop = "10px";
    button.addEventListener("click", openLogin);
    const container = el("div");
    container.append(wrapper, button);
    dom.detailBody.replaceChildren(container);
  }

  function addChip(container, text, variant = "") {
    container.append(el("span", `chip ${variant}`.trim(), text));
  }

  function addSection(parent, title, countText) {
    const section = el("section", "detail-section");
    const heading = el("h3");
    heading.append(document.createTextNode(title));
    if (countText) {
      heading.append(el("span", "section-count", countText));
    }
    section.append(heading);
    parent.append(section);
    return section;
  }

  function renderDemoDetail(camera) {
    setDetailHeader("演示点位 · 非官方数据", camera.primaryLabel || "电子眼");
    const body = document.createDocumentFragment();

    const hero = el("div", "detail-hero");
    const meta = el("div", "hero-meta");
    for (const label of camera.tabLabels || []) {
      addChip(meta, label);
    }
    addChip(meta, "演示数据", "chip-warn");
    hero.append(meta);
    hero.append(
      el(
        "div",
        "address",
        "上海演示点位（坐标为虚构，仅用于调试界面）",
      ),
    );
    body.append(hero);

    const note = addSection(body, "说明");
    note.append(
      el(
        "div",
        "detail-callout",
        "该点位没有官方数据。登录后拖动地图到目标区域并点击「同步官方点位」，即可查看真实电子眼详情、规则和罚单记录。",
      ),
    );

    const coordinates = addSection(body, "调试坐标");
    const stats = el("div", "stat-row");
    const position = camera.position || {};
    const source = camera.sourcePosition || {};
    addChip(stats, `WGS ${Number(position.latitude).toFixed(5)}, ${Number(position.longitude).toFixed(5)}`, "chip-neutral");
    addChip(stats, `GCJ ${Number(source.latitude).toFixed(5)}, ${Number(source.longitude).toFixed(5)}`, "chip-neutral");
    coordinates.append(stats);

    dom.detailBody.replaceChildren(body);
  }

  function renderDetail(camera, detail) {
    setDetailHeader("官方电子眼", detail.title || camera.primaryLabel || "电子眼详情");
    const body = document.createDocumentFragment();
    const hero = el("div", "detail-hero");
    const meta = el("div", "hero-meta");

    const types = Array.isArray(detail.types) ? detail.types : [];
    if (types.length > 0) {
      for (const type of types) {
        addChip(meta, type.name || typeName(type.tabType));
      }
    } else {
      for (const label of camera.tabLabels || []) {
        addChip(meta, label);
      }
    }
    if (detail.white) {
      addChip(meta, "白名单", "chip-neutral");
    }
    if (detail.avoid) {
      addChip(meta, "已加入避让", "chip-warn");
    }
    if (camera.newFlag) {
      addChip(meta, "新点位", "chip-danger");
    }
    addChip(
      meta,
      detail.cached ? `本地缓存 · ${formatAge(detail.fetchedAtMs)}` : "刚刚获取",
      detail.cached ? "chip-neutral" : "chip-ok",
    );
    if (detail.warning) {
      addChip(meta, "官方接口暂时不可用", "chip-warn");
    }
    hero.append(meta);
    if (detail.address) {
      hero.append(el("div", "address", detail.address));
    }
    body.append(hero);

    if (detail.detail) {
      const section = addSection(body, "电子眼说明");
      section.append(el("div", "detail-callout", detail.detail));
    }

    if (detail.contactPerson) {
      const section = addSection(body, "上报者");
      section.append(
        el(
          "div",
          "detail-callout",
          `${detail.contactPerson}${detail.contactWay ? ` · ${maskContact(detail.contactWay)}` : ""}`,
        ),
      );
    }

    if (types.length > 0) {
      const section = addSection(body, "抓拍类型", `${types.length} 项`);
      const list = el("div", "type-list");
      for (const type of types) {
        const card = el("div", "type-card");
        card.append(
          el("strong", null, type.name || typeName(type.tabType)),
          el("span", null, type.remark || "官方电子眼抓拍类型"),
        );
        list.append(card);
      }
      section.append(list);
    }

    const rules = Array.isArray(detail.rules) ? detail.rules : [];
    if (rules.length > 0) {
      const section = addSection(body, "通行规则", `${rules.length} 条`);
      const list = el("div", "rule-list");
      for (const rule of rules) {
        const card = el("article", "rule-card");
        card.append(el("strong", null, rule.plateName || "所有车辆"));
        const metaParts = [];
        if (Array.isArray(rule.dayTypeNames) && rule.dayTypeNames.length > 0) {
          metaParts.push(rule.dayTypeNames.join(" / "));
        }
        if (Array.isArray(rule.tail) && rule.tail.length > 0) {
          metaParts.push(`尾号 ${rule.tail.join("、")}`);
        }
        if (rule.time) {
          metaParts.push(rule.time);
        }
        card.append(el("div", "rule-meta", metaParts.join(" · ") || "未提供额外限制"));
        list.append(card);
      }
      section.append(list);
    }

    const tickets = Array.isArray(detail.tickets) ? detail.tickets : [];
    const ticketCountText =
      detail.ticketsCount > 0 ? `已抓拍 ${detail.ticketsCount} 人` : `${tickets.length} 条`;
    const ticketSection = addSection(body, "现场罚单", ticketCountText);
    if (tickets.length === 0) {
      ticketSection.append(
        el("div", "section-empty", "暂无公开罚单记录；这不代表该点位没有抓拍。"),
      );
    } else {
      const list = el("div", "ticket-list");
      for (const ticket of tickets) {
        const card = el("article", "ticket-card");
        const titleRow = el("div", "ticket-title");
        titleRow.append(
          el("strong", null, ticket.title || "现场罚单"),
          el("span", "ticket-date", formatDateTime(ticket.createDate)),
        );
        card.append(titleRow);
        if (ticket.remark) {
          card.append(el("div", "ticket-remark", ticket.remark));
        }
        if (ticket.photo) {
          const figure = el("a", "ticket-photo");
          figure.href = imageUrl(ticket.photo);
          figure.target = "_blank";
          figure.rel = "noreferrer noopener";
          const image = document.createElement("img");
          image.loading = "lazy";
          image.alt = "现场照片";
          image.src = imageUrl(ticket.photo);
          image.addEventListener("error", () => {
            figure.replaceChildren(el("span", "muted", "照片加载失败，点击查看原图"));
          });
          figure.append(image);
          card.append(figure);
        }
        list.append(card);
      }
      ticketSection.append(list);
    }

    const stats = el("div", "stat-row");
    if (detail.praiseCount > 0) {
      addChip(stats, `点赞 ${detail.praiseCount}`, "chip-neutral");
    }
    if (detail.commentCount > 0) {
      addChip(stats, `评论 ${detail.commentCount}`, "chip-neutral");
    }
    const position = detail.position || camera.position;
    if (position && Number.isFinite(position.latitude) && Number.isFinite(position.longitude)) {
      addChip(
        stats,
        `WGS ${position.latitude.toFixed(5)}, ${position.longitude.toFixed(5)}`,
        "chip-neutral",
      );
    }
    if (stats.childElementCount > 0) {
      body.append(stats);
    }

    const actions = el("div", "detail-actions");
    const refresh = el(
      "button",
      "button button-ghost",
      detail.cached ? "重新获取最新详情" : "再次获取",
    );
    refresh.type = "button";
    refresh.addEventListener("click", () => selectCamera(camera, { refresh: true }));
    actions.append(refresh);
    body.append(actions);

    dom.detailBody.replaceChildren(body);
  }

  async function selectCamera(camera, options = {}) {
    setSelectedMarker(camera.id);

    if (!camera.official) {
      renderDemoDetail(camera);
      toast("这是演示点位。登录并同步后可查看官方点位详情。");
      return;
    }
    if (!state.app || !state.app.loggedIn) {
      renderLoginRequired(camera);
      openLogin();
      toast("请先登录 Motu 账号", "error");
      return;
    }

    const detailId = camera.memberByString || camera.member;
    if (!detailId) {
      renderPanelError("该点位没有 memberByString，无法请求官方详情。");
      return;
    }

    const requestId = ++state.detailRequest;
    renderLoading(camera);
    try {
      const detail = await api("/api/detail", {
        method: "POST",
        body: JSON.stringify({ cameraId: detailId, refresh: Boolean(options.refresh) }),
      });
      if (requestId !== state.detailRequest) {
        return;
      }
      renderDetail(camera, detail);
      refreshCacheStatus();
    } catch (error) {
      if (requestId !== state.detailRequest) {
        return;
      }
      renderPanelError(error.message);
      toast(error.message, "error");
    }
  }

  function openLogin() {
    dom.loginError.hidden = true;
    dom.loginError.textContent = "";
    if (!dom.loginDialog.open) {
      dom.loginDialog.showModal();
    }
    setTimeout(() => dom.loginPhone.focus(), 30);
  }

  function closeLogin() {
    if (dom.loginDialog.open) {
      dom.loginDialog.close();
    }
  }

  async function refreshCacheStatus() {
    try {
      const response = await api("/api/cache");
      const stats = response.stats || {};
      dom.cacheStatus.textContent = `本地缓存：${stats.cameras || 0} 点位 · ${stats.details || 0} 详情 · ${stats.images || 0} 图片`;
    } catch {
      dom.cacheStatus.textContent = "本地缓存：不可用";
    }
  }

  async function handleCacheClear() {
    if (!window.confirm("清空本机缓存？下次查看需要重新请求官方接口。")) {
      return;
    }
    dom.cacheClear.disabled = true;
    try {
      await api("/api/cache/clear", { method: "POST" });
      applyAppState(await api("/api/state"));
      await refreshViewport({ keepDetail: false });
      await refreshCacheStatus();
      toast("本地缓存已清空。");
    } catch (error) {
      toast(error.message, "error");
    } finally {
      dom.cacheClear.disabled = false;
    }
  }

  async function handleLogin(event) {
    event.preventDefault();
    const phone = dom.loginPhone.value.trim();
    const password = dom.loginPassword.value;
    if (!phone || !password) {
      dom.loginError.textContent = "手机号和密码不能为空。";
      dom.loginError.hidden = false;
      return;
    }

    dom.loginError.hidden = true;
    dom.loginSubmit.disabled = true;
    dom.loginSubmit.textContent = "登录中…";
    try {
      const result = await api("/api/login", {
        method: "POST",
        body: JSON.stringify({ phone, password }),
      });
      applyAppState(result.state);
      closeLogin();
      dom.loginPassword.value = "";
      await refreshViewport({ keepDetail: false });
      clearDetail();
      toast("登录成功，可以同步官方点位了。");
    } catch (error) {
      dom.loginError.textContent = error.message;
      dom.loginError.hidden = false;
    } finally {
      dom.loginSubmit.disabled = false;
      dom.loginSubmit.textContent = "登录";
    }
  }

  async function handleLogout() {
    if (!window.confirm("退出 Motu 账号？")) {
      return;
    }
    dom.logoutButton.disabled = true;
    try {
      const result = await api("/api/logout", { method: "POST" });
      applyAppState(result.state);
      await refreshViewport({ keepDetail: false });
      clearDetail();
      if (result.warning) {
        toast(`已退出登录。服务器登出失败：${result.warning}`, "error");
      } else {
        toast("已退出登录。");
      }
    } catch (error) {
      toast(error.message, "error");
    } finally {
      dom.logoutButton.disabled = false;
    }
  }

  function applyPickedPoint(point) {
    if (state.pickMode === "origin") {
      state.routeOrigin = point;
      dom.routeOrigin.setAttribute("aria-pressed", "false");
      dom.routeHint.textContent = "起点已选，继续选择终点。";
    } else {
      state.routeDestination = point;
      dom.routeDestination.setAttribute("aria-pressed", "false");
      dom.routeHint.textContent = "终点已选，点击「规划」使用本地路网计算。";
    }
    state.pickMode = null;
    renderRoutePoints();
  }

  function handleMapClick(event) {
    if (!state.pickMode) {
      clearDetail();
      return;
    }
    applyPickedPoint({ lat: event.latlng.lat, lng: event.latlng.lng });
  }

  function setPickMode(mode) {
    if (state.pickMode === mode) {
      state.pickMode = null;
    } else {
      state.pickMode = mode;
    }
    dom.routeOrigin.setAttribute("aria-pressed", state.pickMode === "origin" ? "true" : "false");
    dom.routeDestination.setAttribute(
      "aria-pressed",
      state.pickMode === "destination" ? "true" : "false",
    );
    dom.routeHint.textContent =
      state.pickMode === "origin"
        ? "点击地图设置起点。"
        : state.pickMode === "destination"
          ? "点击地图设置终点。"
          : "在地图上点选起点和终点，使用本地路网规划。";
  }

  function renderRoutePoints() {
    state.routeLayer.clearLayers();
    if (state.routeOrigin) {
      L.circleMarker([state.routeOrigin.lat, state.routeOrigin.lng], {
        radius: 7,
        color: "#ffffff",
        weight: 2,
        fillColor: "#0f7a4d",
        fillOpacity: 1,
      })
        .bindTooltip("起点", { direction: "top", className: "camera-tooltip" })
        .addTo(state.routeLayer);
    }
    if (state.routeDestination) {
      L.circleMarker([state.routeDestination.lat, state.routeDestination.lng], {
        radius: 7,
        color: "#ffffff",
        weight: 2,
        fillColor: "#b91c1c",
        fillOpacity: 1,
      })
        .bindTooltip("终点", { direction: "top", className: "camera-tooltip" })
        .addTo(state.routeLayer);
    }
    if (state.routePlan) {
      L.polyline(
        state.routePlan.points.map((point) => [point.latitude, point.longitude]),
        { color: "#0f766e", weight: 5, opacity: 0.85 },
      ).addTo(state.routeLayer);
    }
  }

  async function planRoute() {
    if (!state.routeOrigin || !state.routeDestination) {
      toast("请先在地图上点选起点和终点。", "error");
      return;
    }
    dom.routeGo.disabled = true;
    dom.routeHint.textContent = "正在用本地路网计算…";
    try {
      const plan = await api("/api/route", {
        method: "POST",
        body: JSON.stringify({
          origin: {
            latitude: state.routeOrigin.lat,
            longitude: state.routeOrigin.lng,
          },
          destination: {
            latitude: state.routeDestination.lat,
            longitude: state.routeDestination.lng,
          },
          avoidCameras: Boolean(dom.routeAvoid.checked),
          cameraRadiusM: Number(dom.routeRadius.value) || 60,
        }),
      });
      state.routePlan = plan;
      renderRoutePoints();
      const bounds = L.latLngBounds(plan.points.map((p) => [p.latitude, p.longitude]));
      state.map.fitBounds(bounds, { padding: [48, 48] });
      const km = (plan.distanceM / 1000).toFixed(1);
      const minutes = Math.max(1, Math.round(plan.durationS / 60));
      const avoidance = plan.avoidance || null;
      let suffix = "";
      if (avoidance) {
        const extraMinutes = Math.max(0, Math.round(avoidance.extraSeconds / 60));
        if (avoidance.applied) {
          suffix = ` · 已知点位 ${avoidance.camerasBefore}→${avoidance.camerasAfter}，多 ${extraMinutes} 分钟`;
          if (avoidance.camerasAfter > 0 && avoidance.reason) {
            suffix += `（${avoidance.reason}）`;
          }
        } else if (avoidance.reason) {
          suffix = ` · ${avoidance.reason}`;
        }
      }
      dom.routeHint.textContent = `本地路线 ${km} km · 约 ${minutes} 分钟 · ${plan.points.length} 个点${suffix}`;
      renderRouteDetail(plan, km, minutes, avoidance);
      updateCruiseAlert();
    } catch (error) {
      dom.routeHint.textContent = error.message;
      toast(`规划失败：${error.message}`, "error");
    } finally {
      dom.routeGo.disabled = false;
    }
  }

  function renderRouteDetail(plan, km, minutes, avoidance) {
    setDetailHeader("离线路线", `${km} km · 约 ${minutes} 分钟`);
    const body = document.createDocumentFragment();
    const summary = el("div", "route-summary");
    addChip(summary, `${plan.distanceM.toFixed(0)} 米`, "chip-neutral");
    addChip(summary, `${minutes} 分钟`, "chip-neutral");
    addChip(summary, `${plan.points.length} 个路网点`, "chip-neutral");
    addChip(summary, "本地路网 · 无网络", "chip-ok");
    if (avoidance) {
      addChip(
        summary,
        `已知电子眼 ${avoidance.camerasBefore} → ${avoidance.camerasAfter}`,
        avoidance.applied ? "chip-ok" : "chip-warn",
      );
      const extra = Math.max(0, Math.round(avoidance.extraSeconds / 60));
      addChip(summary, extra > 0 ? `多 ${extra} 分钟` : "无额外耗时", "chip-neutral");
      if (avoidance.reason) {
        addChip(summary, avoidance.reason, avoidance.applied ? "chip-warn" : "chip-neutral");
      }
    }
    body.append(summary);

    const section = addSection(body, "导航步骤", `${plan.steps.length} 步`);
    plan.steps.forEach((step, index) => {
      const row = el("div", "route-step");
      row.append(el("span", "route-step-index", index + 1));
      const text = el("div");
      text.append(el("strong", null, step.instruction));
      if (step.distanceM > 0) {
        text.append(
          el("div", "route-hint", `${step.distanceM.toFixed(0)} 米${step.road ? ` · ${step.road}` : ""}`),
        );
      }
      row.append(text);
      section.append(row);
    });
    dom.detailBody.replaceChildren(body);
  }

  function clearRoute() {
    state.routeOrigin = null;
    state.routeDestination = null;
    state.routePlan = null;
    state.pickMode = null;
    state.routeLayer.clearLayers();
    dom.routeOrigin.setAttribute("aria-pressed", "false");
    dom.routeDestination.setAttribute("aria-pressed", "false");
    dom.routeHint.textContent = "在地图上点选起点和终点，使用本地路网规划。";
  }

  function applyOffline(offline) {
    state.offline = Boolean(offline);
    dom.offlineBadge.hidden = !state.offline;
    if (state.offline) {
      dom.syncButton.disabled = true;
      dom.syncButton.title = "离线模式：不访问官方接口";
      dom.cruiseButton.disabled = true;
      dom.loginButton.hidden = true;
      dom.logoutButton.disabled = true;
      setCruise(false, { quiet: true });
    }
  }


  let basemapTimer = null;

  async function refreshBasemap() {
    if (!state.offline || !state.map) {
      return;
    }
    const bounds = state.map.getBounds();
    const bbox = [
      bounds.getSouth(),
      bounds.getWest(),
      bounds.getNorth(),
      bounds.getEast(),
    ]
      .map((value) => value.toFixed(6))
      .join(",");
    try {
      const response = await api(
        `/api/basemap?bbox=${encodeURIComponent(bbox)}&zoom=${state.map.getZoom()}`,
      );
      renderBasemap(response);
    } catch (error) {
      console.warn("basemap:", error.message);
    }
  }

  function renderBasemap(response) {
    state.basemapLayer.clearLayers();
    const groups = { 1: [], 2: [], 3: [] };
    for (const segment of response.segments || []) {
      const level = Math.min(3, Math.max(1, Number(segment[4]) || 3));
      groups[level].push([
        [segment[0], segment[1]],
        [segment[2], segment[3]],
      ]);
    }
    const styles = {
      1: { color: "#64748b", weight: 4, opacity: 0.95 },
      2: { color: "#94a3b8", weight: 2.5, opacity: 0.95 },
      3: { color: "#cbd5e1", weight: 1.5, opacity: 0.95 },
    };
    for (const level of [3, 2, 1]) {
      if (!groups[level].length) {
        continue;
      }
      if (level === 1) {
        L.polyline(groups[level], {
          color: "#ffffff",
          weight: 7,
          opacity: 0.9,
        }).addTo(state.basemapLayer);
      }
      L.polyline(groups[level], styles[level]).addTo(state.basemapLayer);
    }
  }

  function scheduleBasemapRefresh() {
    if (!state.offline) {
      return;
    }
    clearTimeout(basemapTimer);
    basemapTimer = setTimeout(refreshBasemap, 180);
  }

  function metersBetween(lat1, lon1, lat2, lon2) {
    const radius = 6371008.8;
    const phi1 = (lat1 * Math.PI) / 180;
    const phi2 = (lat2 * Math.PI) / 180;
    const dphi = ((lat2 - lat1) * Math.PI) / 180;
    const dlambda = ((lon2 - lon1) * Math.PI) / 180;
    const a =
      Math.sin(dphi / 2) ** 2 +
      Math.cos(phi1) * Math.cos(phi2) * Math.sin(dlambda / 2) ** 2;
    return 2 * radius * Math.asin(Math.sqrt(a));
  }

  function bearingDegrees(lat1, lon1, lat2, lon2) {
    const phi1 = (lat1 * Math.PI) / 180;
    const phi2 = (lat2 * Math.PI) / 180;
    const delta = ((lon2 - lon1) * Math.PI) / 180;
    const y = Math.sin(delta) * Math.cos(phi2);
    const x =
      Math.cos(phi1) * Math.sin(phi2) -
      Math.sin(phi1) * Math.cos(phi2) * Math.cos(delta);
    return ((Math.atan2(y, x) * 180) / Math.PI + 360) % 360;
  }

  function destinationPoint(lat, lon, bearing, distance) {
    const radius = 6371008.8;
    const delta = distance / radius;
    const theta = (bearing * Math.PI) / 180;
    const phi1 = (lat * Math.PI) / 180;
    const lambda1 = (lon * Math.PI) / 180;
    const phi2 = Math.asin(
      Math.sin(phi1) * Math.cos(delta) +
        Math.cos(phi1) * Math.sin(delta) * Math.cos(theta),
    );
    const lambda2 =
      lambda1 +
      Math.atan2(
        Math.sin(theta) * Math.sin(delta) * Math.cos(phi1),
        Math.cos(delta) - Math.sin(phi1) * Math.sin(phi2),
      );
    return {
      lat: (phi2 * 180) / Math.PI,
      lng: (((lambda2 * 180) / Math.PI + 540) % 360) - 180,
    };
  }

  function distanceToSegmentM(lat, lon, from, to) {
    const kx = 111_320 * Math.cos((lat * Math.PI) / 180);
    const ky = 110_540;
    const px = (lon - from.longitude) * kx;
    const py = (lat - from.latitude) * ky;
    const bx = (to.longitude - from.longitude) * kx;
    const by = (to.latitude - from.latitude) * ky;
    const lengthSquared = bx * bx + by * by;
    const t =
      lengthSquared > 0
        ? Math.max(0, Math.min(1, (px * bx + py * by) / lengthSquared))
        : 0;
    return Math.hypot(px - bx * t, py - by * t);
  }

  function distanceToRoute(point) {
    const plan = state.routePlan;
    if (!plan || !Array.isArray(plan.points) || plan.points.length < 2) {
      return null;
    }
    let best = Infinity;
    for (let index = 0; index < plan.points.length - 1; index += 1) {
      best = Math.min(
        best,
        distanceToSegmentM(
          point.lat,
          point.lng,
          plan.points[index],
          plan.points[index + 1],
        ),
      );
    }
    return best;
  }

  function renderPosition() {
    state.positionLayer.clearLayers();
    if (!state.position) {
      return;
    }
    const { lat, lng, heading } = state.position;
    L.circleMarker([lat, lng], {
      radius: 8,
      color: "#ffffff",
      weight: 3,
      fillColor: "#2563eb",
      fillOpacity: 1,
    }).addTo(state.positionLayer);
    if (Number.isFinite(heading)) {
      const end = destinationPoint(lat, lng, heading, 90);
      L.polyline(
        [
          [lat, lng],
          [end.lat, end.lng],
        ],
        { color: "#2563eb", weight: 3, opacity: 0.85 },
      ).addTo(state.positionLayer);
    }
  }

  function nearestRouteIndex(point, points) {
    let best = 0;
    let bestDistance = Infinity;
    for (let index = 0; index < points.length; index += 1) {
      const distance = metersBetween(
        point.lat,
        point.lng,
        points[index].latitude,
        points[index].longitude,
      );
      if (distance < bestDistance) {
        bestDistance = distance;
        best = index;
      }
    }
    return best;
  }

  function updateCruiseAlert() {
    if (!state.position) {
      dom.cruiseAlert.hidden = true;
      return;
    }
    const camera = state.nearestCamera;
    const cameraLine =
      camera && Number(camera.distanceM) <= 500
        ? `前方 ${Math.round(Number(camera.distanceM))} m · ${camera.primaryLabel || "电子眼"}`
        : "";

    let routeLine = "";
    let remainingLine = "";
    const plan = state.routePlan;
    if (
      plan &&
      state.routeDestination &&
      Array.isArray(plan.points) &&
      plan.points.length > 1
    ) {
      const index = nearestRouteIndex(state.position, plan.points);
      const step = (plan.steps || []).find(
        (candidate) =>
          candidate.direction !== "arrive" &&
          Number.isFinite(Number(candidate.endIndex)) &&
          Number(candidate.endIndex) > index,
      );
      if (step) {
        const end = plan.points[Math.min(Number(step.endIndex), plan.points.length - 1)];
        const distance = metersBetween(
          state.position.lat,
          state.position.lng,
          end.latitude,
          end.longitude,
        );
        routeLine = `${step.instruction} · ${Math.round(distance)} m`;
      }
      const remaining = metersBetween(
        state.position.lat,
        state.position.lng,
        state.routeDestination.lat,
        state.routeDestination.lng,
      );
      remainingLine = `距终点 ${(remaining / 1000).toFixed(1)} km`;
    }

    const primary =
      routeLine ||
      cameraLine ||
      `GPS ${state.position.lat.toFixed(5)}, ${state.position.lng.toFixed(5)}`;
    const secondary = routeLine ? cameraLine || remainingLine : remainingLine;
    dom.cruiseStep.textContent = primary;
    dom.cruiseCamera.textContent = secondary;
    dom.cruiseAlert.hidden = false;
  }

  async function checkNearby() {
    if (!state.position) {
      return;
    }
    const now = Date.now();
    if (now - state.lastNearbyAt < 2000) {
      return;
    }
    state.lastNearbyAt = now;
    try {
      const response = await api(
        `/api/cameras/nearby?lat=${state.position.lat}&lon=${state.position.lng}&radius=600`,
      );
      const nearest = (response.cameras || [])[0] || null;
      state.nearestCamera = nearest;
      if (nearest && Number(nearest.distanceM) <= 500) {
        const key = nearest.memberByString || nearest.id;
        if (state.alertedId !== key || now - state.alertedAt > 20000) {
          state.alertedId = key;
          state.alertedAt = now;
        }
      }
      updateCruiseAlert();
    } catch (error) {
      console.warn("nearby:", error.message);
    }
  }

  async function replanFromPosition() {
    if (!state.position || !state.routeDestination) {
      return;
    }
    try {
      const plan = await api("/api/route", {
        method: "POST",
        body: JSON.stringify({
          origin: {
            latitude: state.position.lat,
            longitude: state.position.lng,
          },
          destination: {
            latitude: state.routeDestination.lat,
            longitude: state.routeDestination.lng,
          },
        }),
      });
      state.routePlan = plan;
      renderRoutePoints();
      updateCruiseAlert();
      toast("已偏离路线：按当前位置重新规划。");
    } catch (error) {
      toast(`重算失败：${error.message}`, "error");
    }
  }

  function checkOffRoute() {
    if (!state.routePlan || !state.position) {
      state.offRouteSince = null;
      return;
    }
    const distance = distanceToRoute(state.position);
    const now = Date.now();
    if (distance !== null && distance > 80) {
      if (!state.offRouteSince) {
        state.offRouteSince = now;
      } else if (now - state.offRouteSince > 4000 && now - state.lastRerouteAt > 15000) {
        state.offRouteSince = null;
        state.lastRerouteAt = now;
        replanFromPosition();
      }
    } else {
      state.offRouteSince = null;
    }
  }

  // Local debugging hook: `window.dispatchEvent(new CustomEvent("motu:gps",
  // { detail: { latitude, longitude, heading } }))` while GPS cruise is on.
  // Browsers do not let a test drive `watchPosition`, and a desktop debugger
  // may want to replay a track.
  window.addEventListener("motu:gps", (event) => {
    if (state.watchId === null) {
      return;
    }
    const detail = event.detail || {};
    if (!Number.isFinite(detail.latitude) || !Number.isFinite(detail.longitude)) {
      return;
    }
    onGpsPosition({
      coords: {
        latitude: detail.latitude,
        longitude: detail.longitude,
        heading: Number.isFinite(detail.heading) ? detail.heading : null,
        speed: null,
      },
    });
  });

  function onGpsPosition(position) {
    const coords = position.coords;
    const previous = state.position;
    let heading = coords.heading;
    if (heading === null || Number.isNaN(heading)) {
      heading = previous
        ? bearingDegrees(previous.lat, previous.lng, coords.latitude, coords.longitude)
        : null;
    }
    state.position = {
      lat: coords.latitude,
      lng: coords.longitude,
      heading,
    };
    renderPosition();
    if (state.map) {
      state.map.setView(
        [coords.latitude, coords.longitude],
        Math.max(state.map.getZoom(), 16),
        { animate: true },
      );
    }
    checkNearby();
    checkOffRoute();
    updateCruiseAlert();
  }

  function onGpsError(error) {
    toast(`定位失败：${error.message}`, "error");
    stopGps(true);
  }

  function stopGps(quiet = false) {
    if (state.watchId !== null) {
      navigator.geolocation.clearWatch(state.watchId);
    }
    state.watchId = null;
    if (state.cruiseTimer !== null) {
      clearInterval(state.cruiseTimer);
      state.cruiseTimer = null;
    }
    state.position = null;
    state.nearestCamera = null;
    state.offRouteSince = null;
    dom.gpsButton.setAttribute("aria-pressed", "false");
    state.positionLayer.clearLayers();
    dom.cruiseAlert.hidden = true;
    if (!quiet) {
      toast("GPS 巡航已关闭。");
    }
  }

  function toggleGps() {
    if (state.watchId !== null) {
      stopGps();
      return;
    }
    if (!("geolocation" in navigator)) {
      toast("当前浏览器不支持定位。", "error");
      return;
    }
    state.watchId = navigator.geolocation.watchPosition(
      onGpsPosition,
      onGpsError,
      { enableHighAccuracy: true, maximumAge: 1000, timeout: 20000 },
    );
    state.cruiseTimer = setInterval(() => {
      checkOffRoute();
      updateCruiseAlert();
    }, 2000);
    dom.gpsButton.setAttribute("aria-pressed", "true");
    toast("GPS 巡航已开启：地图跟随定位，电子眼从本地缓存提醒。");
  }

  const AUTO_SYNC_COOLDOWN_MS = 15000;
  let viewportTimer = null;
  let autoSyncTimer = null;
  let lastAutoSyncAt = 0;
  let autoSyncFailedAt = 0;

  function syncPayload(extra = {}) {
    const center = state.map.getCenter();
    const bounds = state.map.getBounds();
    return {
      latitude: center.lat,
      longitude: center.lng,
      cityCode: dom.cityCode.value.trim() || null,
      south: bounds.getSouth(),
      west: bounds.getWest(),
      north: bounds.getNorth(),
      east: bounds.getEast(),
      ...extra,
    };
  }

  function renderAreas(response) {
    state.areasLayer.clearLayers();
    const areas = Array.isArray(response.areas) ? response.areas : [];
    let polygons = 0;
    let lines = 0;
    for (const area of areas) {
      const points = (area.points || [])
        .filter(
          (point) =>
            Number.isFinite(point.latitude) && Number.isFinite(point.longitude),
        )
        .map((point) => [point.latitude, point.longitude]);
      if (points.length < 2) {
        continue;
      }
      const isPolygon = area.shapeType === "polygon";
      const shape = isPolygon
        ? L.polygon(points, {
            pane: "areasPane",
            color: "#b91c1c",
            weight: 2,
            opacity: 0.9,
            fillColor: "#b91c1c",
            fillOpacity: 0.12,
            dashArray: "6 6",
          })
        : L.polyline(points, {
            pane: "areasPane",
            color: "#b91c1c",
            weight: 2,
            opacity: 0.8,
            dashArray: "4 8",
          });
      shape.bindTooltip(area.name || (isPolygon ? "限行区域" : "限行路段"), {
        sticky: true,
        className: "camera-tooltip",
      });
      shape.addTo(state.areasLayer);
      if (isPolygon) {
        polygons += 1;
      } else {
        lines += 1;
      }
    }
    return { polygons, lines, total: areas.length };
  }

  async function ensureAreas(force = false) {
    if (state.areas && !force) {
      return state.areas;
    }
    const response = await api(`/api/areas${force ? "?refresh=true" : ""}`);
    state.areas = response;
    renderAreas(response);
    dom.areasButton.textContent = `限行区域 (${response.areas ? response.areas.length : 0})`;
    return response;
  }

  async function toggleAreas(event) {
    if (!state.areasVisible) {
      dom.areasButton.disabled = true;
      try {
        // Shift+click bypasses the cached layer (never in offline mode).
        const response = await ensureAreas(Boolean(event && event.shiftKey) && !state.offline);
        state.areasLayer.addTo(state.map);
        state.areasVisible = true;
        dom.areasButton.setAttribute("aria-pressed", "true");
        const counts = renderAreas(response);
        const suffix = response.cached
          ? `本地缓存 · ${formatAge(response.fetchedAtMs)}`
          : "刚刚获取";
        toast(`限行区域：${counts.polygons} 个多边形、${counts.lines} 条路段（${suffix}）`);
        if (response.warning) {
          toast(`区域刷新失败，显示旧缓存：${response.warning}`, "error");
        }
      } catch (error) {
        toast(`限行区域加载失败：${error.message}`, "error");
      } finally {
        dom.areasButton.disabled = false;
      }
      return;
    }
    state.map.removeLayer(state.areasLayer);
    state.areasVisible = false;
    dom.areasButton.setAttribute("aria-pressed", "false");
  }

  function setCruise(enabled, options = {}) {
    state.cruise = Boolean(enabled);
    dom.cruiseButton.setAttribute("aria-pressed", state.cruise ? "true" : "false");
    try {
      localStorage.setItem("motu.cruise", state.cruise ? "1" : "0");
    } catch {
      // Private mode without storage: the toggle still works for this page.
    }
    if (!options.quiet) {
      toast(
        state.cruise
          ? "已开启自动刷新：进入未覆盖视野时自动同步（15 秒冷却，巡航标记）。"
          : "已关闭自动刷新。",
      );
    }
    if (state.cruise) {
      scheduleAutoSync();
    }
  }

  async function autoSync() {
    if (state.offline) {
      return;
    }
    if (!state.cruise || !state.app || !state.app.loggedIn || state.syncing || !state.map) {
      return;
    }
    if (Date.now() - lastAutoSyncAt < AUTO_SYNC_COOLDOWN_MS) {
      return;
    }
    // Empty areas answer with a business error ("未匹配到对应电子眼数据");
    // back off instead of retrying (and toasting) on every pan.
    if (Date.now() - autoSyncFailedAt < 60_000) {
      return;
    }
    lastAutoSyncAt = Date.now();
    state.syncing = true;
    try {
      const result = await api("/api/sync", {
        method: "POST",
        body: JSON.stringify(syncPayload({ cruise: true })),
      });
      applyAppState(result.state);
      if (!result.skipped) {
        await refreshViewport({ keepDetail: true });
        await refreshCacheStatus();
        if (result.synced > 0) {
          toast(`巡航自动同步：${result.synced} 个点位。`);
        }
      }
    } catch (error) {
      autoSyncFailedAt = Date.now();
      console.warn("cruise auto sync skipped:", error.message);
    } finally {
      state.syncing = false;
    }
  }

  function scheduleAutoSync() {
    if (!state.cruise || !state.app || !state.app.loggedIn) {
      return;
    }
    clearTimeout(autoSyncTimer);
    autoSyncTimer = setTimeout(autoSync, 900);
  }

  async function refreshViewport(options = {}) {
    if (!state.map) {
      return;
    }
    const bounds = state.map.getBounds();
    const bbox = [
      bounds.getSouth(),
      bounds.getWest(),
      bounds.getNorth(),
      bounds.getEast(),
    ]
      .map((value) => value.toFixed(6))
      .join(",");
    const requestId = ++state.viewportRequest;
    try {
      const snapshot = await api(`/api/cameras?bbox=${encodeURIComponent(bbox)}`);
      if (requestId !== state.viewportRequest) {
        return;
      }
      renderSnapshot(snapshot, { keepDetail: options.keepDetail !== false });
      scheduleAutoSync();
    } catch (error) {
      if (requestId === state.viewportRequest) {
        toast(`视野加载失败：${error.message}`, "error");
      }
    }
  }

  function scheduleViewportRefresh() {
    clearTimeout(viewportTimer);
    viewportTimer = setTimeout(() => refreshViewport(), 160);
  }

  async function handleSync(event) {
    if (state.offline) {
      toast("离线模式：不会请求官方接口。", "error");
      return;
    }
    if (!state.app || !state.app.loggedIn) {
      openLogin();
      toast("请先登录 Motu 账号", "error");
      return;
    }
    if (!state.map) {
      toast("地图还没有准备好。", "error");
      return;
    }

    const force = Boolean(event && event.shiftKey);
    const originalText = dom.syncButton.textContent;
    state.syncing = true;
    lastAutoSyncAt = Date.now();
    dom.syncButton.disabled = true;
    dom.syncButton.textContent = "同步中…";
    try {
      const result = await api("/api/sync", {
        method: "POST",
        body: JSON.stringify(syncPayload({ force })),
      });
      applyAppState(result.state);
      await refreshViewport({ keepDetail: false });
      await refreshCacheStatus();
      const synced = Number.isFinite(result.synced) ? result.synced : 0;
      if (result.skipped) {
        toast(
          `本视野已在缓存覆盖范围内（${formatAge(result.syncedAtMs)}同步），未请求官方接口；Shift+点击可强制刷新。`,
        );
      } else if (synced === 0) {
        toast("同步成功，但官方返回 0 个点位；请移动地图中心后重试。", "error");
      } else {
        toast(`已同步 ${synced} 个官方点位并写入缓存，当前视野已刷新。`);
      }
    } catch (error) {
      toast(error.message, "error");
    } finally {
      state.syncing = false;
      dom.syncButton.disabled = false;
      dom.syncButton.textContent = originalText;
    }
  }

  function bindEvents() {
    dom.syncButton.addEventListener("click", handleSync);
    dom.cacheClear.addEventListener("click", handleCacheClear);
    dom.areasButton.addEventListener("click", toggleAreas);
    dom.cruiseButton.addEventListener("click", () => setCruise(!state.cruise));
    dom.routeOrigin.addEventListener("click", () => setPickMode("origin"));
    dom.routeDestination.addEventListener("click", () => setPickMode("destination"));
    dom.routeGo.addEventListener("click", planRoute);
    dom.routeClear.addEventListener("click", clearRoute);
    dom.routeAvoid.addEventListener("change", () => {
      // Re-plan immediately so checking the box always takes effect.
      if (state.routePlan) {
        planRoute();
      }
    });
    dom.gpsButton.addEventListener("click", toggleGps);
    dom.loginButton.addEventListener("click", openLogin);
    dom.logoutButton.addEventListener("click", handleLogout);
    dom.loginForm.addEventListener("submit", handleLogin);
    dom.loginClose.addEventListener("click", closeLogin);
    dom.loginCancel.addEventListener("click", closeLogin);
    dom.loginDialog.addEventListener("click", (event) => {
      if (event.target === dom.loginDialog) {
        closeLogin();
      }
    });
    dom.detailClose.addEventListener("click", clearDetail);
    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape" && !dom.loginDialog.open) {
        clearDetail();
      }
    });
  }

  async function init() {
    bindEvents();
    try {
      setCruise(localStorage.getItem("motu.cruise") === "1", { quiet: true });
    } catch {
      // Storage unavailable.
    }
    try {
      const config = await api("/api/config");
      state.config = config;
      applyOffline(config.offline);
      initMap(config);
      refreshBasemap();
      const app = await api("/api/state");
      applyAppState(app);
      await refreshViewport();
      refreshCacheStatus();
      if (state.snapshot && state.snapshot.source === "demo") {
        toast("当前是演示点位。登录后同步官方点位。");
      }
    } catch (error) {
      renderPanelError(`无法连接本机 Rust 服务：${error.message}`);
      toast(`初始化失败：${error.message}`, "error");
    }
  }

  init();
})();
