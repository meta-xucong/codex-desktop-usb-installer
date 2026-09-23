import { useEffect, useState } from "react";

import { errorMessage, managerApi } from "../../services/managerApi";
import type { AccountStatus } from "../../shared/types";
import { Icon } from "../icons";

function displayBalance(value: unknown): string {
  if (value === null || value === undefined) return "—";
  if (typeof value === "string" || typeof value === "number") return String(value);
  if (typeof value === "object" && value && "amount" in value) {
    const amount = (value as { amount?: unknown }).amount;
    return amount === undefined ? "—" : String(amount);
  }
  return "已更新";
}

function displayUsage(value: unknown): string | null {
  if (!value || typeof value !== "object") return null;
  const obj = value as Record<string, unknown>;

  // Try common field names for today/monthly usage
  const today = obj.today ?? obj.daily ?? obj.todayUsage;
  const month = obj.thisMonth ?? obj.monthly ?? obj.monthlyUsage ?? obj.this_month;

  if (today !== undefined && month !== undefined) {
    return `今日 ${today} / 本月 ${month}`;
  }
  if (month !== undefined) {
    return `本月 ${month}`;
  }
  if (today !== undefined) {
    return `今日 ${today}`;
  }
  return null;
}

type LoadingState = "idle" | "loading" | "ready" | "error";

export function AccountCard() {
  const [loadingState, setLoadingState] = useState<LoadingState>("loading");
  const [status, setStatus] = useState<AccountStatus | null>(null);
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState<"login" | "refresh" | "redeem" | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setLoadingState("loading");
    const read = managerApi.accountStatus?.() ?? Promise.resolve({
      bound: false,
      accountName: null,
      loggedIn: false,
      balance: null,
      usage: null,
      message: "账户功能不可用",
    });
    void read
      .then((s) => {
        setStatus(s);
        setLoadingState("ready");
      })
      .catch((cause) => {
        setError(errorMessage(cause));
        setLoadingState("error");
      });
  }, []);

  const run = async (action: "login" | "refresh" | "redeem") => {
    setBusy(action);
    setError(null);
    try {
      const next = action === "login"
        ? await managerApi.accountLogin(password)
        : action === "redeem"
          ? await managerApi.accountRedeem(code)
          : await managerApi.accountRefresh();
      setStatus(next);
      if (action === "login") setPassword("");
      if (action === "redeem") setCode("");
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  };

  const startRedeem = () => {
    if (!status?.loggedIn) {
      setError("首次充值请先登录 404token 账号；密码只在本次打开管理器期间使用");
      return;
    }
    void run("redeem");
  };

  // Show loading state
  if (loadingState === "loading") {
    return (
      <section className="account-panel" aria-label="404token 账户">
        <div className="account-panel-message">
          <Icon name="loader" className="spinicon" /> 正在加载账户信息...
        </div>
      </section>
    );
  }

  // Show error state
  if (loadingState === "error") {
    return (
      <section className="account-panel" aria-label="404token 账户">
        <div className="account-panel-error">账户功能加载失败：{error}</div>
      </section>
    );
  }

  // Don't show card if not bound
  if (!status?.bound) return null;

  const usageDisplay = displayUsage(status.usage);

  return (
    <section className="account-panel" aria-label="404token 账户">
      <div className="account-panel-head">
        <span><Icon name="shield" /> {status.accountName ?? "已绑定账号"}</span>
        {status.loggedIn && status.balance !== null ? (
          <span className="account-balance">余额 {displayBalance(status.balance)}</span>
        ) : null}
      </div>
      {!status.loggedIn ? (
        <div className="account-panel-row account-login-row">
          <input
            className="text-input account-input"
            type="password"
            value={password}
            placeholder="输入 404token 密码"
            onChange={(event) => setPassword(event.target.value)}
            autoComplete="new-password"
            onKeyDown={(event) => {
              if (event.key === "Enter" && password.trim() && busy === null) {
                void run("login");
              }
            }}
          />
          <button className="btn ghost" onClick={() => void run("login")} disabled={busy !== null || !password.trim()}>
            {busy === "login" ? <Icon name="loader" className="spinicon" /> : null} 登录账户
          </button>
        </div>
      ) : null}
      <div className="account-panel-row">
        <input
          className="text-input account-input"
          value={code}
          placeholder="输入充值码"
          onChange={(event) => setCode(event.target.value)}
          autoComplete="off"
          onKeyDown={(event) => {
            if (event.key === "Enter" && code.trim() && busy === null) {
              startRedeem();
            }
          }}
        />
        <button className="btn primary" onClick={startRedeem} disabled={busy !== null || !code.trim()}>
          {busy === "redeem" ? <Icon name="loader" className="spinicon" /> : null} 一键充值
        </button>
        {status.loggedIn ? (
          <button className="btn ghost icon-only" onClick={() => void run("refresh")} disabled={busy !== null} aria-label="刷新余额">
            {busy === "refresh" ? <Icon name="loader" className="spinicon" /> : <Icon name="refresh" />}
          </button>
        ) : null}
      </div>
      {status.loggedIn && usageDisplay ? (
        <div className="account-panel-usage">{usageDisplay}</div>
      ) : null}
      {error ? <div className="account-panel-error">{error}</div> : <div className="account-panel-message">{status.message}</div>}
    </section>
  );
}
