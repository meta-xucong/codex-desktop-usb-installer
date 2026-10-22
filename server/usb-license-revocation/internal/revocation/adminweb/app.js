(() => {
  const $ = (id) => document.getElementById(id);
  const state = { csrf: "", offset: 0, limit: 20, total: 0, selected: null, items: [], modelCatalogTimer: null };

  async function request(path, options = {}) {
    const headers = new Headers(options.headers || {});
    headers.set("Accept", "application/json");
    if (options.body !== undefined) headers.set("Content-Type", "application/json");
    if (state.csrf && options.method && options.method !== "GET") headers.set("X-CSRF-Token", state.csrf);
    const response = await fetch(path, { ...options, headers, credentials: "same-origin", cache: "no-store" });
    let data = {};
    try { data = await response.json(); } catch (_) { /* handled below */ }
    if (!response.ok) {
      const error = new Error(data.error || `请求失败 (${response.status})`);
      error.status = response.status;
      throw error;
    }
    return data;
  }

  function showLogin(message = "") {
    state.csrf = "";
    if (state.modelCatalogTimer) clearTimeout(state.modelCatalogTimer);
    state.modelCatalogTimer = null;
    if ($("password-dialog").open) $("password-dialog").close();
    $("app-panel").hidden = true;
    $("first-setup-panel").hidden = true;
    $("first-setup-form").reset();
    $("totp-secret").textContent = "";
    $("login-panel").hidden = false;
    $("login-error").textContent = message;
  }

  function showApp(username) {
    $("login-panel").hidden = true;
    $("first-setup-panel").hidden = true;
    $("app-panel").hidden = false;
    $("signed-in-user").textContent = `已登录：${username}`;
  }

  async function showFirstSetup(username) {
    $("login-panel").hidden = true;
    $("app-panel").hidden = true;
    $("first-setup-panel").hidden = false;
    $("setup-user").textContent = username;
    $("setup-error").textContent = "";
    $("totp-secret").textContent = "正在生成密钥…";
    try {
      const data = await request("/admin/api/initial-setup");
      $("totp-secret").textContent = data.totpSecret;
    } catch (error) {
      if (error.status === 401) return showLogin("登录已过期，请重新登录。");
      $("totp-secret").textContent = "密钥暂时无法读取，请退出后重新登录。";
      $("setup-error").textContent = error.message;
    }
  }

  function setMessage(message, kind = "") {
    const node = $("app-message");
    node.textContent = message;
    node.className = `message ${kind}`;
  }

  function cell(row, value, className = "") {
    const node = document.createElement("td");
    node.textContent = value == null || value === "" ? "—" : String(value);
    if (className) node.className = className;
    row.append(node);
    return node;
  }

  function formatDate(value) {
    if (!value) return "—";
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? "—" : date.toLocaleString("zh-CN", { hour12: false });
  }

  function renderRows() {
    const tbody = $("license-rows");
    tbody.replaceChildren();
    for (const item of state.items) {
      const row = document.createElement("tr");
      cell(row, item.installationId, "mono");
      cell(row, item.usbAssetId, "mono");
      cell(row, item.licenseKeyId, "mono");
      const status = cell(row, item.status === "active" ? "可用" : "已作废");
      status.className = item.status === "active" ? "status-active" : "status-revoked";
      cell(row, item.stateVersion);
      cell(row, formatDate(item.createdAt));
      cell(row, item.licenseSignatureSha256, "mono digest");
      cell(row, item.fingerprintSha256, "mono digest");
      const actions = document.createElement("td");
      if (item.status === "active") {
        const button = document.createElement("button");
        button.type = "button";
        button.className = "small-danger";
        button.textContent = "作废";
        button.addEventListener("click", () => openRevokeDialog(item));
        actions.append(button);
      } else {
        actions.textContent = item.revokedAt ? `作废于 ${formatDate(item.revokedAt)}` : "—";
      }
      row.append(actions);
      tbody.append(row);
    }
    $("empty-state").hidden = state.total !== 0;
    $("total-count").textContent = String(state.total);
    $("active-count").textContent = String(state.items.filter((item) => item.status === "active").length);
    $("revoked-count").textContent = String(state.items.filter((item) => item.status === "revoked").length);
    const first = state.total === 0 ? 0 : state.offset + 1;
    const last = Math.min(state.offset + state.items.length, state.total);
    $("page-info").textContent = `第 ${first}–${last} 条，共 ${state.total} 条`;
    $("previous-page").disabled = state.offset === 0;
    $("next-page").disabled = state.offset + state.limit >= state.total;
  }

  async function loadLicenses() {
    setMessage("正在读取授权列表…");
    try {
      const data = await request(`/admin/api/licenses?offset=${state.offset}&limit=${state.limit}`);
      state.items = data.items || [];
      state.total = data.total || 0;
      renderRows();
      setMessage("");
    } catch (error) {
      if (error.status === 401) return showLogin("登录已过期，请重新登录。");
      setMessage(error.message, "error");
    }
  }

  const modelCatalogStatusNames = {
    idle: "尚未运行", running: "运行中", succeeded: "完成", no_plan: "没有可应用计划",
    partial: "部分完成", blocked: "已阻断", failed: "失败", interrupted: "已中断",
  };
  const modelCatalogReasonNames = {
    completed: "已完成", no_applicable_plan: "没有可应用计划", account_scan_partial: "账号目录扫描不完整",
    native_refresh_running: "原生模型刷新仍在运行", native_refresh_status_unavailable: "原生刷新状态暂不可用",
    native_refresh_status_unknown: "原生刷新状态未知", manager_lock_requires_review: "管理器遗留锁需要人工核验",
    configuration_invalid: "固定配置或程序不可用", credential_unavailable: "管理员 API 凭据不可用",
    discovery_failed: "目录发现失败", approved_rules_blocked: "批准规则校验未通过",
    apply_partial: "映射写入部分完成", apply_failed: "映射写入失败", process_timeout: "任务超时，结果未知",
    manager_result_invalid: "任务结果无效，映射结果未知", service_interrupted: "服务重启前任务未完成",
  };

  function formatModelCatalogTime(value) {
    if (!value) return "—";
    const parsed = new Date(value);
    return Number.isNaN(parsed.getTime()) ? "—" : parsed.toLocaleString("zh-CN", { hour12: false });
  }

  function renderModelCatalog(data) {
    $("model-catalog-status").textContent = modelCatalogStatusNames[data.status] || "状态未知";
    $("model-catalog-started").textContent = formatModelCatalogTime(data.started_at);
    $("model-catalog-finished").textContent = formatModelCatalogTime(data.finished_at);
    $("model-catalog-reason").textContent = modelCatalogReasonNames[data.reason_code] || (data.reason_code ? "原因未知" : "—");
    const count = (value) => Number.isInteger(value) && value >= 0 ? String(value) : "未知/不可用";
    $("model-catalog-planned").textContent = count(data.planned_count);
    $("model-catalog-applied").textContent = count(data.applied_count);
    $("model-catalog-noop").textContent = count(data.noop_count);
    $("model-catalog-failed").textContent = count(data.failed_count);
    $("model-catalog-not-attempted").textContent = count(data.not_attempted_count);
    const note = $("model-catalog-note");
    note.className = "message";
    const failedWriteUnverified = Number.isInteger(data.failed_count) && data.failed_count > 0;
    note.textContent = data.status === "interrupted"
      ? "任务结果无法核实，映射副作用可能未知。检查摘要和备份前不要重试。"
      : failedWriteUnverified
        ? "有账号的映射写入结果未核实；这不代表请求一定到达服务端。先核对摘要和备份，再决定是否重试。系统不会自动重试。"
        : data.status === "partial"
          ? "本轮只完成了部分工作。核对摘要和备份后再决定是否重试。"
          : data.status === "blocked" || data.status === "failed"
            ? "任务未成功完成；不会自动重试。请根据原因和摘要检查。"
            : data.status === "running" ? "任务运行中；完成前计数显示为未知。" : "";
    if (["partial", "blocked", "failed", "interrupted"].includes(data.status)) note.classList.add("error");
    const summary = $("model-catalog-summary");
    summary.textContent = typeof data.summary === "string" ? data.summary : "";
    summary.hidden = !summary.textContent;
    if (data.summary_unavailable) {
      note.textContent = `${note.textContent}${note.textContent ? " " : ""}安全摘要当前不可用。`;
    }
    const running = data.status === "running";
    $("model-catalog-run-button").disabled = running;
    $("model-catalog-run-button").textContent = running ? "任务运行中…" : "刷新并整理模型";
    if (state.modelCatalogTimer) clearTimeout(state.modelCatalogTimer);
    state.modelCatalogTimer = running ? setTimeout(() => {
      state.modelCatalogTimer = null;
      loadModelCatalog();
    }, 2500) : null;
  }

  function renderModelCatalogUnavailable(message) {
    const unknown = "未知/不可用";
    $("model-catalog-status").textContent = "暂不可用";
    $("model-catalog-started").textContent = "—";
    $("model-catalog-finished").textContent = "—";
    $("model-catalog-reason").textContent = "—";
    $("model-catalog-planned").textContent = unknown;
    $("model-catalog-applied").textContent = unknown;
    $("model-catalog-noop").textContent = unknown;
    $("model-catalog-failed").textContent = unknown;
    $("model-catalog-not-attempted").textContent = unknown;
    $("model-catalog-note").textContent = message || "任务状态无法读取；刷新页面后再确认当前状态。";
    $("model-catalog-note").className = "message error";
    const summary = $("model-catalog-summary");
    summary.textContent = "";
    summary.hidden = true;
    const button = $("model-catalog-run-button");
    button.disabled = true;
    button.textContent = "状态暂不可用";
    if (state.modelCatalogTimer) clearTimeout(state.modelCatalogTimer);
    state.modelCatalogTimer = setTimeout(() => {
      state.modelCatalogTimer = null;
      loadModelCatalog();
    }, 10000);
  }

  async function loadModelCatalog() {
    try {
      const data = await request("/admin/api/model-catalog/runs/current");
      renderModelCatalog(data);
    } catch (error) {
      if (error.status === 401) return showLogin("登录已过期，请重新登录。");
      renderModelCatalogUnavailable(error.message);
    }
  }

  function openRevokeDialog(item) {
    state.selected = item;
    $("revoke-installation-id").textContent = item.installationId;
    $("revoke-reason").value = "";
    $("revoke-confirm-check").checked = false;
    $("revoke-confirm-id").value = "";
    updateRevokeButton();
    $("revoke-dialog").showModal();
    $("revoke-reason").focus();
  }

  function openPasswordDialog() {
    $("password-form").reset();
    $("password-error").textContent = "";
    $("password-dialog").showModal();
    $("current-password").focus();
  }

  function updateRevokeButton() {
    const item = state.selected;
    $("confirm-revoke").disabled = !item || !$("revoke-confirm-check").checked || $("revoke-confirm-id").value !== item.installationId || !$("revoke-reason").value.trim();
  }

  async function importRegistration(file) {
    if (!file || file.size > 20 * 1024) {
      setMessage("登记文件过大或未选择文件。", "error");
      return;
    }
    let envelope;
    try {
      envelope = JSON.parse(await file.text());
      if (typeof envelope.payloadB64 !== "string" || typeof envelope.signatureB64 !== "string") throw new Error("登记文件字段不正确");
    } catch (error) {
      setMessage(`无法读取登记文件：${error.message}`, "error");
      return;
    }
    if (!window.confirm("确认导入这份签名器生成的 U 盘登记文件？导入后会在 VPS 创建 active 授权记录。")) return;
    setMessage("正在验证并登记…");
    try {
      const data = await request("/admin/api/licenses", { method: "POST", body: JSON.stringify({ registration: envelope }) });
      state.offset = 0;
      await loadLicenses();
      setMessage(`已登记 ${data.license.installationId}，状态为可用。`, "success");
    } catch (error) {
      if (error.status === 401) return showLogin("登录已过期，请重新登录。");
      setMessage(error.message, "error");
    } finally {
      $("registration-file").value = "";
    }
  }

  $("login-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    $("login-error").textContent = "";
    const button = event.currentTarget.querySelector("button[type=submit]");
    button.disabled = true;
    try {
      const data = await request("/admin/api/login", { method: "POST", body: JSON.stringify({ username: $("username").value, password: $("password").value }) });
      state.csrf = data.csrfToken;
      $("password").value = "";
      if (data.setupRequired) {
        await showFirstSetup(data.username);
        return;
      }
      showApp(data.username);
      await loadLicenses();
      await loadModelCatalog();
    } catch (error) {
      $("login-error").textContent = error.message;
    } finally {
      button.disabled = false;
    }
  });

  $("first-setup-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const newPassword = $("setup-new-password").value;
    if (newPassword !== $("setup-confirm-password").value) {
      $("setup-error").textContent = "两次输入的新密码不一致。";
      return;
    }
    const button = $("complete-setup-button");
    button.disabled = true;
    button.textContent = "正在保存…";
    $("setup-error").textContent = "";
    try {
      await request("/admin/api/initial-setup", {
        method: "POST",
        body: JSON.stringify({ newPassword, totpCode: $("setup-totp-code").value }),
      });
      showLogin("首次设置已完成，请使用新密码重新登录。请妥善保存 Google Authenticator 中的账号。 ");
    } catch (error) {
      if (error.status === 401) {
        $("setup-error").textContent = error.message;
        $("setup-totp-code").value = "";
        $("setup-totp-code").focus();
      } else if (error.status === 403) {
        showLogin("登录状态已失效，请重新登录并完成首次设置。 ");
      } else {
        $("setup-error").textContent = error.message;
      }
    } finally {
      button.disabled = false;
      button.textContent = "保存密码并完成绑定";
    }
  });

  $("setup-logout-button").addEventListener("click", async () => {
    try { await request("/admin/api/logout", { method: "POST", body: "{}" }); } catch (_) { /* clear local view anyway */ }
    showLogin("已退出登录。首次设置尚未完成。");
  });

  $("logout-button").addEventListener("click", async () => {
    try { await request("/admin/api/logout", { method: "POST", body: "{}" }); } catch (_) { /* clear local view anyway */ }
    showLogin("已退出登录。");
  });

  $("change-password-button").addEventListener("click", openPasswordDialog);
  $("cancel-password").addEventListener("click", () => $("password-dialog").close());
  $("password-dialog").addEventListener("close", () => $("password-form").reset());
  $("password-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const currentPassword = $("current-password").value;
    const newPassword = $("new-password").value;
    if (newPassword !== $("confirm-new-password").value) {
      $("password-error").textContent = "两次输入的新密码不一致。";
      return;
    }
    const button = $("confirm-password");
    button.disabled = true;
    button.textContent = "正在保存…";
    $("password-error").textContent = "";
    try {
      await request("/admin/api/change-password", {
        method: "POST",
        body: JSON.stringify({ currentPassword, newPassword, totpCode: $("change-totp").value }),
      });
      showLogin("管理密码已修改，请使用新密码重新登录。旧登录会话已退出。 ");
    } catch (error) {
      if (error.status === 401) {
        $("password-error").textContent = error.message;
        $("change-totp").value = "";
        $("change-totp").focus();
      } else if (error.status === 403) {
        $("password-dialog").close();
        showLogin("登录状态已失效，请重新登录后再修改密码。 ");
      } else {
        $("password-error").textContent = error.message;
      }
    } finally {
      button.disabled = false;
      button.textContent = "保存新密码";
    }
  });

  $("refresh-button").addEventListener("click", () => loadLicenses());
  $("model-catalog-run-button").addEventListener("click", async () => {
    if (!window.confirm("这会刷新所有符合条件的 404token 账号上游目录、按已批准规则更新映射，并对官方证据不足的已映射文本模型调用原生账号测试接口发起真实生成请求。请求可能消耗上游额度，成功后也可能恢复账号状态；账号 model_mapping 改写、模型名不完全匹配或结果不完整时会忽略该模型。确定开始吗？")) return;
    const button = $("model-catalog-run-button");
    button.disabled = true;
    try {
      const data = await request("/admin/api/model-catalog/runs", { method: "POST" });
      renderModelCatalog(data);
      if (data.status === "running") await loadModelCatalog();
    } catch (error) {
      if (error.status === 401) return showLogin("登录已过期，请重新登录。");
      renderModelCatalogUnavailable(error.message);
    }
  });
  $("registration-file").addEventListener("change", (event) => importRegistration(event.target.files[0]));
  $("previous-page").addEventListener("click", () => { state.offset = Math.max(0, state.offset - state.limit); loadLicenses(); });
  $("next-page").addEventListener("click", () => { state.offset += state.limit; loadLicenses(); });
  $("revoke-confirm-check").addEventListener("change", updateRevokeButton);
  $("revoke-confirm-id").addEventListener("input", updateRevokeButton);
  $("revoke-reason").addEventListener("input", updateRevokeButton);
  $("cancel-revoke").addEventListener("click", () => $("revoke-dialog").close());
  $("revoke-form").addEventListener("submit", async (event) => {
    event.preventDefault();
    const item = state.selected;
    if (!item || $("confirm-revoke").disabled) return;
    const button = $("confirm-revoke");
    button.disabled = true;
    button.textContent = "正在作废…";
    try {
      await request(`/admin/api/licenses/${encodeURIComponent(item.installationId)}/revoke`, {
        method: "POST",
        body: JSON.stringify({ reason: $("revoke-reason").value.trim(), confirmInstallationId: item.installationId }),
      });
      $("revoke-dialog").close();
      await loadLicenses();
      setMessage("作废状态已保存到 VPS。U 盘需联网并经管理器启动后同步。", "success");
    } catch (error) {
      if (error.status === 401) {
        $("revoke-dialog").close();
        return showLogin("登录已过期，请重新登录。");
      }
      setMessage(error.message, "error");
    } finally {
      button.textContent = "确认作废";
      updateRevokeButton();
    }
  });

  (async () => {
    try {
      const data = await request("/admin/api/session");
      state.csrf = data.csrfToken;
      if (data.setupRequired) {
        await showFirstSetup(data.username);
        return;
      }
      showApp(data.username);
      await loadLicenses();
      await loadModelCatalog();
    } catch (_) {
      showLogin();
    }
  })();
})();
