import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook } from "@testing-library/react";
import { useDebouncedCallback } from "./useDebouncedCallback";

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("useDebouncedCallback", () => {
  it("invokes the callback only after the delay elapses", () => {
    const cb = vi.fn();
    const { result } = renderHook(() => useDebouncedCallback(cb, 200));

    result.current("a");
    expect(cb).not.toHaveBeenCalled();
    vi.advanceTimersByTime(199);
    expect(cb).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(cb).toHaveBeenCalledTimes(1);
    expect(cb).toHaveBeenCalledWith("a");
  });

  it("collapses rapid calls into a single trailing invocation", () => {
    const cb = vi.fn();
    const { result } = renderHook(() => useDebouncedCallback(cb, 100));

    result.current("first");
    vi.advanceTimersByTime(50);
    result.current("second");
    vi.advanceTimersByTime(50);
    result.current("third");
    vi.advanceTimersByTime(100);

    expect(cb).toHaveBeenCalledTimes(1);
    expect(cb).toHaveBeenCalledWith("third");
  });

  it("always uses the latest callback reference", () => {
    const first = vi.fn();
    const second = vi.fn();
    const { result, rerender } = renderHook(
      ({ cb }) => useDebouncedCallback(cb, 100),
      { initialProps: { cb: first } },
    );

    result.current("x");
    rerender({ cb: second });
    vi.advanceTimersByTime(100);

    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledWith("x");
  });

  it("cancels a pending call on unmount", () => {
    const cb = vi.fn();
    const { result, unmount } = renderHook(() => useDebouncedCallback(cb, 100));

    result.current("x");
    unmount();
    vi.advanceTimersByTime(200);
    expect(cb).not.toHaveBeenCalled();
  });
});
