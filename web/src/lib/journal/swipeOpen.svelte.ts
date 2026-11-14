// Which journal card has its swipe actions open (plan 42): one at a time, as
// in Mail. Opening one closes any other; a scroll of the list does not.

let open = $state<number | null>(null);

export const swipeOpen = {
    /** The open card's transaction index, or null. */
    get current(): number | null {
        return open;
    },
    set(index: number | null): void {
        open = index;
    },
};
