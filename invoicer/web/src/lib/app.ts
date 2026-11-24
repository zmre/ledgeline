// The shell's constants (invoicer PLAN 0d). The layout replaces the default
// appearance with the saved prefs from `GET /api/ui-prefs` (PLAN 3a): the
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
