// A dialog's keymap layer (PLAN 5k, Y8): modal, so while it is open the page
// beneath has no keys (`n`, `j`, Send's `S`, the bill grid's Alt+Shift row
// keys), and only an `always` binding (Cmd/Ctrl+S) reaches through. The
// dialog keeps its own Escape and focus handling (H16); this only blinds.
// Call during the dialog component's init, like `registerKeys`.

import {registerKeys} from "#ui/keys/keymap.svelte.js";
import {PRIORITY} from "#ui/keys/types.js";

export function modalKeys(id: string): void {
    registerKeys({id, priority: PRIORITY.overlay, modal: true, bindings: []});
}
