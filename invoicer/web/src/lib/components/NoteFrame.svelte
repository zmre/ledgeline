<!-- A note as MBR renders it, in a frame (the in-app note view, Phase 4
     feedback J9). By URL, never `srcdoc` (Phase 3 feedback F1), sandboxed
     with `allow-same-origin` only, as the invoice frame is (PLAN B2): no
     script in the page runs, so nothing in a note can reach this window's
     token, and its links can move only the frame, never the app. `height`
     in pixels; 0 fills the space left (at least 24rem). -->
<script lang="ts">
    let {src, title, height = 520}: {src: string; title: string; height?: number} = $props();
</script>

<iframe
    {title}
    {src}
    sandbox="allow-same-origin"
    class="w-full rounded-box border border-base-300 bg-white {height === 0 ? 'min-h-96 flex-1' : ''}"
    style={height === 0 ? undefined : `height: ${height}px`}
    data-testid="note-frame"
></iframe>
