// Carrying out a bank sync problem's button (plan 43 §6), wherever it is
// shown: the Accounts tab, the account drawer, the Problems drawer.
//
// The Accounts tab hands in its own handlers for what opens there (the
// wizard, the account drawer). Anywhere else, those become a request the tab
// takes when it next shows, and a navigation to it.

import {goto} from "$app/navigation";
import {resolve} from "$app/paths";
import {openJournal} from "#lib/journal/openJournal.js";
import type {ProblemEffect} from "../problemAction";
import {syncStore, type PanelIntent} from "../syncStore.svelte";

/** What the host can open itself. Absent: the request goes to the Accounts tab. */
export type LocalOpen = (intent: PanelIntent) => void;

/** `/imports?tab=…`, the one place the query string is spelled. */
export function importsHref(tab: "accounts" | "new" | "rules"): string {
    return `${resolve("/imports")}?tab=${tab}`;
}

export async function runEffect(effect: ProblemEffect, local?: LocalOpen): Promise<void> {
    switch (effect.type) {
        case "url":
            // Rendered as a link by ProblemNotice; this path is for keyboard
            // shortcuts or a caller without one. `noopener`: the page opened is
            // the Bridge's or a bank's, and gets no handle on this window.
            window.open(effect.url, "_blank", "noopener,noreferrer");
            return;
        case "sync":
            await syncStore.syncNow({
                ...(effect.accountKey !== null ? {accountKey: effect.accountKey} : {}),
                ...(effect.accountKey === null && effect.connectionId !== null ? {connectionId: effect.connectionId} : {}),
            });
            return;
        case "edit-rules":
            // eslint-disable-next-line svelte/no-navigation-without-resolve -- importsHref resolves the route; the query string is appended
            await goto(importsHref("rules"));
            return;
        case "set-hledger":
            // The hledger path's banner lives on New Transactions.
            // eslint-disable-next-line svelte/no-navigation-without-resolve -- importsHref resolves the route; the query string is appended
            await goto(importsHref("new"));
            return;
        case "show-balance": {
            const account = syncStore.status?.accounts.find((a) => a.key === effect.accountKey)?.journalAccount;
            if (account !== undefined) await openJournal({accounts: [account], pin: {account}, preset: "all"});
            return;
        }
        case "reconnect":
        case "review-held":
        case "choose-file":
        case "connect":
            if (local !== undefined) {
                local(effect);
                return;
            }
            syncStore.request(effect);
            // eslint-disable-next-line svelte/no-navigation-without-resolve -- importsHref resolves the route; the query string is appended
            await goto(importsHref("accounts"));
            return;
    }
}
