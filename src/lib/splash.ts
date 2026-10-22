const HOLD_MS = 150;
const FADE_FALLBACK_MS = 400;

let dismissed = false;

/** Resolves once the splash's bar-draw animations finished (immediately when
 *  reduced motion disables them), so the fade never cuts the icon mid-draw. */
function drawn(el: HTMLElement): Promise<unknown> {
  const draws = el
    .getAnimations({ subtree: true })
    .filter((a) => a instanceof CSSAnimation && a.animationName === "splash-draw");
  return Promise.all(draws.map((a) => a.finished.catch(() => undefined)));
}

export function dismissSplash(): void {
  if (dismissed) return;
  dismissed = true;

  const el = document.getElementById("splash");
  if (!el) return;

  drawn(el).then(() => {
    window.setTimeout(() => {
      console.info(`[startup] app ready in ${Math.round(performance.now())}ms`);
      el.addEventListener("transitionend", () => el.remove(), { once: true });
      window.setTimeout(() => el.remove(), FADE_FALLBACK_MS);
      el.classList.add("splash-out");
    }, HOLD_MS);
  });
}
