import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { useToastStore, toast } from "./toast";

beforeEach(() => {
  useToastStore.setState({ toasts: [] });
  vi.useFakeTimers();
});
afterEach(() => vi.useRealTimers());

describe("toast", () => {
  it("pushes error and success toasts in order", () => {
    toast.error("boom");
    toast.success("yay");
    const ts = useToastStore.getState().toasts;
    expect(ts.map((t) => [t.kind, t.message])).toEqual([
      ["error", "boom"],
      ["success", "yay"],
    ]);
  });

  it("auto-dismisses a success toast after its ttl", () => {
    toast.success("hi");
    expect(useToastStore.getState().toasts).toHaveLength(1);
    vi.advanceTimersByTime(4000);
    expect(useToastStore.getState().toasts).toHaveLength(0);
  });

  it("keeps error toasts longer than success toasts", () => {
    toast.error("err");
    vi.advanceTimersByTime(4000);
    expect(useToastStore.getState().toasts).toHaveLength(1);
    vi.advanceTimersByTime(4000);
    expect(useToastStore.getState().toasts).toHaveLength(0);
  });

  it("dismiss removes a specific toast immediately", () => {
    const id = useToastStore.getState().push("error", "x");
    useToastStore.getState().push("success", "y");
    useToastStore.getState().dismiss(id);
    const ts = useToastStore.getState().toasts;
    expect(ts).toHaveLength(1);
    expect(ts[0].message).toBe("y");
  });
});
