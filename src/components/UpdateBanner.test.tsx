import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const checkForUpdate = vi.fn();
const installAndRelaunch = vi.fn();
vi.mock("../lib/updater", () => ({
  checkForUpdate: (...a: unknown[]) => checkForUpdate(...a),
  installAndRelaunch: (...a: unknown[]) => installAndRelaunch(...a),
}));

import { UpdateBanner } from "./UpdateBanner";
import { useToastStore } from "../lib/toast";

const fakeUpdate = { version: "0.2.0" } as never;

beforeEach(() => {
  checkForUpdate.mockReset();
  installAndRelaunch.mockReset();
  installAndRelaunch.mockResolvedValue(undefined);
  useToastStore.setState({ toasts: [] });
});

describe("UpdateBanner", () => {
  it("renders nothing when no update", async () => {
    checkForUpdate.mockResolvedValue(null);
    const { container } = render(<UpdateBanner />);
    // give the effect a chance to resolve
    await waitFor(() => expect(checkForUpdate).toHaveBeenCalled());
    expect(container).toBeEmptyDOMElement();
  });

  it("shows version and actions when update available", async () => {
    checkForUpdate.mockResolvedValue(fakeUpdate);
    render(<UpdateBanner />);
    expect(await screen.findByText(/Update available: v0\.2\.0/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Install & relaunch" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Dismiss" })).toBeInTheDocument();
  });

  it("install button installs and disables", async () => {
    checkForUpdate.mockResolvedValue(fakeUpdate);
    let resolveInstall!: () => void;
    installAndRelaunch.mockImplementation(
      () => new Promise<void>((res) => (resolveInstall = res)),
    );
    render(<UpdateBanner />);
    const btn = await screen.findByRole("button", { name: "Install & relaunch" });
    await userEvent.click(btn);
    expect(installAndRelaunch).toHaveBeenCalledWith(fakeUpdate);
    const installing = await screen.findByRole("button", { name: "Installing…" });
    expect(installing).toBeDisabled();
    resolveInstall();
  });

  it("install failure toasts and re-enables", async () => {
    checkForUpdate.mockResolvedValue(fakeUpdate);
    installAndRelaunch.mockRejectedValue(new Error("nope"));
    render(<UpdateBanner />);
    const btn = await screen.findByRole("button", { name: "Install & relaunch" });
    await userEvent.click(btn);
    await waitFor(() =>
      expect(useToastStore.getState().toasts.some((t) => t.kind === "error")).toBe(true),
    );
    expect(screen.getByRole("button", { name: "Install & relaunch" })).not.toBeDisabled();
  });

  it("dismiss hides the banner", async () => {
    checkForUpdate.mockResolvedValue(fakeUpdate);
    render(<UpdateBanner />);
    const dismiss = await screen.findByRole("button", { name: "Dismiss" });
    await userEvent.click(dismiss);
    expect(screen.queryByText(/Update available/)).not.toBeInTheDocument();
    expect(installAndRelaunch).not.toHaveBeenCalled();
  });
});
