// The last change the server reported (lib/events.ts), as shared state: the
// layout listens; each page re-fetches what it shows when `seq` moves
// (`onFilesChanged`).
import {untrack} from "svelte";

/** Bumped on every change event; `paths` is that event's (`[]`: everything). */
export const changes = $state<{seq: number; paths: string[]}>({seq: 0, paths: []});

/** Record one change event. */
export function noteChange(paths: string[]): void {
    changes.paths = paths;
    changes.seq += 1;
}

/**
 * Live reload: run `reload` once for each change event after this call (not
 * for one already seen), with that event's paths (`[]`: anything may have
 * changed). `reload` runs untracked, so what it reads doesn't re-run it.
 * Call it while a component initialises (it is an `$effect`).
 */
export function onFilesChanged(reload: (paths: string[]) => void): void {
    let seen = changes.seq;
    $effect(() => {
        const seq = changes.seq;
        if (seq === seen) return;
        seen = seq;
        untrack(() => reload(changes.paths));
    });
}
