// The last change the server reported (lib/events.ts), as shared state: the
// layout listens; each page re-fetches what it shows when `seq` moves.

/** Bumped on every change event; `paths` is that event's (`[]`: everything). */
export const changes = $state<{seq: number; paths: string[]}>({seq: 0, paths: []});

/** Record one change event. */
export function noteChange(paths: string[]): void {
    changes.paths = paths;
    changes.seq += 1;
}
