import { useEffect, useMemo, useRef } from "react";

/** Returns a stable debounced wrapper around `cb`. Latest `cb` is always used. */
export function useDebouncedCallback<A extends unknown[]>(
  cb: (...args: A) => void,
  delay: number,
) {
  const cbRef = useRef(cb);
  useEffect(() => {
    cbRef.current = cb;
  });
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  useEffect(() => () => clearTimeout(timer.current), []);

  return useMemo(() => {
    return (...args: A) => {
      clearTimeout(timer.current);
      timer.current = setTimeout(() => cbRef.current(...args), delay);
    };
  }, [delay]);
}
