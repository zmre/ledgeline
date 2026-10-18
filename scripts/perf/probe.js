// Launch + scroll probe for the desktop app (docs/development/perf-baseline.md).
//
// Injected by the GUI when LEDGELINE_PERF names this file (see `mod perf` in
// crates/ledgeline-server/src/gui.rs). Reports milestones to Rust over
// window.ipc; Rust prints each as `PERF js <ms since launch> <json>` and exits
// on `scroll_done`. Run it through scripts/perf/run-probe.sh, which also samples
// memory while the probe holds the app open after the scroll.
(function () {
    if (window.top !== window) return;
    // The synthetic corpus ends 2025-12-31, so the default last-90-days view
    // would be empty: show the whole journal.
    if (!location.search.includes("preset=all")) history.replaceState(null, "", location.pathname + "?preset=all" + location.hash);
    const post = (o) => window.ipc.postMessage(JSON.stringify(o));
    post({ev: "script_init", t: performance.now()});

    // Every API request the page makes before its first rows are on screen:
    // what the launch is waiting on. (Wrapped at document start, before the
    // SPA's own scripts run, so nothing escapes it.)
    let launched = false;
    const realFetch = window.fetch.bind(window);
    window.fetch = (input, init) => {
        const started = performance.now();
        const path = String(input instanceof Request ? input.url : input).replace(/^https?:\/\/[^/]+/, "");
        return realFetch(input, init).then((response) => {
            if (!launched) post({ev: "fetch", path, t: started, ms: Math.round(performance.now() - started), status: response.status});
            return response;
        });
    };
    document.addEventListener("DOMContentLoaded", () => post({ev: "dom_content_loaded", t: performance.now()}));

    // REAL rows only: the windowed table draws skeleton rows (`data-skeleton`,
    // no `data-txn`) while a block loads, and those are not data on screen.
    const rowSel = "table tbody tr[data-txn]";
    const skeletonSel = "table tbody tr[data-skeleton]";
    const waitData = () => {
        if (document.querySelectorAll(rowSel).length > 0) {
            // Double rAF: the frame with rows has been committed.
            requestAnimationFrame(() =>
                requestAnimationFrame(() => {
                    launched = true;
                    post({ev: "data_on_screen", t: performance.now(), rows: document.querySelectorAll(rowSel).length});
                    setTimeout(scrollTest, 1500);
                })
            );
        } else {
            requestAnimationFrame(waitData);
        }
    };
    requestAnimationFrame(waitData);

    function pct(a, p) {
        const s = [...a].sort((x, y) => x - y);
        return s[Math.min(s.length - 1, Math.floor(p * s.length))];
    }
    function stats(a) {
        return {median: pct(a, 0.5), p95: pct(a, 0.95), max: Math.max(...a)};
    }

    function scrollTest() {
        const scroller = document.querySelector("table").closest(".overflow-y-auto");
        const FRAMES = 300;
        const STEP = 120; // ~ a hard fling: 120px/frame
        const intervals = [];
        const layout = [];
        let skeletonFrames = 0;
        let last = 0;
        let n = 0;
        const frame = (ts) => {
            if (last) intervals.push(ts - last);
            last = ts;
            const t = performance.now();
            void scroller.offsetHeight; // force pending style/layout from the last update
            layout.push(performance.now() - t);
            if (document.querySelectorAll(skeletonSel).length > 0) skeletonFrames += 1;
            if (n++ < FRAMES) {
                scroller.scrollTop += STEP;
                requestAnimationFrame(frame);
                return;
            }
            const vsync = pct(intervals, 0.5);
            post({
                ev: "scroll_stats",
                frames: intervals.length,
                rowsVisible: document.querySelectorAll(rowSel).length,
                scrollTop: scroller.scrollTop,
                vsync_ms: vsync,
                interval: stats(intervals),
                dropped: intervals.filter((x) => x > vsync * 1.5).length,
                layout_ms: stats(layout),
                // Frames on which some row was still a skeleton (its block in flight).
                skeleton_frames: skeletonFrames,
            });
            // Hold the app open so run-probe.sh can sample memory, then end.
            setTimeout(() => post({ev: "scroll_done"}), Number(window.__LEDGELINE_PERF_HOLD_MS__ ?? 8000));
        };
        requestAnimationFrame(frame);
    }
})();
