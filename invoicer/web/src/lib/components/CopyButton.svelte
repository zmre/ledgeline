<!-- Copy one value from the customer card (contact design §3.5): a small
     square button beside the value, revealed on hover or focus on a wide
     window with a mouse, always shown on touch and narrow windows (app.css,
     `.copy-reveal`), and always in the tab order. On success it shows a
     check, and its name reads "Copied", for 1.5 s; the card's status region
     announces it either way (`copyValue`). -->
<script lang="ts">
    import {copyValue} from "#lib/copy.svelte.js";
    import Icon from "./Icon.svelte";

    let {
        value,
        what,
        keys,
    }: {
        value: string;
        /** What it is, in words: "billing email". */
        what: string;
        /** Its chord, for the tooltip ("y e"); a chord can't be an `aria-keyshortcuts`. */
        keys?: string;
    } = $props();

    let copied = $state(false);
    let timer: ReturnType<typeof setTimeout> | undefined;

    async function copy(): Promise<void> {
        if (!(await copyValue(value, what))) return;
        copied = true;
        clearTimeout(timer);
        timer = setTimeout(() => (copied = false), 1500);
    }

    $effect(() => () => clearTimeout(timer));
</script>

<button
    type="button"
    class="copy-reveal btn btn-square btn-ghost btn-xs"
    aria-label={copied ? "Copied" : `Copy ${what}`}
    title={copied ? "Copied" : `Copy ${what}${keys === undefined ? "" : ` (${keys})`}`}
    onclick={() => void copy()}
    data-testid="copy-button"
    data-copy={what}
>
    <Icon name={copied ? "check" : "copy"} class="size-3.5" />
</button>
