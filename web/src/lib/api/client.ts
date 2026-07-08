// hledger-web JSON API client (WP-02). Fetch + error taxonomy only; wire-shape
// knowledge lives in normalize.ts.

/** Network/CORS/HTTP failure — the setup modal reacts by showing the launch command. */
export class ApiUnreachableError extends Error {
    constructor(message: string, options?: ErrorOptions) {
        super(message, options);
        this.name = "ApiUnreachableError";
    }
}

/** The server answered, but the JSON was not what an hledger-web API returns. */
export class ApiShapeError extends Error {
    constructor(message: string, options?: ErrorOptions) {
        super(message, options);
        this.name = "ApiShapeError";
    }
}

function stringArray(value: unknown, route: string): string[] {
    if (!Array.isArray(value) || !value.every((item): item is string => typeof item === "string")) {
        throw new ApiShapeError(`GET ${route}: expected a JSON array of strings`);
    }
    return value;
}

export class HledgerApi {
    readonly baseUrl: string;

    constructor(baseUrl: string) {
        this.baseUrl = baseUrl.replace(/\/+$/, "");
    }

    private async get(route: string): Promise<unknown> {
        const url = `${this.baseUrl}${route}`;
        let response: Response;
        try {
            response = await fetch(url, {headers: {Accept: "application/json"}});
        } catch (cause) {
            throw new ApiUnreachableError(`Cannot reach hledger-web at ${this.baseUrl} (network or CORS failure)`, {cause});
        }
        if (!response.ok) {
            throw new ApiUnreachableError(`GET ${url} responded ${response.status} ${response.statusText}`);
        }
        try {
            return (await response.json()) as unknown;
        } catch (cause) {
            throw new ApiShapeError(`GET ${route}: response is not valid JSON`, {cause});
        }
    }

    async version(): Promise<string> {
        const value = await this.get("/version");
        if (typeof value !== "string") throw new ApiShapeError("GET /version: expected a JSON string");
        return value;
    }

    /** Raw wire JSON; pass through normalizeTransactions separately. */
    transactions(): Promise<unknown> {
        return this.get("/transactions");
    }

    async accountNames(): Promise<string[]> {
        return stringArray(await this.get("/accountnames"), "/accountnames");
    }

    /** Raw wire JSON; pass through normalizePrices separately. */
    prices(): Promise<unknown> {
        return this.get("/prices");
    }

    async commodities(): Promise<string[]> {
        return stringArray(await this.get("/commodities"), "/commodities");
    }
}
