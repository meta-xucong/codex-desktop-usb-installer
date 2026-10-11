(() => {
  const $ = (id) => document.getElementById(id);
  const state = { csrf: "", offset: 0, limit: 50, total: 0, selected: null, items: [] };

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
    } catch (_) {
      showLogin();
    }
  })();
})();
