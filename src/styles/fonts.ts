/*
 * Bundled webfonts for the three design directions. Static @fontsource weights
 * (not the `-variable` packages) so the family names match the tokens.css stacks
 * exactly ("Atkinson Hyperlegible", "Atkinson Hyperlegible Mono", "Inter",
 * "JetBrains Mono"). Vite bundles the woff2 locally — the app is fully offline.
 *
 * Chosen for on-screen legibility (esp. the diff/review pane):
 * A · Continuity — Atkinson Hyperlegible (UI) + Atkinson Hyperlegible Mono (code/display)
 * B · Modern     — Inter (UI/display) + JetBrains Mono (code)
 * C · Terminal   — JetBrains Mono everywhere
 *
 * Atkinson Hyperlegible (sans) ships only 400/700; CSS weights 500/600 resolve to
 * the nearest real face (no faux synthesis). The mono families carry 400–700.
 */

// Atkinson Hyperlegible 400/700 (only weights published)
import "@fontsource/atkinson-hyperlegible/400.css";
import "@fontsource/atkinson-hyperlegible/700.css";

// Atkinson Hyperlegible Mono 400/500/600/700
import "@fontsource/atkinson-hyperlegible-mono/400.css";
import "@fontsource/atkinson-hyperlegible-mono/500.css";
import "@fontsource/atkinson-hyperlegible-mono/600.css";
import "@fontsource/atkinson-hyperlegible-mono/700.css";

// Inter 400/500/600/700
import "@fontsource/inter/400.css";
import "@fontsource/inter/500.css";
import "@fontsource/inter/600.css";
import "@fontsource/inter/700.css";

// JetBrains Mono 400/500/600/700
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/500.css";
import "@fontsource/jetbrains-mono/600.css";
import "@fontsource/jetbrains-mono/700.css";
