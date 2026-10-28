// Settings › hledger config: the domain shape of `GET /api/hledger-conf`
// (crates/ledgeline-server/src/conf_api.rs `WireConf`), decoded by
// `nativeDecode.ts::decodeHledgerConf`.

/** The four report commands whose `--depth` Ledgeline honours, by hledger's full (section) name. */
export type DepthCommand = "balancesheet" | "incomestatement" | "cashflow" | "balance";

/** A config-file section this screen edits: the general one, or a report command's. */
export type ConfSection = "general" | DepthCommand;

export const DEPTH_COMMANDS: readonly DepthCommand[] = ["balancesheet", "incomestatement", "cashflow", "balance"];
export const CONF_SECTIONS: readonly ConfSection[] = ["general", ...DEPTH_COMMANDS];

/** One `--depth ACCOUNTREGEX=N`. */
export interface RegexDepth {
    readonly regex: string;
    readonly depth: number;
}

/** One section's `--depth` options: a plain limit (`--depth N`) and per-account ones. */
export interface DepthSpec {
    readonly all: number | null;
    readonly perRegex: readonly RegexDepth[];
}

export interface ConfWarning {
    /** `aliasSection` | `duplicateSection` | `badDepth`. */
    readonly code: string;
    readonly message: string;
}

/** Which tier of hledger's lookup found the file. */
export type ConfOrigin = "ledger" | "home" | "xdg";

export interface HledgerConf {
    /** `hledger.conf`, `../hledger.conf`, `~/.hledger.conf` … — never an absolute path; null when none is in force. */
    readonly handle: string | null;
    readonly origin: ConfOrigin | null;
    /** Where a first write creates one, relative to the journal's directory; null when a file is in force or none can be created. */
    readonly createHandle: string | null;
    readonly editable: boolean;
    readonly readOnlyReason: string | null;
    /** The engine's sentence for a file whose first word replaces the command; null normally. */
    readonly hijacked: string | null;
    /** The revision a save must echo ("" for no file). */
    readonly revision: string;
    readonly auto: boolean;
    /** Each section's options as written there. */
    readonly depth: Readonly<Record<ConfSection, DepthSpec>>;
    /** What hledger applies to each report: general, then its own section. */
    readonly effectiveDepth: Readonly<Record<DepthCommand, DepthSpec>>;
    readonly warnings: readonly ConfWarning[];
}

/** `PUT /api/hledger-conf`. Absent fields are left as they are; a named section's depth becomes exactly what is sent. */
export interface SaveHledgerConfBody {
    revision: string;
    auto?: boolean;
    depth?: Partial<Record<ConfSection, {all: number | null; perRegex: RegexDepth[]}>>;
}
