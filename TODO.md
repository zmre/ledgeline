# TODO


## Known issues
- fix: display issue where pie chart is not round, but oval when the window narrows horizontally or vertically.  Update: seems to be specific to linux as I can't reproduce on mac.
- chore: route bad `issection:` / `holdings:` / `valuation:` / `bsterm:` / `type:` tag values into Problems
  - a mistyped `issection:` currently fails the whole P&L request with a 400 naming the account and the
    valid codes, and `holdings:` now does the same to the Holdings tab. Right that it isn't silently
    dropped, wrong that one typo takes the tab down. Problems
    entries anchor to a `txnIndex` and an `account` directive has none, so this needs a wire field that
    allows a directive-anchored diagnostic plus an allow-list entry in the SPA's `normalize.ts`.

## Misc
- check security csrf to be sure other apps/browser pages can't fetch financial data
- chore: Add screenshots and better descriptions to the readme

## Performance
- test: lets try to understand performance on large repos by making a fixture with 10k transactions per year, 15 years, and around 200 commodities and 75 accounts

## Import improvements
- feat: quickbooks import handling
  - transaction matching and skipping
  - account mapping (prompt for unmapped) (aliases?)
- feat: import drag/drop
  - command line options
  - fix styling of numbers issues; infected the entire ui now
- feat: create new import rules files
  - take a csv file and make intelligent guesses on setup. we want intelligent mapping of headings, ask what account it is and default categorizations, figure out ordering of rows. detect separator, skip rows number, and encoding automatically. figure out date-format automatically. 

## Editors
- Account List Editor
  - Most financial apps allow editing of the chart of accounts. We should detect where they live and allow editing. If there aren't any, we should create an accounts.journal and include it from the main file.
  - For each account, we should provide an editor for comments/notes, type, tags in general, and our special tags used in various reports
  - Lets put this under a Settings top level tab or gear icon. And lets figure out what else might go in here, like the number format stuff -- basically whatever hledger provides that we might want to set or edit
    - commodity, decimal-mark, tag list, and we should probably move aliases to here under "settings" too
- Rules editor ui improvements
  - we need to figure out a new rules editor approach because the current one is ugly, hard to find what you're looking for, very long vertically and not scannable
  - also: we can't do more sophisticated rules (with conditional logic in them) so we need to add that and figure out ways to display and edit them
  - perhaps instead of one giant form, we have display separate from edit and can therefore make this nicer

## AI
- feat: private AI integration
  - Need to make use of a per-user preference specifying the url for the AI and any necessary api keys or whatever
  - Need to make a way for the user to edit this in the app
  - Only show an AI icon if we have a successful connection; or maybe there's an AI chat icon that has a red dot over it if the configured url doesn't work, a green dot if it is working, and the icon is hidden if this isn't setup (under settings tab)
  - clicking the chat slides over a drawer
  - we need to build out a system prompt with information about the files in the folder. i think we need to allow a tool call to fetch files in the folder, but nothing else. and a tool call that could write to files, but with strict user approval checks.
  - i have mostly used ai functionality on my hledger repos when i want to use an external file -- often a pdf -- and produce some specific journal entries from it such as balance checks or setting up a partnership or doing fair value / nav adjustments
  - ultimately, the AI should interact with the user through the chat drawer. the user should allow it to see specific files or all files as they desire. user should be able to clear history.  ai should be able to request reading of files and propose changes to files.
  - i don't want to reinvent too much here.  and i'd like to use our API endpoints rather than any direct writing. maybe that's what we offer as tools is our api endpoints, but with per-session user approval?  i mean, if the AI is private, it's probably fine to offer up the read-only endpoints. the main thing is to gate writes, not reads.

## Stocks
- feat: gain timeline improvements
  - when i change the gain timeline, everything else should update, too, notably the "value over time" which is fixed to previous 12 months
  - the gain timeline also needs more options. lets do 5yr, 3mo, 1mo, and 1 week as additions
- feat: in holdings tab, optional compare performance to S&P, Dow, Nasdaq, Bonds, US Stock Index
  - Idea is to be able to see how one's portfolio is doing versus just standard options where each can be checked or unchecked.
  - Our current line chart is value (dollars) over time and we need to overlay basically what it would have been like if every investment was made at the same time with the same money but into X as a separate line (dotted and different color) to show the comparison and difference in the chart.
  - We don't need this per stock and we don't need it as an overall percentage -- just the line chart.
- feat: more pie chart views of stocks
  - if yahoo gives us more than just price information then it might be nice to have a drop down on our stock holdings pie chart to show it divided up in a few ways: risk categorization, asset class, security type, industry, etc. -- whatever we can get -- then we show the pie chart instead of by investment, by category
  - we would need a way to have a tag on commodities that could specify (or override even) this information
  - if yahoo doesn't provide this information for free, then we'd need to see if there's another place that does. there's no server, so any sort of signup or account requirement is a blocker. we'll skip this feature if we don't have a data source.

## Ledger 
- feat: intelligent category suggestions
  - only real way to do this is with some sort of lookback comparing similar descriptions in the past and seeing associated expense or revenue accounts
  - need to remove random numbers from description and maybe do a predominance calculation or a vector comparison rather than full equality.  if we're doing equality and removing numbers, we need to normalize some by lowercasing.  but in a perfect world, "netflix.com" might see a previous "netflix" and guess category based on that.  the more exact the match and the more recent, the higher the sort ranking
  - feat: remember categorization functionality — write a chosen category back into the rules file as a new `if` rule (the rules editor and its write path are done; this is the one-click path into them)
- feat: File -> New
  - Here I'm assuming we're setting up a new set of journal files, chart of accounts, etc. Probably we prompt with some questions and use an empty folder as a starting point and then create a skeleton so someone can start using us to track things. We should have a default chart of accounts for individuals and another for businesses and then we should allow them to start with an empty set of accounts to add their own.
- feat: saved report filters?

## Planning / modeling
- feat: personal planning calculators a la quicken financial planner; see inspiration from [credit karma](https://www.creditkarma.com/calculators/money) and [nerdwallet](https://www.nerdwallet.com/investing/calculators)
  - great free tools with details at [engaging-data](https://engaging-data.com/early-retirement-calculators-and-tools/)
  - TODO: investigate [projection lab](https://projectionlab.com) to understand if that's worthwhile or anything there we want to learn from. from a friend: "really nice stuff built on top of it (roth conversions, drawdown simulation, flex spending, tax strategy, "what if" checkpointing to compare decisions, nice milestone tools to setup when costs are known to change and how, etc"

  - in business, I also build modeling spreadsheets that allow for what-ifs and let me model sales growth, investments, ramp up in hiring/expenses, etc., so i can understand runway (cash balance over time anyway)
    - It would be great if we had a mechanism for unifying this
  - we need to understand how forecasting works in hledger -- is it just the budgeting? can we use that for forecasts meaningfully?
    - partly answered by the budget editor (`plans/15-budget-editor.md`): hledger's `--forecast` reads the
      SAME `~` periodic rules `bal --budget` does, so `ledgeline-core/src/periodic.rs` — the span editor
      the budget tab writes through — is already the document model a forecast scenario would be written
      with. It is named `periodic` rather than `budget` for exactly that reason.
  - fix: our parser REJECTS every period expression outside the five fixed intervals — `~ every 2 weeks`,
    `~ monthly from 2026-01 to 2027-01` — and the rejection fails the WHOLE journal parse, not just the
    rule. Forecasts need those forms (a scenario is bounded by definition), and today a user who writes
    one cannot open their journal at all. `parse::parse_period_expr` + `model::PeriodExpr` are where it
    lives; `periodic::BlockLock::Period` already presents an unmodelled period read-only, so the editor
    side is ready for whatever the parser learns to read.
  - i want to be able to save different forecasts, probably as files not included in the main, but which we can read in and apply and then produce reports into the future (balance sheet, p&l, cash flow, etc).
  - we should be able to do things with percentages, too, so we can assume some percentage return on investments or increase in salaries, expenses, and so on.  Not sure if we'd have to calculate this and save it out or if we could bake it in using hledger's auto postings
  - I really want this feature to be a nice GUI but then something that persists and still works with hledger to the greatest extent possible.
  - We should be able to have pre-canned report templates where there are sliders that for indicating certain things and they produce the forecast files that we then show based on those values (which could be captured as comments if needed so we can edit and change easily later).
    - The idea here would be for a basic personal one with questions about how many years until retirement, how much invested into retirement accounts, assumptions on stock returns and inflation, assumptions on spending per year in retirement, taxes, what income streams will continue (dollar amount and adjustment per year) in retirement, etc.
    - There would be different questions for business ones and other ideas, but the core here is that a user could do absolutely anything, but we make it really simple to setup some basic forecast scenarios.
