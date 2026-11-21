// The `theme` pref → the variant `<html data-theme>` shows. Pure: the caller
// supplies the OS scheme (`matchMedia("(prefers-color-scheme: dark)")`).

import {DEFAULT_THEME_PREF, FALLBACK_VARIANT, isVariantId, themeFamily, type VariantId} from "./catalog";

/**
 * The variant for `pref` under the OS's current scheme.
 *
 * - a variant id is pinned: it is returned as is, whatever the OS says;
 * - a family id with both variants follows the OS: its dark variant when
 *   `prefersDark`, else its light one;
 * - a dark-only family (`nord`, …) has one variant and always shows it;
 * - anything else (unknown, empty, a stale id) resolves as `default`.
 */
export function resolveTheme(pref: string, prefersDark: boolean): VariantId {
    if (isVariantId(pref)) return pref;
    const family = themeFamily(pref) ?? themeFamily(DEFAULT_THEME_PREF);
    const {light, dark} = family?.variants ?? {};
    return (prefersDark ? (dark ?? light) : (light ?? dark)) ?? FALLBACK_VARIANT;
}
