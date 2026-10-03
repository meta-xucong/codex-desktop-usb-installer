import { useCallback, useEffect, useRef, useState } from "react";

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

export type AccountAuthorizationState = "checking" | "authorized" | "unavailable";

export function AccountCard({
  authorizationState,
}: {
  authorizationState: AccountAuthorizationState;
}) {
  const [loadingState, setLoadingState] = useState<LoadingState>("loading");
  const [status, setStatus] = useState<AccountStatus | null>(null);
  const [password, setPassword] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState<"login" | "refresh" | "redeem" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const statusRequestGeneration = useRef(0);

  const canUseAccount = authorizationState === "authorized" && status?.bound === true;

  const refreshStatus = useCallback(async () => {
    const generation = ++statusRequestGeneration.current;
    setLoadingState("loading");
    setError(null);
    try {
      const next = await managerApi.accountStatus();
      if (generation !== statusRequestGeneration.current) return;
      setStatus(next);
      setLoadingState("ready");
    } catch (cause) {
      if (generation !== statusRequestGeneration.current) return;
      setStatus(null);
      setError(errorMessage(cause));
      setLoadingState("error");
    }
  }, []);

  useEffect(() => {
    if (authorizationState === "authorized") {
      void refreshStatus();
    } else {
      // Invalidate an account read that began before a drive removal or guard
      // transition. The card remains rendered; only credential actions pause.
      statusRequestGeneration.current += 1;
      setStatus(null);
      setBusy(null);
      setError(null);
      setLoadingState(authorizationState === "checking" ? "loading" : "error");
    }
    return () => {
      statusRequestGeneration.current += 1;
    };
  }, [authorizationState, refreshStatus]);

  const run = async (action: "login" | "refresh" | "redeem") => {
    if (!canUseAccount) {
      setError("U 盘账户授权尚未就绪，请重新检查 U 盘连接和授权状态");
      return;
    }
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
      const failure = errorMessage(cause);
      if (action !== "login") {
        // A locally expired token or an upstream 401 clears the in-memory
        // session in the backend. Reconcile that state so the login form is
        // shown again instead of leaving a stale logged-in recharge card.
        try {
          const current = await managerApi.accountStatus();
          if (!current.bound || !current.loggedIn) {
            setStatus({
              ...current,
              message: current.bound
                ? "404token 登录会话已过期，请重新登录。"
                : current.message,
            });
            setError(null);
            return;
          }
        } catch {
          // Preserve the original request failure if local status cannot be
          // read; the user can still retry or re-check account authorization.
        }
      }
      setError(failure);
    } finally {
      setBusy(null);
    }
  };

  const startRedeem = () => {
    if (!canUseAccount) {
      setError("U 盘账户授权尚未就绪，请重新检查 U 盘连接和授权状态");
      return;
    }
    if (!status?.loggedIn) {
      setError("首次充值请先登录 404token 账号；密码只在本次打开管理器期间使用");
      return;
    }
    void run("redeem");
  };

  const usageDisplay = displayUsage(status?.usage);
  const accountMessage = error
    ? `账户信息暂不可用：${error}`
    : authorizationState === "checking"
      ? "正在校验 U 盘账户授权；授权就绪后会自动加载账户信息。"
      : authorizationState === "unavailable"
        ? "U 盘账户授权暂不可用。请重新连接已签名的 U 盘，或点击重新检查。"
        : loadingState === "loading"
          ? "正在加载 404token 账户信息..."
          : status?.message ?? "此 U 盘尚未绑定 404token 账号";
  const accountMessageIsError = Boolean(
    error || authorizationState === "unavailable" || status?.bound === false,
  );

  return (
    <section className="account-panel" aria-label="404token 账户">
      <div className="account-panel-head">
        <span><Icon name="shield" /> {status?.accountName ?? "404token 账户"}</span>
        {status?.loggedIn && status.balance !== null ? (
          <span className="account-balance">余额 {displayBalance(status.balance)}</span>
        ) : null}
      </div>
      {!status?.loggedIn ? (
        <div className="account-panel-row account-login-row">
          <input
            className="text-input account-input"
            type="password"
            value={password}
            placeholder="输入 404token 密码"
            onChange={(event) => setPassword(event.target.value)}
            autoComplete="new-password"
            disabled={!canUseAccount || busy !== null}
            onKeyDown={(event) => {
              if (event.key === "Enter" && password.trim() && busy === null && canUseAccount) {
                void run("login");
              }
            }}
          />
          <button className="btn ghost" onClick={() => void run("login")} disabled={!canUseAccount || busy !== null || !password.trim()}>
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
          disabled={!canUseAccount || busy !== null}
          onKeyDown={(event) => {
            if (event.key === "Enter" && code.trim() && busy === null && canUseAccount) {
              startRedeem();
            }
          }}
        />
        <button className="btn primary" onClick={startRedeem} disabled={!canUseAccount || busy !== null || !code.trim()}>
          {busy === "redeem" ? <Icon name="loader" className="spinicon" /> : null} 一键充值
        </button>
        {status?.loggedIn ? (
          <button className="btn ghost icon-only" onClick={() => void run("refresh")} disabled={busy !== null} aria-label="刷新余额">
            {busy === "refresh" ? <Icon name="loader" className="spinicon" /> : <Icon name="refresh" />}
          </button>
        ) : null}
      </div>
      {status?.loggedIn && usageDisplay ? (
        <div className="account-panel-usage">{usageDisplay}</div>
      ) : null}
      <div className={accountMessageIsError ? "account-panel-error" : "account-panel-message"}>
        {accountMessage}
      </div>
      {authorizationState !== "checking" && loadingState !== "loading" && (!canUseAccount || loadingState === "error") ? (
        <button
          className="btn ghost"
          onClick={() => void refreshStatus()}
          disabled={busy !== null}
        >
          <Icon name="refresh" />
          重新检查账户
        </button>
      ) : null}
    </section>
  );
}
