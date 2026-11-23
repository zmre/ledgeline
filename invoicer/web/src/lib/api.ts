// The invoicer's API client (invoicer PLAN 3a). The SPA is served by the
// ledgeline-invoicer server under /app, so every request is same-origin and
// relative; under `vite dev` (port 4273) the vite proxy forwards /api to the
// `--server` on 5199. The token comes from the shared client in packages/ui:
// injected by the window, or adopted from the `#token=` launch fragment.
import {ApiShapeError, ApiUnreachableError, authHeaders} from "#ui/api/client.js";

/** `GET /api/ui-prefs`'s body: the shared theme and scale, among others. */
export interface UiPrefsBody {
    prefs: {theme?: unknown; uiScale?: unknown};
}

/** The saved theme and scale, or null when the body does not carry them. */
export function appearanceFrom(body: UiPrefsBody): {theme: string; uiScale: number} | null {
    const {theme, uiScale} = body.prefs ?? {};
    if (typeof theme !== "string" || typeof uiScale !== "number") return null;
    return {theme, uiScale};
}

async function call(method: string, path: string): Promise<Response> {
    let response: Response;
    try {
        response = await fetch(path, {method, headers: authHeaders({Accept: "application/json"}, path)});
    } catch (error) {
        throw new ApiUnreachableError(`${method} ${path}: ${String(error)}`, {cause: error});
    }
    if (!response.ok) throw new ApiUnreachableError(`${method} ${path}: HTTP ${response.status}`);
    return response;
}

/** The effective UI prefs. */
export async function fetchUiPrefs(): Promise<UiPrefsBody> {
    const body: unknown = await (await call("GET", "/api/ui-prefs")).json();
    if (typeof body !== "object" || body === null || !("prefs" in body)) {
        throw new ApiShapeError("GET /api/ui-prefs: no prefs in the body");
    }
    return body as UiPrefsBody;
}

/**
 * Ask for the session cookie MBR's pages need (they load by navigation, so
 * they can't carry the bearer token).
 */
export async function startSession(): Promise<void> {
    await call("POST", "/api/session");
}
