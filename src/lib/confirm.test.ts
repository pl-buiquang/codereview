import { describe, it, expect, beforeEach } from "vitest";
import { useConfirmStore, confirmDialog } from "./confirm";

beforeEach(() => useConfirmStore.setState({ request: null }));

describe("confirmDialog", () => {
  it("opens a request carrying the provided options", () => {
    confirmDialog({ title: "Remove", message: "sure?", danger: true });
    const req = useConfirmStore.getState().request;
    expect(req?.title).toBe("Remove");
    expect(req?.message).toBe("sure?");
    expect(req?.danger).toBe(true);
  });

  it("resolves true and closes when confirmed", async () => {
    const p = confirmDialog({ title: "T" });
    useConfirmStore.getState().settle(true);
    await expect(p).resolves.toBe(true);
    expect(useConfirmStore.getState().request).toBeNull();
  });

  it("resolves false when cancelled", async () => {
    const p = confirmDialog({ title: "T" });
    useConfirmStore.getState().settle(false);
    await expect(p).resolves.toBe(false);
  });

  it("opening a second dialog cancels the first", async () => {
    const first = confirmDialog({ title: "first" });
    const second = confirmDialog({ title: "second" });
    await expect(first).resolves.toBe(false);
    expect(useConfirmStore.getState().request?.title).toBe("second");
    useConfirmStore.getState().settle(true);
    await expect(second).resolves.toBe(true);
  });
});
