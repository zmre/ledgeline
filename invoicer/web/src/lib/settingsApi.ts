// The Settings page's requests (SETTINGS-PLAN §5). A refusal is an
// EditorApiError with the server's `code`: `changed` (with the current
// revision), `no-file`, `unusable`, `confirm` (with `effects`), `invalid`
// (with `problems`) and `refused` (with `refused`).
import {EditorApiError, send} from "./editorApi.js";
import type {Effect, Refusal, SettingProblem, SettingsChange, SettingsCheck, SettingsPage, SettingsSaved} from "./settings.js";

const isObject = (body: unknown): body is Record<string, unknown> => typeof body === "object" && body !== null && !Array.isArray(body);

/** `GET /api/settings`: the form, as the file is now. */
export function fetchSettings(): Promise<SettingsPage> {
    return send<SettingsPage>("GET", "/api/settings", (b) => isObject(b) && typeof b.file === "string" && isObject(b.values));
}

/** `POST /api/settings/check`: what Save would do. Writes nothing. */
export function checkSettings(change: SettingsChange): Promise<SettingsCheck> {
    return send<SettingsCheck>("POST", "/api/settings/check", (b) => isObject(b) && Array.isArray(b.problems) && Array.isArray(b.effects), change);
}

/** `PUT /api/settings`: write the change. */
export function saveSettings(change: SettingsChange): Promise<SettingsSaved> {
    return send<SettingsSaved>("PUT", "/api/settings", (b) => isObject(b) && typeof b.revision === "string", change);
}

/** What a refused save carries, read off its error. */
export const effectsOf = (error: EditorApiError): Effect[] => (Array.isArray(error.body.effects) ? (error.body.effects as Effect[]) : []);
export const problemsOf = (error: EditorApiError): SettingProblem[] => (Array.isArray(error.body.problems) ? (error.body.problems as SettingProblem[]) : []);
export const refusalOf = (error: EditorApiError): Refusal | null => (isObject(error.body.refused) ? (error.body.refused as unknown as Refusal) : null);

/** The books' settings file, from the books folder (what "Open the settings file" opens). */
export const SETTINGS_FILE = "ledgeline.toml";
