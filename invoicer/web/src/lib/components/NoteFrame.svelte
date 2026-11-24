<!-- A note as MBR renders it, in a frame (the customer's contact card: MBR's
     own page for the saved note, invoicer PLAN 4b). By URL, never `srcdoc`
     (Phase 3 feedback F1), sandboxed with `allow-same-origin` only, as the
     invoice frame is (PLAN B2): no script in the page runs, so nothing in a
     note can reach this window's token. The page wraps it in `{#key}` to
     show the file again after a save. -->
<script lang="ts">
    let {src, title, height = 520}: {src: string; title: string; height?: number} = $props();
</script>

<iframe {title} {src} sandbox="allow-same-origin" class="w-full rounded-box border border-base-300 bg-white" style="height: {height}px" data-testid="note-frame"
></iframe>
