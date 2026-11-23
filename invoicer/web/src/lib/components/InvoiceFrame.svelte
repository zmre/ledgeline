<!-- An invoice as its template renders it, in a frame: MBR's own page for
     the saved file (`src`, which needs the session cookie: invoicer PLAN A2),
     or a preview's HTML (`html`, from `POST /api/preview`).

     Sandboxed with `allow-same-origin` and nothing else: no script in the
     page runs, so a template can never reach this window (or its token),
     while the page still loads MBR's assets with the session cookie and stays
     readable for sizing. A template needs no script to show an invoice; MBR's
     export-ready partial only matters inside a PDF export, which has its own
     webview. The frame grows to its page's height once it loads. -->
<script lang="ts">
    let {src, html, title}: {src?: string; html?: string; title: string} = $props();

    let frame = $state<HTMLIFrameElement | null>(null);
    let height = $state(1100);

    function fit(): void {
        const doc = frame?.contentDocument;
        const measured = doc?.documentElement?.scrollHeight ?? 0;
        if (measured > 0) height = measured;
    }
</script>

{#if html !== undefined}
    <iframe
        bind:this={frame}
        {title}
        srcdoc={html}
        sandbox="allow-same-origin"
        class="w-full rounded-box border border-base-300 bg-white"
        style="height: {height}px"
        onload={fit}
        data-testid="invoice-frame"
        data-mode="preview"
    ></iframe>
{:else if src !== undefined}
    <iframe
        bind:this={frame}
        {title}
        {src}
        sandbox="allow-same-origin"
        class="w-full rounded-box border border-base-300 bg-white"
        style="height: {height}px"
        onload={fit}
        data-testid="invoice-frame"
        data-mode="page"
    ></iframe>
{/if}
