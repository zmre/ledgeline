<script lang="ts">
    // The optional statement balance, and the account it is a balance of.
    //
    // Pre-filled when the source format volunteered one — OFX carries
    // `LEDGERBAL/BALAMT` inside the statement, so an OFX drop already knows the
    // closing balance and the user does not retype their statement. It is
    // carried as verbatim decimal TEXT the whole way: nothing on this side
    // parses it, because the reconciliation is the engine's arithmetic (done by
    // concatenating the journal and the proposed entries — two `-f` flags
    // silently give the wrong combined answer) and a browser-side float would
    // only be able to disagree with it.
    //
    // The account defaults to the chosen rules file's `account1`, which is the
    // account the imported postings actually land in — asserting a statement
    // balance against anything else is a check that is guaranteed to fail.
    import AccountInput from "$lib/journal/edit/AccountInput.svelte";
    import type {StatementMeta} from "../importTypes";

    let {
        balance,
        balanceAccount,
        statement,
        accountNames,
        writeAssertion,
        disabled,
        onBalance,
        onAccount,
        onWriteAssertion,
    }: {
        balance: string;
        balanceAccount: string;
        /** What the format volunteered, for the "as of" hint. Null when it volunteered nothing. */
        statement: StatementMeta | null;
        accountNames: string[];
        writeAssertion: boolean;
        disabled: boolean;
        onBalance: (value: string) => void;
        onAccount: (value: string) => void;
        onWriteAssertion: (value: boolean) => void;
    } = $props();

    const prefilled = $derived(statement?.ledgerBalance !== null && statement?.ledgerBalance !== undefined);
</script>

<section class="flex flex-col gap-3 rounded-box border border-base-content/10 p-3" aria-label="Statement balance" data-testid="imports-balance">
    <h2 class="text-sm font-semibold tracking-tight">Statement balance <span class="font-normal text-base-content/50">(optional)</span></h2>
    <p class="text-xs text-base-content/60">
        {#if prefilled}
            Taken from the statement itself{statement?.balanceAsOf === null || statement?.balanceAsOf === undefined ? "" : `, as of ${statement.balanceAsOf}`}.
            Ledgeline checks it against what your journal plus these transactions comes to, before writing anything.
        {:else}
            Type the closing balance from your statement and Ledgeline will check it against what your journal plus these transactions comes to.
        {/if}
    </p>

    <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
        <label class="form-control">
            <span class="label-text text-xs">Balance</span>
            <input
                type="text"
                class="input w-full font-mono input-sm"
                value={balance}
                {disabled}
                inputmode="decimal"
                spellcheck="false"
                autocomplete="off"
                placeholder="2945.05"
                data-testid="imports-balance-amount"
                oninput={(event) => onBalance(event.currentTarget.value)}
            />
        </label>
        <label class="form-control">
            <span class="label-text text-xs">of account</span>
            <!-- A function binding, not `bind:value={local}`: the value lives in
                 the store, and a local mirror would need an effect to stay in
                 step with the default the chosen rules file supplies. -->
            <AccountInput bind:value={() => balanceAccount, (value) => onAccount(value)} {accountNames} {disabled} placeholder="assets:bank:checking" />
        </label>
    </div>

    {#if balance.trim() !== ""}
        <label class="label cursor-pointer justify-start gap-2">
            <input
                type="checkbox"
                class="checkbox checkbox-sm"
                checked={writeAssertion}
                {disabled}
                data-testid="imports-write-assertion"
                onchange={(event) => onWriteAssertion(event.currentTarget.checked)}
            />
            <span class="label-text text-xs"> Write it into the journal as a balance assertion, so hledger re-checks it every time it reads the file. </span>
        </label>
    {/if}
</section>
