// The blur-vs-click guard for a field with a popup list (AccountInput,
// Combobox): a pointerdown on an option blurs the field, and without a guard
// the blur would commit the old text and unmount the list before the click
// landed. The guard holds from the press until its click, or until the press
// ends without one (dragged off and released, a pointercancel, a touch
// scroll of the list): then `abandoned` runs, so the field can do what its
// skipped blur would have done. A guard left set forever made every later
// blur skip its commit.

export interface PressGuard {
    /** A press on the popup is under way: the field's blur is the press's, not a leave. */
    readonly pressing: boolean;
    /** On the option's (or chevron's) pointerdown. */
    press(): void;
    /** On its click: the press became a choice. */
    release(): void;
}

export function pressGuard(abandoned: () => void): PressGuard {
    let pressing = false;
    const settle = (): void => {
        window.removeEventListener("pointerup", settle);
        window.removeEventListener("pointercancel", settle);
        // A task later: the click, when there is one, comes after pointerup.
        setTimeout(() => {
            if (!pressing) return;
            pressing = false;
            abandoned();
        }, 0);
    };
    return {
        get pressing() {
            return pressing;
        },
        press() {
            pressing = true;
            window.addEventListener("pointerup", settle);
            window.addEventListener("pointercancel", settle);
        },
        release() {
            pressing = false;
        },
    };
}
