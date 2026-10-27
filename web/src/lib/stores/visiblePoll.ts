// "Check again every few seconds while someone can see the answer, and at once
// when they come back to it."
//
// The one shape every "did the disk change behind our back?" check here takes.
// It began as the journal's live-update loop (WP-08, `startPolling`) and was
// lifted out when the Projections tab needed the same thing for its open
// scenario file, which the engine's journal watcher does not watch (a scenario
// is by definition a file the main journal does not include).
//
// Two return signals, because they mean different things in the desktop app:
//
//  - `visibilitychange` → visible: the window was minimized or the tab hidden.
//  - `focus`: the window was VISIBLE all along, but the user was typing in
//    another program — their editor, which is exactly when a file changes. A
//    visible-but-unfocused window never fires `visibilitychange`.
//
// Polling stops while hidden; `run` is expected to be cheap and to dedup its
// own concurrent calls.

/** Call `run` every `intervalMs` while the document is visible, and immediately on return. Returns a stop function. */
export function pollWhileVisible(run: () => void, intervalMs: number): () => void {
    let timer: ReturnType<typeof setInterval> | null = null;
    const start = (): void => {
        timer ??= setInterval(run, intervalMs);
    };
    const stop = (): void => {
        if (timer !== null) {
            clearInterval(timer);
            timer = null;
        }
    };
    const onVisibilityChange = (): void => {
        if (document.visibilityState === "hidden") {
            stop();
        } else {
            run();
            start();
        }
    };
    const onFocus = (): void => run();
    document.addEventListener("visibilitychange", onVisibilityChange);
    window.addEventListener("focus", onFocus);
    start();
    return () => {
        stop();
        document.removeEventListener("visibilitychange", onVisibilityChange);
        window.removeEventListener("focus", onFocus);
    };
}
