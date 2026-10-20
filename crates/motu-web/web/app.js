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

    state.markerLayer = L.layerGroup().addTo(state.map);
    L.control.scale({ imperial: false, maxWidth: 120 }).addTo(state.map);
    state.map.on("click", clearDetail);
    window.addEventListener("resize", () => state.map.invalidateSize());
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

  function renderSnapshot(snapshot) {
    state.snapshot = snapshot;
    state.markerLayer.clearLayers();
    state.markers.clear();
    state.camerasById.clear();
    state.selectedId = null;
    state.detailRequest += 1;

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
      marker.on("click", () => selectCamera(camera));
      marker.addTo(state.markerLayer);
      state.markers.set(camera.id, marker);
      state.camerasById.set(camera.id, camera);
    }

    const official = snapshot.source === "official";
    const count = (snapshot.cameras || []).length;
    dom.sourceBadge.textContent = official ? "官方点位" : "演示数据";
    dom.sourceBadge.className = `badge ${official ? "badge-official" : "badge-demo"}`;
    const clock = official && snapshot.fetchedAtMs ? ` · ${formatClock(snapshot.fetchedAtMs)}` : "";
    dom.cameraCount.textContent = `${count} 个点位${clock}`;
    clearDetail();
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
    hero.append(meta);
    if (detail.address) {
      hero.append(el("div", "address", detail.address));
    }
    body.append(hero);

    if (detail.detail) {
      const section = addSection(body, "电子眼说明");
      section.append(el("div", "detail-callout", detail.detail));
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
          const tags = el("div", "ticket-tags");
          addChip(tags, "有现场照片", "chip-warn");
          card.append(tags);
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

    dom.detailBody.replaceChildren(body);
  }

  async function selectCamera(camera) {
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
        body: JSON.stringify({ cameraId: detailId }),
      });
      if (requestId !== state.detailRequest) {
        return;
      }
      renderDetail(camera, detail);
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

  async function refreshCameras() {
    const snapshot = await api("/api/cameras");
    renderSnapshot(snapshot);
    return snapshot;
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
      await refreshCameras();
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
      await refreshCameras();
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

  async function handleSync() {
    if (!state.app || !state.app.loggedIn) {
      openLogin();
      toast("请先登录 Motu 账号", "error");
      return;
    }
    if (!state.map) {
      toast("地图还没有准备好。", "error");
      return;
    }

    const center = state.map.getCenter();
    const cityCode = dom.cityCode.value.trim();
    const originalText = dom.syncButton.textContent;
    dom.syncButton.disabled = true;
    dom.syncButton.textContent = "同步中…";
    try {
      const result = await api("/api/sync", {
        method: "POST",
        body: JSON.stringify({
          latitude: center.lat,
          longitude: center.lng,
          cityCode: cityCode || null,
        }),
      });
      applyAppState(result.state);
      renderSnapshot(result.snapshot);
      const count = (result.snapshot.cameras || []).length;
      if (count === 0) {
        toast("同步成功，但官方返回 0 个点位；请移动地图中心后重试。", "error");
      } else {
        toast(`已同步 ${count} 个官方点位。`);
      }
    } catch (error) {
      toast(error.message, "error");
    } finally {
      dom.syncButton.disabled = false;
      dom.syncButton.textContent = originalText;
    }
  }

  function bindEvents() {
    dom.syncButton.addEventListener("click", handleSync);
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
      const config = await api("/api/config");
      state.config = config;
      initMap(config);
      const [app, snapshot] = await Promise.all([api("/api/state"), api("/api/cameras")]);
      applyAppState(app);
      renderSnapshot(snapshot);
      if (snapshot.source === "demo") {
        toast("当前是演示点位。登录后同步官方点位。");
      }
    } catch (error) {
      renderPanelError(`无法连接本机 Rust 服务：${error.message}`);
      toast(`初始化失败：${error.message}`, "error");
    }
  }

  init();
})();
