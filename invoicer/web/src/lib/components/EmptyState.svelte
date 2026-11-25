<!-- One empty state for every list page (Phase 4 feedback H21): a centred
     card with a title, one sentence and a primary button that starts the
     first one. The button is a link when `href` is given (a page to go to),
     else a button calling `onAction` (the page makes it in place). With no
     `action` there is no button: the dashboard's points at the sidebar's New
     instead of repeating it (R13). -->
<script lang="ts">
    let {
        title,
        text,
        action,
        href,
        onAction,
        busy = false,
        testid,
        actionTestid,
    }: {
        title: string;
        /** One sentence: what goes here. */
        text: string;
        /** The button's label ("New invoice"); none: no button. */
        action?: string;
        href?: string;
        onAction?: () => void;
        busy?: boolean;
        testid: string;
        actionTestid?: string;
    } = $props();
</script>

<section class="mx-auto w-full max-w-lg py-6 sm:py-10" aria-labelledby="{testid}-title" data-testid={testid}>
    <div class="flex flex-col items-center gap-3 rounded-box border border-base-300 bg-base-200/50 px-5 py-8 text-center shadow-sm sm:px-8 sm:py-10">
        <h2 id="{testid}-title" class="text-lg font-semibold tracking-tight">{title}</h2>
        <p class="max-w-prose text-sm text-base-content/70">{text}</p>
        {#if action === undefined}
            <!-- No button: the text says where to start. -->
        {:else if href !== undefined}
            <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -- pages pass hrefs built with resolve() -->
            <a class="btn mt-2 btn-primary" {href} data-testid={actionTestid}>{action}</a>
        {:else}
            <button type="button" class="btn mt-2 btn-primary" disabled={busy} onclick={() => onAction?.()} data-testid={actionTestid}>{action}</button>
        {/if}
    </div>
</section>
