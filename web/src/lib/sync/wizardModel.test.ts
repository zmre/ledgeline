import {describe, expect, it} from "vitest";
import {allyRemote, chaseRemote, CONNECTION_ID, firstTechRemote, fixturePreviews, fixtureRun, FIXTURE_TODAY} from "./fixtures";
import {
    balanceCompare,
    checkSetupToken,
    chunkNote,
    decodeBase64,
    draftFor,
    draftIssues,
    followAccount,
    historyNote,
    linkRequests,
    looksLikeSetupToken,
    newRulesName,
    previewLine,
    previousStep,
    railSteps,
    rulesFor,
    signQuestion,
    toLinkRequest,
    wizardInit,
    wizardReduce,
} from "./wizardModel";

const token = (url: string): string => btoa(url);
const urlSafe = (url: string): string => btoa(url).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");

describe("UNIT sync wizardModel steps", () => {
    it("walks choose → simplefin → link → preview → done", () => {
        let s = wizardInit();
        expect(previousStep(s)).toBeNull();
        s = wizardReduce(s, {type: "choose", method: "simplefin"});
        expect(s.step).toBe("simplefin");
        s = wizardReduce(s, {type: "claimed", connectionId: CONNECTION_ID, accounts: [firstTechRemote()]});
        expect(s).toMatchObject({step: "link", connectionId: CONNECTION_ID});
        s = wizardReduce(s, {type: "previewed", previews: fixturePreviews()});
        expect(s.step).toBe("preview");
        s = wizardReduce(s, {type: "finished", run: fixtureRun()});
        expect(s.step).toBe("done");
        expect(previousStep(s)).toBeNull();
        expect(wizardReduce(s, {type: "back"})).toBe(s);
    });

    it("goes back one step and drops the preview", () => {
        let s = wizardReduce(wizardInit("link", CONNECTION_ID, [firstTechRemote()]), {type: "previewed", previews: fixturePreviews()});
        s = wizardReduce(s, {type: "back"});
        expect(s).toMatchObject({step: "link", previews: null});
        // Never before where it opened.
        expect(wizardReduce(s, {type: "back"}).step).toBe("link");
    });

    it("closes and highlights Manual downloads for 'download myself'", () => {
        expect(wizardReduce(wizardInit(), {type: "choose", method: "manual"}).closed).toEqual({highlightManual: true});
    });

    it("ignores events out of order", () => {
        const s = wizardInit();
        expect(wizardReduce(s, {type: "previewed", previews: []})).toBe(s);
        expect(wizardReduce(s, {type: "finished", run: fixtureRun()})).toBe(s);
        const linking = wizardInit("link");
        expect(wizardReduce(linking, {type: "choose", method: "simplefin"})).toBe(linking);
    });

    it("shows the rail from where it opened", () => {
        expect(railSteps(wizardInit())).toEqual(["choose", "simplefin", "link", "preview"]);
        expect(railSteps(wizardInit("simplefin"))).toEqual(["simplefin", "link", "preview"]);
        expect(railSteps(wizardInit("link"))).toEqual(["link", "preview"]);
    });

    it("takes a re-check's accounts", () => {
        const s = wizardReduce(wizardInit("link", CONNECTION_ID, []), {type: "accounts", accounts: [allyRemote()]});
        expect(s.accounts).toHaveLength(1);
    });
});

describe("UNIT sync wizardModel setup token", () => {
    it("shows the Bridge host for a Bridge token, standard or URL-safe", () => {
        const url = "https://beta-bridge.simplefin.org/simplefin/claim/DEMO";
        for (const t of [token(url), urlSafe(url), `  ${token(url)}\n`]) {
            expect(checkSetupToken(t)).toEqual({
                state: "ok",
                host: "beta-bridge.simplefin.org",
                claimUrl: url,
                bridge: true,
                note: "Token from beta-bridge.simplefin.org",
            });
        }
    });

    it("flags a host that isn't the Bridge", () => {
        const check = checkSetupToken(token("https://beta-bridge-example.com/claim/abc"));
        expect(check).toMatchObject({state: "ok", bridge: false, note: "Not SimpleFIN Bridge. Only continue if you trust beta-bridge-example.com."});
    });

    it("refuses http", () => {
        const check = checkSetupToken(token("http://beta-bridge.simplefin.org/simplefin/claim/DEMO"));
        expect(check.state).toBe("invalid");
        expect(check.state === "invalid" && check.message).toContain("plain http");
    });

    it("refuses what isn't a token", () => {
        expect(checkSetupToken("")).toEqual({state: "empty"});
        expect(checkSetupToken("   ")).toEqual({state: "empty"});
        for (const bad of ["not base64 at all!", token("hello world"), token("https://example.com/no-claim"), token("ftp://x.org/claim/1"), "a"]) {
            expect(checkSetupToken(bad).state).toBe("invalid");
        }
        expect(checkSetupToken("https://user:pass@beta-bridge.simplefin.org/simplefin")).toMatchObject({
            state: "invalid",
            message: expect.stringContaining("access URL"),
        });
    });

    it("decodes base64 strictly", () => {
        expect(decodeBase64(btoa("hi"))).toBe("hi");
        expect(decodeBase64("aGk")).toBe("hi");
        expect(decodeBase64("%%%")).toBeNull();
        // Not UTF-8.
        expect(decodeBase64(btoa("ÿþ"))).toBeNull();
    });

    it("offers clipboard text only when it decodes to a claim URL", () => {
        expect(looksLikeSetupToken(token("https://beta-bridge.simplefin.org/simplefin/claim/X"))).toBe(true);
        expect(looksLikeSetupToken(token("http://beta-bridge.simplefin.org/simplefin/claim/X"))).toBe(true);
        expect(looksLikeSetupToken("my grocery list")).toBe(false);
    });
});

describe("UNIT sync wizardModel drafts", () => {
    it("names new rules files from the account", () => {
        expect(newRulesName("assets:bank:firsttech:checking")).toBe("firsttech-checking.sync.csv.rules");
        expect(newRulesName("liabilities:cc:chase")).toBe("chase.sync.csv.rules");
        expect(newRulesName("assets:bank:ally:savings")).toBe("ally-savings.sync.csv.rules");
        expect(newRulesName("assets")).toBe("sync.sync.csv.rules");
        expect(newRulesName("Assets:Bank:Big Credit Union:Share Draft")).toBe("big-credit-union-share-draft.sync.csv.rules");
    });

    it("seeds a draft from the suggestion", () => {
        const draft = draftFor(firstTechRemote(), "main.journal", FIXTURE_TODAY);
        expect(draft).toMatchObject({
            include: true,
            journalAccount: "assets:bank:firsttech:checking",
            startDate: "2026-11-01",
            target: "2026.journal",
            rulesChoice: "new",
            newRulesName: "firsttech-checking.sync.csv.rules",
            share: "separate",
            balanceSign: 1,
        });
        expect(draftIssues(draft)).toEqual([]);
    });

    it("prefers a fitting sync rules file, asks about sharing, and takes the flipped sign", () => {
        const draft = draftFor(chaseRemote(), "main.journal", FIXTURE_TODAY);
        expect(draft).toMatchObject({rulesChoice: "imports/rules/old-chase.sync.csv.rules", share: "ask", balanceSign: -1});
        expect(draftIssues(draft)).toEqual(["Say whether to share the existing categorization rules."]);
        expect(toLinkRequest({...draft, share: "share"}, chaseRemote())?.share).toEqual({manualRulesId: "imports/rules/chase.csv.rules"});
        expect(toLinkRequest({...draft, share: "separate"}, chaseRemote())?.share).toBeNull();
    });

    it("never preselects another account's rules file", () => {
        const remote = chaseRemote();
        remote.suggestion.syncRules = [{id: "imports/rules/visa.sync.csv.rules", fits: true, account1: "liabilities:cc:visa"}];
        expect(draftFor(remote, "main.journal", FIXTURE_TODAY).rulesChoice).toBe("new");
        expect(rulesFor(remote.suggestion.syncRules, "liabilities:cc:visa")).toBe("imports/rules/visa.sync.csv.rules");
        expect(rulesFor([{id: "x.sync.csv.rules", fits: true, account1: null}], "liabilities:cc:visa")).toBe("new");
    });

    it("follows a corrected account: the new file's name, and the choice back to New", () => {
        // A guessed liabilities:mystery, with its own (fitting) file, corrected to a new liabilities:mortgage.
        const remote = chaseRemote();
        const choices = [
            {id: "imports/rules/mystery.sync.csv.rules", fits: true, account1: "liabilities:mystery"},
            {id: "imports/rules/creditcard.sync.csv.rules", fits: true, account1: "liabilities:cc:visa"},
        ];
        remote.suggestion = {...remote.suggestion, journalAccount: null, newAccountName: "liabilities:mystery", syncRules: choices};
        const draft = draftFor(remote, "main.journal", FIXTURE_TODAY);
        draft.include = true;
        draft.createAccount = true;
        expect(draft.rulesChoice).toBe("imports/rules/mystery.sync.csv.rules");

        draft.newAccount.name = "liabilities:mortgage";
        followAccount(draft, choices);
        expect(draft).toMatchObject({rulesChoice: "new", newRulesName: "mortgage.sync.csv.rules"});

        // Back to an account with a file of its own: that file again.
        draft.createAccount = false;
        draft.journalAccount = "liabilities:cc:visa";
        followAccount(draft, choices);
        expect(draft).toMatchObject({rulesChoice: "imports/rules/creditcard.sync.csv.rules", newRulesName: "visa.sync.csv.rules"});

        // A file that belongs to no account in particular stays picked.
        const shared = [...choices, {id: "imports/rules/any.sync.csv.rules", fits: true, account1: null}];
        draft.rulesChoice = "imports/rules/any.sync.csv.rules";
        draft.journalAccount = "liabilities:mortgage";
        followAccount(draft, shared);
        expect(draft.rulesChoice).toBe("imports/rules/any.sync.csv.rules");
    });

    it("starts an unknown account as Don't sync, with the new-account option ready", () => {
        const draft = draftFor(allyRemote(), "main.journal", FIXTURE_TODAY);
        expect(draft).toMatchObject({include: false, target: "main.journal", newRulesName: "ally-savings.sync.csv.rules"});
        expect(draft.newAccount).toEqual({name: "assets:bank:ally:savings", type: "C", openingBalance: "20000.00", openingDate: "2026-11-13"});
        expect(toLinkRequest(draft, allyRemote())).toBeNull();
        const creating = {...draft, include: true, createAccount: true};
        expect(draftIssues(creating)).toEqual([]);
        expect(toLinkRequest(creating, allyRemote())).toEqual({
            connectionId: CONNECTION_ID,
            remoteId: "ACT-ally-0077",
            label: "Ally · Savings ••0077",
            journalAccount: "assets:bank:ally:savings",
            newAccount: {type: "C", openingBalance: "20000.00", openingDate: "2026-11-13"},
            startDate: "2026-08-15",
            target: "main.journal",
            balanceSign: 1,
            rules: {mode: "new", name: "ally-savings.sync.csv.rules"},
            share: null,
        });
    });

    it("lists what blocks a draft", () => {
        const base = {...draftFor(firstTechRemote(), "main.journal", FIXTURE_TODAY)};
        expect(draftIssues({...base, journalAccount: ""})).toEqual(["Choose a journal account."]);
        expect(draftIssues({...base, journalAccount: "checking"})).toEqual(["A journal account looks like assets:bank:checking."]);
        expect(draftIssues({...base, startDate: ""})).toEqual(["Choose a date to start syncing from."]);
        expect(draftIssues({...base, target: " "})).toEqual(["Choose the file to write into."]);
        expect(draftIssues({...base, newRulesName: "../evil.rules"})).toEqual(["A new rules file needs a plain name ending in .rules."]);
        expect(draftIssues({...base, createAccount: true, newAccount: {...base.newAccount, openingBalance: "lots"}})).toContain(
            "The opening balance must be a number, like 1234.56."
        );
        expect(draftIssues({...base, include: false, journalAccount: ""})).toEqual([]);
    });

    it("builds every request, or says what blocks them", () => {
        const remotes = [firstTechRemote(), allyRemote()];
        const drafts = remotes.map((r) => draftFor(r, "main.journal", FIXTURE_TODAY));
        expect(linkRequests(drafts, remotes)).toMatchObject({issues: [], requests: [{remoteId: "ACT-firsttech-1234"}]});
        expect(linkRequests([{...drafts[0], include: false}, drafts[1]], remotes)).toEqual({requests: [], issues: ["Choose at least one account to sync."]});
        expect(linkRequests([{...drafts[0], target: ""}], remotes).issues).toEqual(["First Tech · Checking ••1234: Choose the file to write into."]);
    });
});

describe("UNIT sync wizardModel notes", () => {
    it("asks about a reversed sign", () => {
        expect(signQuestion(chaseRemote())).toBe("Chase reports this as +1,032.18. Your ledger has −1,032.18. Treat Chase's sign as reversed?");
        expect(signQuestion(firstTechRemote())).toBeNull();
    });

    it("compares the bank's balance with the ledger's", () => {
        expect(balanceCompare(chaseRemote(), -1)).toEqual({text: "ledger: −$1,032.18 ✓ matches", matches: true});
        expect(balanceCompare(firstTechRemote(), 1)).toEqual({text: "ledger: $4,198.15 ✓ close", matches: true});
        expect(balanceCompare({...firstTechRemote(), balance: "100.00"}, 1)).toEqual({text: "ledger: $4,198.15", matches: false});
        expect(balanceCompare(allyRemote(), 1)).toBeNull();
    });

    it("warns when the history needs more than 4 requests", () => {
        expect(chunkNote("2026-08-20", FIXTURE_TODAY)).toBeNull();
        expect(chunkNote("2025-06-01", FIXTURE_TODAY)).toBe("That much history takes 6 requests to SimpleFIN, which allows about 24 a day.");
        expect(chunkNote("bad", FIXTURE_TODAY)).toBeNull();
    });

    it("says how far back the bank goes", () => {
        expect(historyNote(firstTechRemote(), "2026-06-01", FIXTURE_TODAY, "en-US")).toBe("First Tech only provides history back to Aug 20.");
        expect(historyNote(firstTechRemote(), "2026-09-01", FIXTURE_TODAY)).toBeNull();
    });

    it("words a preview", () => {
        expect(previewLine(fixturePreviews()[0])).toBe("38 new · 0 already in your journal · 2 possible duplicates held · 6 uncategorized");
        expect(previewLine(fixturePreviews()[1])).toBe(
            "12 new · 3 already in your journal · 0 possible duplicates held · 1 uncategorized · 4 before the start date"
        );
    });
});
