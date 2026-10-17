import { invoke as rawInvoke, type InvokeArgs, type InvokeOptions } from "@tauri-apps/api/core";
import { brief, log, logEnabled } from "./log";

/** Commands that must not trace themselves: the Settings log viewer polls them. */
const UNTRACED = new Set(["read_app_log", "log_path", "frontend_log", "get_log_level"]);

/** Drop-in for `@tauri-apps/api/core::invoke` that logs every backend call:
 *  command, args, duration and outcome (debug), failures at warn. */
export async function invoke<T>(cmd: string, args?: InvokeArgs, options?: InvokeOptions): Promise<T> {
  const call = () => (options ? rawInvoke<T>(cmd, args, options) : args === undefined ? rawInvoke<T>(cmd) : rawInvoke<T>(cmd, args));
  if (UNTRACED.has(cmd) || !logEnabled("warn")) return call();
  const trace = logEnabled("debug");
  const started = performance.now();
  if (trace) log.debug("invoke", `→ ${cmd} ${args === undefined ? "" : brief(args)}`);
  try {
    const result = await call();
    if (trace) log.debug("invoke", `← ${cmd} ok in ${Math.round(performance.now() - started)}ms`);
    return result;
  } catch (e) {
    log.warn("invoke", `← ${cmd} FAILED in ${Math.round(performance.now() - started)}ms: ${brief(e)}`);
    throw e;
  }
}
