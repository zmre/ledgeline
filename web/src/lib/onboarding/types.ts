// File → New wizard (plan 39 Phase 6): the shapes the wizard works in. The
// wire shapes are the engine's `/api/new-ledger*` bodies, decoded in
// `decode.ts`; everything here is immutable.

/** Which starting chart of accounts. */
export type LedgerKind = "individual" | "business" | "empty";

/** The wizard's two paths. */
export type WizardPath = "fresh" | "import";

/** One step of the wizard. Which ones a run visits is `model.stepsFor`. */
export type WizardStep = "start" | "kind" | "source" | "details" | "accounts" | "create";

/** How the base currency's amounts are written; mirrors the engine's `Currency`. */
export type CurrencyStyle = Readonly<{
    commodity: string;
    side: "left" | "right";
    spaced: boolean;
    decimals: number;
    decimalMark: "period" | "comma";
}>;

/** One tag on an account line; `value` is "" for a value-less tag such as `clearing:`. */
export type AccountTag = Readonly<{name: string; value: string}>;

/** One `account` line the new chart will declare. */
export type TemplateAccount = Readonly<{name: string; tags: readonly AccountTag[]}>;

/** `GET /api/new-ledger`. */
export type NewLedgerStatus = Readonly<{
    /** The desktop app, which can pick a folder and open a journal. */
    desktop: boolean;
    /** A journal is open, so the wizard can offer to go back to it. */
    journalOpen: boolean;
}>;

/** The folder the native picker chose: a one-time id and its name, never a path. */
export type PickedFolder = Readonly<{id: string; name: string; empty: boolean}>;

/** `POST /api/new-ledger` / `/sample`: the folder's name and the files written there. */
export type CreatedLedger = Readonly<{folder: string; files: readonly string[]}>;

/** An app whose export the "Start from another app" path can bring in. */
export type SourceApp = Readonly<{
    id: string;
    label: string;
    /** The export format, as the user would recognise it. */
    format: string;
    /** File extensions (without the dot) that app's export has. */
    extensions: readonly string[];
    /** Where to find the export in that app. */
    howTo: string;
}>;

/** `POST /api/new-ledger`'s body. */
export type CreateLedgerBody = Readonly<{
    folderId: string;
    title: string;
    currency: CurrencyStyle;
    accounts: readonly TemplateAccount[];
}>;
