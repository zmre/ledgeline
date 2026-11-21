// What the empty shell needs before the invoicer has an API (invoicer PLAN 0d).
// Phase 3 replaces the default appearance with the saved prefs, read from the
// same ui-prefs.toml and ledgeline.toml [ui] pin Ledgeline uses.
import {DEFAULT_THEME_PREF} from "#ui/theme/catalog.js";
import {DEFAULT_SCALE} from "#ui/theme/scale.js";
import type {AppearancePrefs} from "#ui/theme/appearance.svelte.js";

export const APP_NAME = "Ledgeline Invoicer";

/**
 * The appearance until prefs load. `uiPrefsReady` stays false, so the shell
 * never overwrites the first-paint cache with these defaults.
 */
export const defaultAppearance = (): AppearancePrefs => ({theme: DEFAULT_THEME_PREF, uiScale: DEFAULT_SCALE, uiPrefsReady: false});
