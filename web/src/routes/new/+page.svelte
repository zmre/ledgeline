<script lang="ts">
    // File → New (plan 39 Phase 6): the new-ledger wizard's page. The desktop
    // app opens it from File → New… and on a first run with no journal.
    //
    // Once a ledger is open the engine is already serving it, so where to go
    // next is all that is left. A fresh ledger reloads the app at the journal:
    // a full load, so no screen keeps anything it cached for the old one. The
    // import path stays in the page instead, because the wizard is still
    // holding the exported file the user chose and is handing it to Imports.
    import {goto} from "$app/navigation";
    import {resolve} from "$app/paths";
    import NewLedgerWizard from "#lib/onboarding/ui/NewLedgerWizard.svelte";

    function done(destination: "journal" | "import"): void {
        if (destination === "import") {
            // eslint-disable-next-line svelte/no-navigation-without-resolve -- resolve("/imports") IS the route id; the query string is appended
            void goto(`${resolve("/imports")}?tab=new`);
        } else {
            window.location.assign(resolve("/"));
        }
    }
</script>

<svelte:head><title>Ledgeline — New ledger</title></svelte:head>

<NewLedgerWizard onDone={done} onCancel={() => void goto(resolve("/"))} />
