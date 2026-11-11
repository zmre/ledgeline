// The (i) help on Insights: one entry per box, list, "More metrics" row, MRR
// bridge category and segment-table column, keyed by the id the component
// renders it under. Acronyms are spelled out in the title, and each formula is
// the one the engine computes (crates/ledgeline-core/src/reports/saas.rs and
// insights.rs; docs/reports/insights.md and startup-metrics.md say the same in
// prose). Change a formula here only alongside the engine.

/** One help entry, spread straight into `InfoTip`. */
export interface MetricHelp {
    /** The short name, for the button's "About <label>" label. */
    label: string;
    /** The first line: the name, with any acronym spelled out. */
    title: string;
    /** One or two plain sentences on what it tells you. */
    text: string;
    /** How it is computed, when that isn't obvious from the text. */
    formula?: string;
}

export const METRIC_HELP = {
    // The six headline boxes, and Runway in Net Worth's slot.
    revenue: {
        label: "Revenue",
        title: "Revenue",
        text: "Everything you earned over the current period. The line below compares it with the previous period.",
        formula: "total of revenue accounts over the period",
    },
    expenses: {
        label: "Expenses",
        title: "Expenses",
        text: "Everything you spent over the current period. The line below compares it with the previous period.",
        formula: "total of expense accounts over the period",
    },
    networth: {
        label: "Net Worth",
        title: "Net Worth",
        text: "What you own minus what you owe, at the end of the current period, in your base currency.",
        formula: "assets − liabilities, at period end",
    },
    costofliving: {
        label: "Avg Monthly Costs",
        title: "Avg Monthly Costs",
        text: "What a typical month cost you. Accounts under expenses:tax and expenses:taxes are left out.",
        formula: "expenses ÷ months in the period",
    },
    investment: {
        label: "Investment Performance",
        title: "Investment Performance",
        text: "How much your holdings' market value changed over the current period. Shares bought during it count from zero. The small line is the previous period's own figure.",
        formula: "value at end − value at start; % = that ÷ value at start",
    },
    cash: {
        label: "Cash Balance",
        title: "Cash Balance",
        text: "The total in your cash accounts at the end of the current period.",
        formula: "balance of type: C accounts, at period end",
    },
    runway: {
        label: "Runway",
        title: "Runway",
        text: "Months your cash lasts at the recent burn. With deferred revenue or receivables, burn is adjusted: as if customers paid when served, so an annual prepayment doesn't read as a windfall.",
        formula: "cash ÷ 3-month average burn; adjusted burn = cash burn + change in deferred revenue − change in receivables",
    },

    "closed-through": {
        label: "Closed through",
        title: "Closed through",
        text: "Startup metrics stop at the last month with recurring revenue recorded, so a month whose revenue recognition isn't booked yet doesn't read as churn.",
        formula: "the last month with any kpi: recurring posting on or before the span's end",
    },

    // The startup boxes.
    arr: {
        label: "ARR",
        title: "ARR — Annual Recurring Revenue",
        text: "Subscription revenue as a yearly rate, from the latest complete month. MRR (Monthly Recurring Revenue) is that month's figure. Usage revenue is shown beside it, not counted.",
        formula: "ARR = MRR × 12; MRR = kpi: recurring revenue that month",
    },
    netnewarr: {
        label: "Net new ARR",
        title: "Net new ARR",
        text: "ARR added over the period, after churn and contraction. CMGR (Compounded Monthly Growth Rate) is the steady monthly growth that takes MRR from start to end.",
        formula: "12 × (MRR at period end − MRR the month before it began)",
    },
    customers: {
        label: "Customers",
        title: "Customers",
        text: "Customers paying recurring revenue in the latest complete month. Growth compares with the month before the period. * means some were inferred from payees.",
        formula: "count of customers with MRR above zero",
    },
    "customers-billings": {
        label: "Customers (from billings)",
        title: "Customers (from billings)",
        text: "Customers billed in the last 12 months. Shown because per-customer MRR isn't available: customer tags are on invoices, not revenue. A customer lapses after 12 months without a bill.",
        formula: "customers with a billing in the 12 months to the latest complete month",
    },
    nrr: {
        label: "NRR",
        title: "NRR — Net Revenue Retention",
        text: "What last year's customers pay now, as a share of what they paid then. Over 100% means expansion beat churn. GRR (Gross Revenue Retention) caps each at last year's amount.",
        formula: "Σ MRR now of customers paying 12 months ago ÷ Σ their MRR then",
    },
    burnmultiple: {
        label: "Burn multiple",
        title: "Burn multiple",
        text: "Burn per dollar of net new ARR. Under 1 is very efficient, 1 to 2 good, over 2 to 3 suspect, over 3 bad. Uses adjusted burn with deferred revenue or receivables, else cash burn.",
        formula: "adjusted burn = cash burn + Δ deferred revenue − Δ receivables",
    },
    grossmargin: {
        label: "Gross margin",
        title: "Gross margin",
        text: "The share of revenue left after the direct cost of delivering it. Cost of goods sold (COGS) is accounts tagged issection: cogs.",
        formula: "(revenue − COGS) ÷ revenue",
    },

    // "More metrics" rows, keyed by `moreMetrics`' keys.
    "acv-billings": {
        label: "ACV (from billings)",
        title: "ACV — Annual Contract Value, from billings",
        text: "Average yearly billing per customer, when per-customer MRR isn't available.",
        formula: "billings over the last 12 months ÷ customers billed in them",
    },
    "logo-churn-billings": {
        label: "Logo churn (from billings)",
        title: "Logo churn, from billings",
        text: "The share of billed customers who lapsed during the period: 12 months without a bill.",
        formula: "lapsed customers ÷ customers billed in the 12 months before the period",
    },
    arpa: {
        label: "ARPA",
        title: "ARPA — Average Revenue Per Account",
        text: "Average MRR per paying customer in the latest complete month.",
        formula: "MRR from attributed customers ÷ customers",
    },
    acv: {
        label: "ACV",
        title: "ACV — Annual Contract Value",
        text: "Average yearly value of the customers won this period, from their first month. A prorated first month reads low.",
        formula: "average over new customers of 12 × first month's MRR",
    },
    "logo-churn": {
        label: "Logo churn",
        title: "Logo churn",
        text: "The share of customers lost during the period.",
        formula: "customers lost ÷ customers in the month before the period",
    },
    "quick-ratio": {
        label: "Quick ratio",
        title: "Quick ratio",
        text: "MRR gained for each dollar of MRR lost. Above 1, you gained more than you lost.",
        formula: "(new + expansion + reactivation) ÷ (contraction + churn)",
    },
    "top-customers": {
        label: "Top customer share",
        title: "Top customer share",
        text: "How concentrated revenue is: the share of MRR from your largest 1, 5 and 10 customers, in the latest complete month.",
    },
    cac: {
        label: "CAC",
        title: "CAC — Customer Acquisition Cost",
        text: "Sales and marketing spend per new customer. Blended: all of it, including spend that drove expansion, with no lag.",
        formula: "S&M spend ÷ new customers, over the period",
    },
    "cac-payback": {
        label: "CAC payback",
        title: "CAC payback",
        text: "Months of gross profit from new and expansion MRR it takes to earn back sales and marketing spend.",
        formula: "S&M ÷ ((new + expansion) ÷ months × gross margin)",
    },
    "sm-pct": {
        label: "S&M %",
        title: "S&M — Sales & Marketing",
        text: "Sales and marketing spend as a share of revenue.",
        formula: "kpi: sales-marketing spend ÷ total revenue, over the period",
    },
    "rnd-pct": {
        label: "R&D %",
        title: "R&D — Research & Development",
        text: "Engineering, product and design spend as a share of revenue.",
        formula: "kpi: rnd spend ÷ total revenue, over the period",
    },
    "ga-pct": {
        label: "G&A %",
        title: "G&A — General & Administrative",
        text: "Overhead spend as a share of revenue.",
        formula: "kpi: ga spend ÷ total revenue, over the period",
    },
    "rule-of-40": {
        label: "Rule of 40",
        title: "Rule of 40",
        text: "Growth plus cash margin; 40 or more is the usual bar. It assumes annual growth, so read it on Year over year.",
        formula: "revenue growth % vs the same months before + free cash flow ÷ revenue %; FCF = −net burn",
    },
    dso: {
        label: "DSO",
        title: "DSO — Days Sales Outstanding",
        text: "How many days, on average, customers take to pay you.",
        formula: "receivables at month end ÷ revenue over the period × days in the period",
    },
    dpo: {
        label: "DPO",
        title: "DPO — Days Payable Outstanding",
        text: "How many days, on average, you take to pay suppliers.",
        formula: "payables at month end ÷ expenses over the period × days in the period",
    },
    "deferred-revenue": {
        label: "Deferred revenue",
        title: "Deferred revenue",
        text: "Money billed or received for service not yet delivered. It becomes revenue as you deliver.",
        formula: "kpi: deferred-revenue balance at the latest complete month end",
    },
    billings: {
        label: "Billings",
        title: "Billings",
        text: "What you invoiced over the period, earned or not.",
        formula: "revenue + change in deferred revenue, over the period",
    },

    // MRR bridge categories, keyed `bridge-<category>`.
    "bridge-new": {
        label: "New",
        title: "New MRR",
        text: "MRR from customers paying for the first time ever.",
        formula: "their first month's MRR, summed over the period's months",
    },
    "bridge-expansion": {
        label: "Expansion",
        title: "Expansion MRR",
        text: "Existing customers paying more than the month before.",
        formula: "the increase, summed over the period's months",
    },
    "bridge-contraction": {
        label: "Contraction",
        title: "Contraction MRR",
        text: "Existing customers paying less than the month before, but still paying.",
        formula: "the decrease, summed over the period's months",
    },
    "bridge-churn": {
        label: "Churn",
        title: "Churned MRR",
        text: "Customers who stopped paying, counted in the first month their MRR is zero or below. There is no grace period.",
        formula: "their MRR the month before, summed over the period's months",
    },
    "bridge-reactivation": {
        label: "Reactivation",
        title: "Reactivation MRR",
        text: "Customers who paid before, stopped, and came back.",
        formula: "their MRR on return, summed over the period's months",
    },
    "bridge-other": {
        label: "Other",
        title: "Other",
        text: "What the customer lines don't explain: unattributed revenue, and credits that take a customer's month below zero. It keeps the bridge adding up.",
        formula: "end − start − (new + expansion − contraction − churn + reactivation)",
    },

    // Segment-table columns.
    "segment-arpa": {
        label: "ARPA",
        title: "ARPA — Average Revenue Per Account",
        text: "Average MRR per paying customer in this segment, in the latest complete month.",
        formula: "segment MRR from attributed customers ÷ segment customers",
    },
    "segment-nrr": {
        label: "NRR",
        title: "NRR — Net Revenue Retention",
        text: "What this segment's customers from 12 months ago pay in it now, as a share of what they paid then.",
        formula: "Σ segment MRR now of customers paying 12 months ago ÷ Σ their MRR then",
    },

    // The four lists.
    expensechanges: {
        label: "Biggest Expense Changes",
        title: "Biggest Expense Changes",
        text: "Expense accounts whose totals moved most against the previous period, ranked by the move in money. Moves under 10 in your base currency are ignored; accounts with no previous activity aren't listed.",
    },
    revenuechanges: {
        label: "Biggest Revenue Changes",
        title: "Biggest Revenue Changes",
        text: "Revenue accounts whose totals moved most against the previous period, ranked by the move in money. Moves under 10 in your base currency are ignored; accounts with no previous activity aren't listed.",
    },
    movers: {
        label: "Biggest Movers",
        title: "Biggest Movers",
        text: "Your stocks ranked by percent move over the current period, with the gain in money. * means no price before the period, so it's measured from purchase cost.",
    },
    toptxns: {
        label: "Top Transactions",
        title: "Top Transactions",
        text: "This period's transactions with the biggest income or expense impact. Payments, transfers, moves to deferred revenue and closing entries drop out.",
        formula: "the larger of the debit and credit sides of its revenue and expense postings",
    },
} as const satisfies Record<string, MetricHelp>;

/** Every id with help. */
export type MetricId = keyof typeof METRIC_HELP;
