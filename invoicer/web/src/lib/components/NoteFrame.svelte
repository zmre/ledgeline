<!-- A note as MBR renders it, in a frame (the in-app note view, Phase 4
     feedback J9). By URL, never `srcdoc` (Phase 3 feedback F1), sandboxed
     with `allow-same-origin` only, as the invoice frame is (PLAN B2): no
     script in the page runs, so nothing in a note can reach this window's
     token, and its links can move only the frame, never the app.

     Outside links: an https link navigates the frame, which the shell's CSP
     (`frame-src`) lets through so that the desktop window's navigation
     handler sees it, opens it in the default browser and keeps the frame
     where it is; in `--server` mode, in a plain browser, it loads in the
     sandboxed frame. A sandboxed frame may not load a mailto link at all,
     so on each load `handMailtoLinks` (lib/frameLinks.ts) rewrites the
     frame's mailto links to a same-origin hand-off, which the desktop
     window opens in the mail app while the frame stays put; in `--server`
     mode the server answers it with nothing, so mailto links do nothing
     there. `height` in pixels; 0 fills the space left (at least 24rem). -->
<script lang="ts">
    import {handMailtoLinks} from "../frameLinks";

    let {src, title, height = 520}: {src: string; title: string; height?: number} = $props();

    let frame = $state<HTMLIFrameElement | null>(null);
</script>

<iframe
    bind:this={frame}
    onload={() => frame && handMailtoLinks(frame)}
    {title}
    {src}
    sandbox="allow-same-origin"
    class="w-full rounded-box border border-base-300 bg-white {height === 0 ? 'min-h-96 flex-1' : ''}"
    style={height === 0 ? undefined : `height: ${height}px`}
    data-testid="note-frame"
></iframe>
