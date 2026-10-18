// Test-only builders for engine-shaped account classes (`/api/journal`
// `accounts`). Imported ONLY by *.test.ts files — never by runtime code.
//
// The engine resolves every account the journal knows; a test stands in for it
// by writing down what it would answer. A `[name, type]` tuple is a DECLARED
// type (`typeDeclared: true`), the common case; the object form spells out the
// rest (an inferred type, a term, a guess).

import type {AccountClass, AccountClasses, AccountType, BsTerm} from "./accountClass";

export type ClassEntry =
    | readonly [name: string, type: AccountType | null]
    | {name: string; type: AccountType | null; typeDeclared?: boolean; term?: BsTerm | null; termGuessed?: boolean};

/** An engine-shaped classification table from `entries`. */
export function classesOf(...entries: ClassEntry[]): AccountClasses {
    return new Map(
        entries.map((entry): [string, AccountClass] => {
            if (Array.isArray(entry)) {
                const [name, type] = entry as readonly [string, AccountType | null];
                return [name, {type, typeDeclared: true, term: null, termGuessed: false}];
            }
            const {name, type, typeDeclared = true, term = null, termGuessed = false} = entry as Exclude<ClassEntry, readonly unknown[]>;
            return [name, {type, typeDeclared, term, termGuessed}];
        })
    );
}
