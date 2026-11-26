<!-- A note as MBR renders it, in a frame (the in-app note view, Phase 4
     feedback J9). By URL, never `srcdoc` (Phase 3 feedback F1), sandboxed
     with `allow-same-origin` only, as the invoice frame is (PLAN B2): no
     script in the page runs, so nothing in a note can reach this window's
     token, and its links can move only the frame, never the app.

     Outside links: an https link navigates the frame, which the shell's CSP
     (`frame-src`) lets through so that the desktop window's navigation
     handler sees it, opens it in the default browser and keeps the frame
     where it is; in `--server` mode, in a plain browser, it loads in the
     sandboxed frame. A mailto link would be refused in a sandboxed frame, so
     `handMailtoLinks` (lib/frameLinks.ts) turns its click into a navigation
     of this window: the desktop window hands it to the mail app, a plain
     browser to its mail client, and the page stays. `height` in pixels; 0
     fills the space left (at least 24rem). -->
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
