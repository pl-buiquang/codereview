/*
 * Bundled webfonts for the three design directions. Static @fontsource weights
 * (not the `-variable` packages) so the family names match the tokens.css stacks
 * exactly ("Manrope", "IBM Plex Sans", "IBM Plex Mono", "JetBrains Mono"). Vite
 * bundles the woff2 locally — the app is fully offline.
 *
 * A · Continuity — IBM Plex Sans (UI) + IBM Plex Mono (display/mono)
 * B · Modern     — Manrope (UI/display) + JetBrains Mono (mono)
 * C · Terminal   — JetBrains Mono everywhere
 */

// IBM Plex Sans 400/500/600/700
import "@fontsource/ibm-plex-sans/400.css";
import "@fontsource/ibm-plex-sans/500.css";
import "@fontsource/ibm-plex-sans/600.css";
import "@fontsource/ibm-plex-sans/700.css";

// IBM Plex Mono 400/500/600/700
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/500.css";
import "@fontsource/ibm-plex-mono/600.css";
import "@fontsource/ibm-plex-mono/700.css";

// Manrope 400/500/600/700/800
import "@fontsource/manrope/400.css";
import "@fontsource/manrope/500.css";
import "@fontsource/manrope/600.css";
import "@fontsource/manrope/700.css";
import "@fontsource/manrope/800.css";

// JetBrains Mono 400/500/600/700
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/500.css";
import "@fontsource/jetbrains-mono/600.css";
import "@fontsource/jetbrains-mono/700.css";
