import type {ClientInit} from "@sveltejs/kit";
import {adoptLaunchToken} from "#lib/api/client.js";

// Runs once, before SvelteKit's router reads the URL or writes its first
// history entry: a headless engine's `#token=…` launch fragment is moved into
// sessionStorage and stripped from the address bar before anything can record
// it. `apiToken` would adopt it lazily anyway; this just does it first.
export const init: ClientInit = () => {
    adoptLaunchToken();
};
