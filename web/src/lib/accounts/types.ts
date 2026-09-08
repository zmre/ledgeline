// The account-list editor's domain types (Settings → Accounts), decoded from
// the engine's `/api/accounts` wire by nativeDecode.ts.
//
// Every field mirrors `ledgeline-server/src/account_api.rs`'s wire types,
// the same rule `budget/types.ts` and `imports/importTypes.ts` state for
// their own routes.

/** One tag: `type: A`, `bsgroup: Cash`, or any tag of the user's own. */
export interface AccountTag {
    readonly name: string;
    readonly value: string;
}

/** One `account NAME  ; tags...` declaration. */
export interface AccountEntry {
    /** The file it is declared in, relative to the include root. */
    readonly journalId: string;
    /** 0-based position among that FILE's account lines — the handle a save names. */
    readonly index: number;
    /** 1-based line number in that file. */
    readonly line: number;
    readonly name: string;
    /** In the order the comment writes them. */
    readonly tags: readonly AccountTag[];
    /** The comment's free-text note, or empty. */
    readonly note: string;
}

/** One journal file's account declarations, and the revision a save must echo. */
export interface AccountFile {
    readonly journalId: string;
    readonly label: string;
    /** Echo this back in a save to prove the edit is against these bytes. */
    readonly revision: string;
    readonly writable: boolean;
    readonly accounts: readonly AccountEntry[];
}

/** `GET /api/accounts` — every account the open journal declares. */
export interface AccountListing {
    readonly editable: boolean;
    /** Whether `POST /api/accounts/file` would succeed. */
    readonly canCreateFile: boolean;
    /** The name that button would create. */
    readonly createFileName: string;
    readonly files: readonly AccountFile[];
}

/** `POST /api/accounts/file` — what was created. */
export interface CreatedAccountsFile {
    readonly journalId: string;
    readonly label: string;
    readonly includedAs: string;
    readonly mainJournalId: string;
}
