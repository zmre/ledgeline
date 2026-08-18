import {describe, expect, it} from "vitest";
import {
    ACCOUNT_LABEL_BUDGET,
    abbreviateAccount,
    accountMatches,
    accountRenderings,
    buildAccountTree,
    categorize,
    clampAccount,
    fitAccount,
    shareWidths,
} from "./accounts";

// A deliberately LUMPY proportional font, because the bug this file now guards
// was believing characters are a unit of width. Nothing here depends on the real
// metrics of any typeface — the point is only that glyphs differ, which every
// proportional font satisfies and a character count denies.
const NARROW = new Set([..."ilj:.,'"]);
const WIDE = new Set([..."mwMW"]);

/** Width of `text` in this fake font, in px. */
function px(text: string): number {
    return [...text].reduce((sum, glyph) => sum + (NARROW.has(glyph) ? 3 : WIDE.has(glyph) ? 10 : 6), 0);
}

const REAL_NAMES = [
    "assets:bank:checking",
    "expenses:auto:maintenance",
    "expenses:household:repairs:plumbing",
    "assets:morganstanley:pw-roth-ira:cash",
    "liabilities:creditcards:visa-signature",
    "revenues:consulting",
    "singleaccountwithnoancestorsatall",
];

describe("UNIT accounts", () => {
    describe("buildAccountTree", () => {
        it("builds a nested, sorted tree from flat names", () => {
            const tree = buildAccountTree([
                "expenses:food",
                "assets:bank:checking",
                "assets:bank:savings",
                "assets:broker",
                "expenses",
                "assets:bank",
                "assets",
            ]);
            expect(tree.map((node) => node.fullName)).toEqual(["assets", "expenses"]);
            const assets = tree[0];
            expect(assets.name).toBe("assets");
            expect(assets.children.map((node) => node.fullName)).toEqual(["assets:bank", "assets:broker"]);
            expect(assets.children[0].children.map((node) => node.name)).toEqual(["checking", "savings"]);
        });

        it("creates missing intermediate ancestors", () => {
            const tree = buildAccountTree(["assets:bank:checking"]);
            expect(tree).toHaveLength(1);
            expect(tree[0].fullName).toBe("assets");
            expect(tree[0].children[0].fullName).toBe("assets:bank");
            expect(tree[0].children[0].children[0].fullName).toBe("assets:bank:checking");
        });

        it("handles empty input", () => {
            expect(buildAccountTree([])).toEqual([]);
        });
    });

    describe("clampAccount", () => {
        it("clamps to the requested depth", () => {
            expect(clampAccount("assets:morganstanley:checking", 1)).toBe("assets");
            expect(clampAccount("assets:morganstanley:checking", 2)).toBe("assets:morganstanley");
            expect(clampAccount("assets:morganstanley:checking", 3)).toBe("assets:morganstanley:checking");
            expect(clampAccount("assets", 4)).toBe("assets");
        });
    });

    describe("accountMatches", () => {
        it("matches exact names and sub-accounts only", () => {
            expect(accountMatches("assets:bank", "assets:bank")).toBe(true);
            expect(accountMatches("assets:bank", "assets:bank:checking")).toBe(true);
            expect(accountMatches("assets:bank", "assets:bankx")).toBe(false);
            expect(accountMatches("assets:bank:checking", "assets:bank")).toBe(false);
        });
    });

    describe("abbreviateAccount", () => {
        // The bug this exists for: a chip that reads `expenses:auto:ma…` has
        // spent its width on the segment the reader already knew. Every case
        // below is really the same assertion — the LEAF is still there.

        it("leaves anything within the budget exactly as it is", () => {
            expect(abbreviateAccount("expenses:auto:maintenance")).toBe("expenses:auto:maintenance");
            expect(abbreviateAccount("assets:bank:checking")).toBe("assets:bank:checking");
            expect(abbreviateAccount("")).toBe("");
            // The boundary itself is not an abbreviation: 30 characters, budget 30.
            const exact = "expenses:utilities:water:sewer";
            expect(exact).toHaveLength(ACCOUNT_LABEL_BUDGET);
            expect(abbreviateAccount(exact)).toBe(exact);
        });

        it("spends the outermost ancestors first, and only as far as it must", () => {
            // 35 chars. Shortening `expenses` alone gets under 30, so
            // `household` and `repairs` are left whole.
            expect(abbreviateAccount("expenses:household:repairs:plumbing")).toBe("exp:household:repairs:plumbing");
            // 37 chars. One is not enough here, so the second segment goes too.
            expect(abbreviateAccount("assets:morganstanley:pw-roth-ira:cash")).toBe("ass:mor:pw-roth-ira:cash");
        });

        it("keeps the leaf whole down to single-letter ancestors", () => {
            expect(abbreviateAccount("expenses:auto:maintenance", 16)).toBe("e:a:maintenance");
            expect(abbreviateAccount("assets:broker:taxable:vti", 10)).toBe("a:b:t:vti");
            // Depth and the leaf survive even when the budget is absurd; the
            // label's CSS finishes the job by clipping the left edge.
            expect(abbreviateAccount("assets:broker:taxable:vti", 1)).toBe("a:b:t:vti");
        });

        it("returns a too-long leaf untouched rather than mangling it", () => {
            // Nothing to spend: no ancestors at all. Truncating here would cut
            // the only thing on the chip, so the CSS ellipsis takes it instead.
            const single = "averyveryverylongsingleaccountname";
            expect(abbreviateAccount(single)).toBe(single);
            // Same when the leaf alone blows the budget: ancestors go to one
            // letter and the leaf is still whole and still over.
            expect(abbreviateAccount("expenses:reimbursable-conference-travel")).toBe("e:reimbursable-conference-travel");
        });

        it("counts and cuts whole characters, not UTF-16 halves", () => {
            // Each `🇺🇸` is four UTF-16 units and one visible character. Cutting
            // by `.slice()` would leave a lone surrogate (renders as `�`);
            // cutting by code point would leave half a flag (a stray letter in
            // a box). Both are wrong, and both used to be one `.slice()` away.
            const flags = "🇺🇸🇺🇸🇺🇸🇺🇸:expenses:maintenance-and-repairs";
            const short = abbreviateAccount(flags);

            expect(short).toBe("🇺🇸:exp:maintenance-and-repairs");
            expect(short).not.toContain("�");
            expect([...short].every((unit) => unit.codePointAt(0)! < 0xd800 || unit.codePointAt(0)! > 0xdfff)).toBe(true);
            // A combining mark stays with the letter it belongs to. The
            // accent here is DECOMPOSED (`e` + U+0301), which is what a name
            // pasted out of macOS Finder looks like: two code points for one
            // visible character, so a code-point cut at three would hand back
            // `Ame` with the accent quietly dropped.
            const decomposed = "Ame\u0301lie:budget:travel";
            const cut = abbreviateAccount(decomposed, 14);

            expect(cut.normalize("NFC")).toBe("Am\u00e9:bud:travel");
            expect(cut).toContain("\u0301");
            // Width is counted the same way, so an accented name is not
            // abbreviated for length it does not visibly have.
            expect(abbreviateAccount("dépenses:café:entretien")).toBe("dépenses:café:entretien");
        });

        it("never invents or drops a separator", () => {
            const deep = "a:bb:ccc:dddd:eeeee:ffffff:ggggggg:hhhhhhhh:leaf";
            expect(abbreviateAccount(deep).split(":")).toHaveLength(deep.split(":").length);
            expect(abbreviateAccount(deep).endsWith(":leaf")).toBe(true);
        });
    });

    describe("accountRenderings", () => {
        it("offers the full name first and narrows strictly from there", () => {
            const ladder = [...accountRenderings("assets:morganstanley:pw-roth-ira:cash")];

            expect(ladder[0]).toBe("assets:morganstanley:pw-roth-ira:cash");
            expect(ladder).toEqual([...new Set(ladder)]);
            for (let i = 1; i < ladder.length; i++) expect(px(ladder[i])).toBeLessThan(px(ladder[i - 1]));
        });

        it("keeps the leaf and the depth on every rung", () => {
            for (const name of REAL_NAMES) {
                const depth = name.split(":").length;
                const leaf = name.split(":").at(-1);
                for (const rendering of accountRenderings(name)) {
                    expect(rendering.split(":")).toHaveLength(depth);
                    expect(rendering.split(":").at(-1)).toBe(leaf);
                }
            }
        });

        it("has nothing to offer a name with no ancestors", () => {
            expect([...accountRenderings("cash")]).toEqual(["cash"]);
        });
    });

    describe("fitAccount", () => {
        // The regression these exist for: the label used to abbreviate to a
        // fixed THIRTY CHARACTERS, so it gave up width it had and the chip,
        // being `width: fit-content`, shrank to match — leaving the tail of the
        // accounts column blank. Every case below is the same assertion from a
        // different side: nothing is given up that the room could have shown.

        it("returns the whole name whenever it fits, exactly or with room to spare", () => {
            for (const name of REAL_NAMES) {
                expect(fitAccount(name, px(name), px)).toBe(name);
                expect(fitAccount(name, px(name) + 40, px)).toBe(name);
            }
        });

        it("gives up no more than it must at any width", () => {
            for (const name of REAL_NAMES) {
                const ladder = [...accountRenderings(name)];
                for (const room of [0, 20, 40, 60, 90, 120, 150, 181, 240, 400]) {
                    const chosen = fitAccount(name, room, px);
                    const rung = ladder.indexOf(chosen);

                    expect(rung).toBeGreaterThanOrEqual(0);
                    // Everything wider than what was chosen genuinely did not
                    // fit, so the chosen string is the widest one available.
                    for (const wider of ladder.slice(0, rung)) expect(px(wider)).toBeGreaterThan(room);
                    // And what was chosen either fits or is the last resort.
                    if (rung < ladder.length - 1) expect(px(chosen)).toBeLessThanOrEqual(room);
                }
            }
        });

        it("REGRESSION: a character budget is wrong in BOTH directions, and pixels are not", () => {
            // 32 narrow characters. The old budget of 30 abbreviated this; the
            // chip it was abbreviated for is ~181px wide and this is 96px, so
            // the whole name was on offer and a third of the width went unused.
            const narrow = "illiili:jillili:lilliji:illjilli";
            expect(narrow).toHaveLength(32);
            expect(px(narrow)).toBe(96);
            expect(abbreviateAccount(narrow)).not.toBe(narrow);
            expect(fitAccount(narrow, 181, px)).toBe(narrow);

            // 28 wide characters. The old budget left this ALONE — it is inside
            // thirty — while at 266px it overflows the same chip by half again,
            // so the reader got a CSS clip where an abbreviation was available.
            const wide = "mwmwmwmw:mwmwmwmw:mwmwmwmwmw";
            expect(wide.length).toBeLessThanOrEqual(ACCOUNT_LABEL_BUDGET);
            expect(px(wide)).toBe(266);
            expect(abbreviateAccount(wide)).toBe(wide);
            expect(fitAccount(wide, 181, px)).toBe("mwm:mwm:mwmwmwmwmw");
        });

        it("widens the label as the column widens, instead of holding one budget", () => {
            const name = "expenses:household:repairs:plumbing";
            // Same name, three cell widths: more room buys back more of the
            // ancestors, one rung at a time, until the name is whole.
            expect(fitAccount(name, 100, px)).toBe("e:hou:rep:plumbing");
            expect(fitAccount(name, 140, px)).toBe("exp:hou:repairs:plumbing");
            expect(fitAccount(name, 175, px)).toBe("exp:household:repairs:plumbing");
            expect(fitAccount(name, 400, px)).toBe(name);
        });

        it("hands back the narrowest rung when even that overflows, rather than mangling the leaf", () => {
            // The CSS clips the left edge from here; the leaf survives either way.
            expect(fitAccount("assets:broker:taxable:vti", 0, px)).toBe("a:b:t:vti");
            expect(fitAccount("averyveryverylongsingleaccountname", 0, px)).toBe("averyveryverylongsingleaccountname");
        });
    });

    describe("shareWidths", () => {
        it("leaves everything at its natural width when the line is not full", () => {
            expect(shareWidths([100, 120], 400)).toEqual([100, 120]);
            expect(shareWidths([100, 120], 220)).toEqual([100, 120]);
        });

        it("REGRESSION: a short chip lends ALL its slack to a long one, and keeps none back", () => {
            // The old CSS capped each chip at 45% of the cell independently, so
            // on a 300px line the long name could never exceed 135px however
            // little the short one used.
            const [short, long] = shareWidths([100, 400], 300);

            expect(short).toBe(100); // untouched: it wanted 100 and 100 was cheap
            expect(long).toBe(200); // and every pixel it did not want went here
            expect(short + long).toBeCloseTo(300);
            expect(long).toBeGreaterThan(300 * 0.45);
        });

        it("splits evenly only between items that all want more than their share", () => {
            expect(shareWidths([200, 200, 400], 300)).toEqual([100, 100, 100]);
            // …and a cheap item is still satisfied first, whatever its neighbours want.
            expect(shareWidths([50, 400, 400], 450)).toEqual([50, 200, 200]);
        });

        it("survives a line with no room and items with no width", () => {
            expect(shareWidths([100, 200], 0)).toEqual([0, 0]);
            expect(shareWidths([], 300)).toEqual([]);
            expect(shareWidths([0, 0], -5)).toEqual([0, 0]);
        });
    });

    describe("categorize", () => {
        it("maps hledger-convention roots, singular or plural, any case", () => {
            expect(categorize("assets:bank:checking")).toBe("asset");
            expect(categorize("Asset:cash")).toBe("asset");
            expect(categorize("liabilities:card")).toBe("liability");
            expect(categorize("equity:opening")).toBe("equity");
            expect(categorize("revenues:salary")).toBe("revenue");
            expect(categorize("income:consulting")).toBe("revenue");
            expect(categorize("expenses:food")).toBe("expense");
            expect(categorize("virtual:budget")).toBe("other");
        });
    });
});
