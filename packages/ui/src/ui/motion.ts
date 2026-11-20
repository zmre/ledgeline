// Motion preferences shared by the package's helpers and the apps' own
// transitions (Ledgeline's review overlay re-exports this from
// web/src/lib/review/ui/motion.ts).

/** The user's `prefers-reduced-motion`, read when asked (it can change mid-session). */
export function prefersReducedMotion(): boolean {
    return typeof window !== "undefined" && typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}
