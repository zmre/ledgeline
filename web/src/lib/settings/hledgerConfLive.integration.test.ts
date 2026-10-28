// INTEGRATION check against a live ledgeline-server. Skipped unless
// LEDGELINE_API_URL is set — `just test-integration` sets it (and
// LEDGELINE_IGNORE_USER_HLEDGER_CONF, so the engine never reads the
// developer's own ~/.hledger.conf).
//
// READ-ONLY on purpose: the engine is bound to `fixtures/sample.journal`, and a
// successful PUT would create `fixtures/hledger.conf` in the repository. What a
// live engine can prove without writing is the wire contract — the GET decodes
// through the SPA's decoder — and the revision guard: a stale revision is a 409
// before anything is touched.

import {describe, expect, it} from "vitest";
import {ConflictError, LedgelineApi} from "#lib/api/native.js";
import {decodeHledgerConf} from "#lib/api/nativeDecode.js";
import {CONF_SECTIONS, DEPTH_COMMANDS} from "./hledgerConfTypes";

const apiUrl = process.env.LEDGELINE_API_URL;

describe.runIf(apiUrl !== undefined && apiUrl !== "")("INTEGRATION live /api/hledger-conf", () => {
    const api = (): LedgelineApi => new LedgelineApi(apiUrl ?? "");

    it("decodes, with a client-safe handle and every section present", async () => {
        const conf = decodeHledgerConf(await api().hledgerConf());
        for (const section of CONF_SECTIONS) expect(conf.depth[section]).toBeDefined();
        for (const command of DEPTH_COMMANDS) expect(conf.effectiveDepth[command]).toBeDefined();
        for (const handle of [conf.handle, conf.createHandle]) {
            if (handle !== null) expect(handle.startsWith("/")).toBe(false);
        }
        // The fixture server is bound to a file, so it is editable unless a
        // config it found says otherwise.
        expect(typeof conf.editable).toBe("boolean");
    });

    it("refuses a stale revision with a 409 and writes nothing", async () => {
        const before = decodeHledgerConf(await api().hledgerConf());
        await expect(api().saveHledgerConf({revision: "not-the-revision", auto: !before.auto})).rejects.toBeInstanceOf(ConflictError);
        const after = decodeHledgerConf(await api().hledgerConf());
        expect(after.revision).toBe(before.revision);
        expect(after.handle).toBe(before.handle);
    });
});
