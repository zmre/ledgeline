import prettier from "eslint-config-prettier";
import path from "node:path";
import js from "@eslint/js";
import svelte from "eslint-plugin-svelte";
import {defineConfig, includeIgnoreFile} from "eslint/config";
import globals from "globals";
import ts from "typescript-eslint";

const gitignorePath = path.resolve(import.meta.dirname, ".gitignore");

export default defineConfig(
    includeIgnoreFile(gitignorePath),
    js.configs.recommended,
    ts.configs.recommended,
    svelte.configs.recommended,
    prettier,
    svelte.configs.prettier,
    {
        languageOptions: {globals: {...globals.browser, ...globals.node}},
        rules: {
            // typescript-eslint strongly recommend that you do not use the no-undef lint rule on TypeScript projects.
            // see: https://typescript-eslint.io/troubleshooting/faqs/eslint/#i-get-errors-from-the-no-undef-rule-about-global-variables-not-being-defined-even-though-there-are-no-typescript-errors
            "no-undef": "off",
        },
    },
    {
        files: ["**/*.svelte", "**/*.svelte.ts", "**/*.svelte.js"],
        languageOptions: {
            parserOptions: {
                projectService: true,
                extraFileExtensions: [".svelte"],
                parser: ts.parser,
            },
        },
    },
    {
        rules: {
            // Dates in this codebase are ISO "YYYY-MM-DD" strings compared lexically.
            // `new Date(someString)` parses to UTC midnight, so reading it back through
            // local getters lands on the PREVIOUS day in every negative-offset zone.
            // Five separate source comments forbid this; this makes it enforceable.
            "no-restricted-syntax": [
                "error",
                {
                    selector: 'NewExpression[callee.name="Date"] > Literal[value=type(string)]',
                    message:
                        'Never `new Date("YYYY-MM-DD")` — it parses as UTC midnight and shifts a day in negative-offset zones. Use the string/integer date math in $lib/reports/periods, or `new Date(Date.UTC(y, m - 1, d))` read back via getUTC* getters.',
                },
                {
                    selector: 'NewExpression[callee.name="Date"] > TemplateLiteral',
                    message:
                        'Never build a Date from a template string — same UTC-parse hazard as `new Date("YYYY-MM-DD")`. Use $lib/reports/periods, or `new Date(Date.UTC(y, m - 1, d))` read back via getUTC* getters.',
                },
            ],
        },
    }
);
