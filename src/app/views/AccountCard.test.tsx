import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { AccountCard } from "./AccountCard";

describe("AccountCard startup state", () => {
  it("keeps the account controls visible and still while USB authorization is loading", () => {
    render(<AccountCard authorizationState="checking" />);

    expect(screen.getByRole("region", { name: "404token 账户" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "登录账户" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "一键充值" })).toBeDisabled();
    expect(screen.getByText("正在校验 U 盘账户授权；授权就绪后会自动加载账户信息。")).toBeInTheDocument();
    expect(document.querySelector(".account-panel .spinicon")).toBeNull();
  });

  it("keeps the login and recharge rows as separate layout rows", () => {
    const { container } = render(<AccountCard authorizationState="checking" />);
    const rows = container.querySelectorAll(".account-panel-row");

    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveClass("account-login-row");
    expect(rows[1]).not.toHaveClass("account-login-row");
  });
});
