// The app is ~1 MB of JS and its first render is heavy: both block the main
// thread. Let the splash paint first so its animations are already running on
// the compositor instead of freezing half-drawn. The timeout covers a hidden
// window, where requestAnimationFrame never fires.
const splashPainted = new Promise<void>((resolve) => {
  requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
  window.setTimeout(resolve, 150);
});

splashPainted.then(() => import("./bootstrap"));
