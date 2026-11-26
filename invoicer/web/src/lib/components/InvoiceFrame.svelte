<!-- An invoice as its template renders it, in a frame, always by URL (both
     need the session cookie: invoicer PLAN A2):

     - `page` mode: MBR's own page for the saved file;
     - `preview` mode: the saved file through another template
       (`/.ledgeline-invoicer/preview`, under MBR's CSP).

     Never `srcdoc`: an about:srcdoc document inherits this window's CSP,
     whose `frame-ancestors 'none'` WebKit applies to it, so the window showed
     a blank frame (2026-11-23).

     Sandboxed with `allow-same-origin` and nothing else: no script in the
     page runs, so a template can never reach this window (or its token),
     while the page still loads MBR's assets with the session cookie and stays
     readable for sizing. A template needs no script to show an invoice; MBR's
     export-ready partial only matters inside a PDF export, which has its own
     webview. The frame grows to its page's height once it loads.

     Links move only the frame, never the app. In the desktop window an
     outside https or mailto link opens in the default browser or mail app and
     the frame stays put; in `--server` mode, in a plain browser, it loads in
     the sandboxed frame. -->
<script lang="ts">
    let {src, title, mode = "page"}: {src?: string; title: string; mode?: "page" | "preview"} = $props();

    let frame = $state<HTMLIFrameElement | null>(null);
    let height = $state(1100);

    function fit(): void {
        const doc = frame?.contentDocument;
        const measured = doc?.documentElement?.scrollHeight ?? 0;
        if (measured > 0) height = measured;
    }
</script>

{#if src !== undefined}
    <iframe
        bind:this={frame}
        {title}
        {src}
        sandbox="allow-same-origin"
        class="w-full rounded-box border border-base-300 bg-white"
        style="height: {height}px"
        onload={fit}
        data-testid="invoice-frame"
        data-mode={mode}
    ></iframe>
{/if}
