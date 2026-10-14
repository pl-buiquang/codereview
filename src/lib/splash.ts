const MIN_VISIBLE_MS = 650;
const FADE_FALLBACK_MS = 400;

let dismissed = false;

export function dismissSplash(): void {
  if (dismissed) return;
  dismissed = true;

  const el = document.getElementById("splash");
  if (!el) return;

  const elapsed = performance.now();
  const wait = Math.max(0, MIN_VISIBLE_MS - elapsed);

  window.setTimeout(() => {
    console.info(`[startup] app ready in ${Math.round(performance.now())}ms`);
    el.addEventListener("transitionend", () => el.remove(), { once: true });
    window.setTimeout(() => el.remove(), FADE_FALLBACK_MS);
    el.classList.add("splash-out");
  }, wait);
}
