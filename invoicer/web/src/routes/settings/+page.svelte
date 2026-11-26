<!-- Settings (SETTINGS-PLAN §4): two tabs, `?tab=books|appearance`.

     These books: one form over the books' settings file and the payment-
     matching settings shared with Ledgeline. Save and Discard changes sit top
     right, once each, sticky, and only while something is unsaved; ⌘/Ctrl+S
     saves. Save asks before creating a settings file, confirms each change
     that affects new bookings with what it does, and refuses (listing the
     deposits) a deposit-payments change that would count them twice.
     Leaving with unsaved changes asks Save / Discard / Cancel (the shared
     guard). A change on disk refetches; under unsaved edits it says so.

     Appearance: the theme and text size, shared with Ledgeline and applied at
     once (no Save), with "Use for these books only" where the books can keep
     them.

     Getting here: the sidebar's foot, `g ,` and ⌘/Ctrl+,. The tabs move with
     ←/→ (Home, End) once one has focus. -->
<script lang="ts">
    import {onMount, tick} from "svelte";
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import {page} from "$app/state";
    import {registerKeys} from "#ui/keys/keymap.svelte.js";
    import AppearancePanel from "#ui/theme/AppearancePanel.svelte";
    import {onFilesChanged} from "#lib/changes.svelte.js";
    import BooksSettings from "#lib/components/settings/BooksSettings.svelte";
    import ConfirmDialog from "#lib/components/ConfirmDialog.svelte";
    import ConflictNotice from "#lib/components/ConflictNotice.svelte";
    import SaveDiscardDialog from "#lib/components/SaveDiscardDialog.svelte";
    import {settingLabel, settingsMessage} from "#lib/labels.js";
    import {checkSettings, fetchSettings, saveSettings} from "#lib/settingsApi.js";
    import {SettingsEditor} from "#lib/settingsEditor.svelte.js";
    import {unsavedGuard} from "#lib/unsavedGuard.svelte.js";
    import {uiPrefs} from "#lib/uiPrefs.svelte.js";

    type Tab = "books" | "appearance";
    const TABS: readonly {id: Tab; label: string}[] = [
        {id: "books", label: "These books"},
        {id: "appearance", label: "Appearance"},
    ];

    const tab = $derived<Tab>(page.url.searchParams.get("tab") === "appearance" ? "appearance" : "books");

    const editor = new SettingsEditor({
        fetch: fetchSettings,
        check: checkSettings,
        save: saveSettings,
        matching: () => uiPrefs.matching,
        saveMatching: (values) => uiPrefs.saveMatching(values),
    });

    const guard = unsavedGuard({
        dirty: () => editor.dirty,
        save: () => editor.save(),
        discard: () => editor.discard(),
        saveError: () => editor.saveError,
        conflict: () => editor.conflict !== null,
        noun: "settings file",
    });

    const showSave = $derived(tab === "books" && editor.dirty && !editor.readOnly);

    /** Switch tab: the same page, so the form (and its edits) stay. */
    async function choose(next: Tab, focus = false): Promise<void> {
        if (next !== tab) {
            // eslint-disable-next-line svelte/no-navigation-without-resolve -- the path is resolve()d; only the query is added
            await goto(`${resolve("/settings")}?tab=${next}`, {replace: true, reset: false});
        }
        if (focus) {
            await tick();
            document.getElementById(`settings-tab-${next}`)?.focus();
        }
    }

    function onTabKey(event: KeyboardEvent): void {
        const at = TABS.findIndex((t) => t.id === tab);
        const to =
            event.key === "ArrowRight"
                ? (at + 1) % TABS.length
                : event.key === "ArrowLeft"
                  ? (at + TABS.length - 1) % TABS.length
                  : event.key === "Home"
                    ? 0
                    : event.key === "End"
                      ? TABS.length - 1
                      : null;
        if (to === null) return;
        event.preventDefault();
        void choose(TABS[to].id, true);
    }

    async function save(): Promise<void> {
        await editor.save();
    }

    registerKeys({
        id: "settings-page",
        bindings: [
            {
                keys: "mod+s",
                label: "Save the settings",
                group: "Settings",
                always: true,
                run: () => {
                    if (tab === "books" && editor.canSave && editor.question === null) void save();
                },
            },
            {keys: "1", label: "These books", group: "Settings", run: () => void choose("books")},
            {keys: "2", label: "Appearance", group: "Settings", run: () => void choose("appearance")},
        ],
    });

    onMount(() => {
        void editor.load();
    });

    // Live reload: the settings file, and the shared prefs (a pin, another app's save).
    onFilesChanged(() => {
        void editor.refresh();
        void uiPrefs.load();
    });

    const question = $derived(editor.question);
</script>

<svelte:head>
    <title>Settings · Ledgeline Invoicer</title>
</svelte:head>

<div class="mx-auto flex max-w-6xl flex-col gap-4" data-testid="settings-page">
    <header
        class="sticky -top-3 z-10 -mx-3 flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-base-300 bg-base-100 px-3 py-2 sm:-top-4 sm:-mx-4 sm:px-4 md:-top-6 md:-mx-6 md:px-6"
    >
        <h1 class="text-2xl font-semibold tracking-tight">Settings</h1>
        <div role="tablist" class="tabs tabs-border" aria-label="Settings" tabindex="-1" onkeydown={onTabKey}>
            {#each TABS as t (t.id)}
                <button
                    type="button"
                    role="tab"
                    id="settings-tab-{t.id}"
                    class={["tab", t.id === tab && "tab-active"]}
                    aria-selected={t.id === tab}
                    aria-controls="settings-panel-{t.id}"
                    tabindex={t.id === tab ? 0 : -1}
                    onclick={() => void choose(t.id)}
                    data-testid="settings-tab-{t.id}">{t.label}</button
                >
            {/each}
        </div>
        {#if showSave}
            <div class="ml-auto flex flex-wrap items-center gap-2" data-testid="settings-actions">
                <span class="text-sm text-base-content/70">Unsaved changes</span>
                <button type="button" class="btn btn-ghost btn-sm" disabled={editor.saving} onclick={() => editor.discard()} data-testid="settings-discard"
                    >Discard changes</button
                >
                <button
                    type="button"
                    class="btn btn-primary btn-sm"
                    disabled={!editor.canSave}
                    title={editor.canSave ? undefined : "Fix the marked settings first"}
                    onclick={() => void save()}
                    data-testid="settings-save">{editor.saving ? "Saving…" : "Save"}</button
                >
            </div>
        {/if}
    </header>

    <div role="tabpanel" id="settings-panel-{tab}" aria-labelledby="settings-tab-{tab}" tabindex="-1" class="flex flex-col gap-4">
        {#if tab === "appearance"}
            <AppearancePanel
                theme={uiPrefs.theme}
                uiScale={uiPrefs.uiScale}
                onTheme={(id) => (uiPrefs.theme = id)}
                onScale={(percent) => (uiPrefs.uiScale = percent)}
                canPin={uiPrefs.canPin}
                isPinned={(key) => uiPrefs.isPinned(key)}
                pin={(key, pinned) => uiPrefs.pin(key, pinned)}
                pinLabel="Use for these books only"
            >
                {#snippet intro()}
                    <p class="text-sm text-base-content/70" data-testid="appearance-shared">
                        Shared with Ledgeline: a change here changes it there too{uiPrefs.canPin ? ", unless you keep it for these books only" : ""}. Changes
                        apply at once.
                    </p>
                {/snippet}
                {#snippet themePinnedNote()}These books have their own theme.{/snippet}
            </AppearancePanel>
            {#if uiPrefs.failure !== null}
                <div role="alert" class="alert alert-soft text-sm alert-error" data-testid="appearance-failure">{settingsMessage(uiPrefs.failure)}</div>
            {/if}
        {:else}
            {#if editor.loadError !== null}
                <div role="alert" class="alert alert-soft alert-error" data-testid="settings-load-error">
                    <span>{editor.loadError}</span>
                    <button type="button" class="btn btn-sm" onclick={() => void editor.load()}>Try again</button>
                </div>
            {:else if editor.page === null}
                <span class="loading loading-md loading-spinner" aria-label="Loading the settings"></span>
            {:else}
                {#if editor.conflict !== null}
                    <ConflictNotice
                        theirs={null}
                        what="settings file"
                        busy={editor.saving}
                        onReload={() => void editor.reloadTheirs()}
                        onKeepMine={() => void editor.keepMine()}
                    />
                {/if}
                {#if editor.saveError !== null}
                    <div role="alert" class="alert alert-soft alert-error" data-testid="settings-save-error">{settingsMessage(editor.saveError)}</div>
                {/if}
                {#if uiPrefs.failure !== null}
                    <div role="alert" class="alert alert-soft text-sm alert-error">{settingsMessage(uiPrefs.failure)}</div>
                {/if}
                {#if editor.savedNote !== null && !editor.dirty}
                    <div role="status" class="text-sm text-success" data-testid="settings-saved">{editor.savedNote}</div>
                {/if}
                <BooksSettings {editor} prefs={uiPrefs} />
            {/if}
        {/if}
    </div>
</div>

{#if question?.kind === "create"}
    <ConfirmDialog
        title="Create a settings file?"
        confirmLabel="Create it and save"
        busy={editor.saving}
        onConfirm={() => void editor.answer(true)}
        onCancel={() => void editor.answer(false)}
        testid="settings-create-file"
    >
        <p>These books have no settings file yet. Create it?</p>
        <p class="text-base-content/70">It goes beside your main journal, where Ledgeline keeps this books' own settings too.</p>
    </ConfirmDialog>
{:else if question?.kind === "confirm"}
    <ConfirmDialog
        title="Change how new bookings are made?"
        confirmLabel="Save these changes"
        tone="warning"
        busy={editor.saving}
        onConfirm={() => void editor.answer(true)}
        onCancel={() => void editor.answer(false)}
        testid="settings-confirm"
    >
        <ul class="flex list-disc flex-col gap-2 pl-5" data-testid="settings-effects">
            {#each question.effects as effect (effect.key)}
                <li><span class="font-medium">{settingLabel(effect.key)}:</span> {settingsMessage(effect.message)}</li>
            {/each}
        </ul>
    </ConfirmDialog>
{:else if question?.kind === "refused"}
    <ConfirmDialog
        title="This change can't be saved yet"
        confirmLabel="Undo this change"
        cancelLabel="Keep editing"
        onConfirm={() => {
            editor.revert(question.refusal.key);
            void editor.answer(false);
        }}
        onCancel={() => void editor.answer(false)}
        testid="settings-refused"
    >
        <p><span class="font-medium">{settingLabel(question.refusal.key)}:</span> {settingsMessage(question.refusal.message)}</p>
        <ul class="flex flex-col gap-1" data-testid="settings-refused-deposits">
            {#each question.refusal.deposits as d (d.number)}
                <li>Deposit invoice {d.number} for {d.customer}, {d.date}: {d.waiting} waiting</li>
            {/each}
        </ul>
    </ConfirmDialog>
{/if}

{#if guard.pending !== null}
    <SaveDiscardDialog
        message="Your settings have unsaved changes. Save them first?"
        saving={editor.saving}
        error={guard.error === null ? null : settingsMessage(guard.error)}
        onSave={() => void guard.save()}
        onDiscard={() => void guard.discard()}
        onCancel={() => void guard.cancel()}
    />
{/if}
