// See https://svelte.dev/docs/kit/types#app.d.ts
// for information about these interfaces
declare global {
    namespace App {
        // interface Error {}
        // interface Locals {}
        // interface PageData {}
        interface PageState {
            /** New → the editor: what went wrong once the draft was written (`POST /api/invoices` `warnings`). */
            createWarnings?: string[];
        }
        // interface Platform {}
    }
}

export {};
