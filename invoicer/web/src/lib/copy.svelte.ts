// Copying a value from the customer card (a copy button, or a `y` chord):
// the clipboard write and the one shared announcement the card's
// `role="status"` region reads (`CopyStatus`). The failure message is the
// Send panel's.

/** What the last copy did, for the card's status region. */
export const copyStatus = $state({message: ""});

/** Put `text` on the clipboard; `what` names it ("billing email"). Whether it worked. */
export async function copyValue(text: string, what: string): Promise<boolean> {
    try {
        await navigator.clipboard.writeText(text);
        copyStatus.message = `Copied ${what}`;
        return true;
    } catch {
        copyStatus.message = `Couldn't copy the ${what}; select it and copy by hand.`;
        return false;
    }
}
