import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

const check = vi.fn();
const relaunch = vi.fn();
vi.mock("@tauri-apps/plugin-updater", () => ({ check: (...a: unknown[]) => check(...a) }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: (...a: unknown[]) => relaunch(...a) }));

import { checkForUpdate, installAndRelaunch } from "./updater";

beforeEach(() => {
  check.mockReset();
  relaunch.mockReset();
  relaunch.mockResolvedValue(undefined);
});

afterEach(() => {
  vi.unstubAllEnvs();
  vi.restoreAllMocks();
});

describe("checkForUpdate", () => {
  it("skips the check in dev mode", async () => {
    // default vitest env: import.meta.env.DEV is true
    const result = await checkForUpdate();
    expect(result).toBeNull();
    expect(check).not.toHaveBeenCalled();
  });

  it("returns null when up to date", async () => {
    vi.stubEnv("DEV", false);
    check.mockResolvedValue(null);
    expect(await checkForUpdate()).toBeNull();
    expect(check).toHaveBeenCalledOnce();
  });

  it("returns the update when available", async () => {
    vi.stubEnv("DEV", false);
    const fake = { version: "0.2.0" };
    check.mockResolvedValue(fake);
    expect(await checkForUpdate()).toBe(fake);
  });

  it("swallows check failures", async () => {
    vi.stubEnv("DEV", false);
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    check.mockRejectedValue(new Error("offline"));
    expect(await checkForUpdate()).toBeNull();
    expect(warn).toHaveBeenCalled();
  });
});

describe("installAndRelaunch", () => {
  it("orders download before relaunch", async () => {
    const order: string[] = [];
    const downloadAndInstall = vi.fn().mockImplementation(async () => {
      order.push("download");
    });
    relaunch.mockImplementation(async () => {
      order.push("relaunch");
    });
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    await installAndRelaunch({ downloadAndInstall } as any);
    expect(order).toEqual(["download", "relaunch"]);
  });

  it("does not relaunch when download fails", async () => {
    const downloadAndInstall = vi.fn().mockRejectedValue(new Error("boom"));
    await expect(
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      installAndRelaunch({ downloadAndInstall } as any),
    ).rejects.toThrow("boom");
    expect(relaunch).not.toHaveBeenCalled();
  });
});
