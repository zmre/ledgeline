# 32 — Phase 3 contract amendments

These belong under "Contract amendments made during implementation" in
`32-transaction-review.md`. Fold them in when the branches meet.

1. **The overlay is opened through a small store.** `review/ui/launch.svelte.ts`
   (`reviewLaunch`) holds which queue is open, and the Journal page mounts
   `ReviewOverlay` while it is set. The same store holds:
   - the payee and tag vocabularies (D15), loaded on open and again when the
     generation changes;
   - a one-shot `returnTo` request. The table consumes it, as it does the
     problems drawer's `focusRequest`, to put its cursor on the last card saved
     or ignored.
2. **Focus on close.** `dismissible` returns DOM focus to whatever opened the
   overlay: the button, or the page for `u`/`r`. The journal's cursor is
   visual, not DOM focus, so "focus on the row" means the cursor moves there.
3. **Escape on a dirty card** shows an inline bar with **Keep editing** and
   **Discard and close**. A second Escape keeps editing.
4. **Keys.**
   - `?` is bound inside the overlay, because its modal layer hides the global
     binding.
   - Single keys are off while the help sheet or the close bar is up.
   - **↵** in card mode clicks the focused overlay button or checkbox if there
     is one. Otherwise it does what ⌘↵ does.
   - **Space** is bound only while Similar is open.
   - `TransactionModal`'s overlay layer is a new `keys/ModalKeyLayer.svelte`,
     rendered inside `{#if txnModal.open}`. A modal layer blocks everything
     beneath it, so it must exist only while the popup is open. It rebinds `?`
     too.
5. **Suggestion chips.**
   - **Strong top chip:** pre-selected on an unknown card.
   - **Only weak chips:** none is pre-selected. The first Tab selects the top
     chip and fills it in.
   - **"usually X" chip** (a categorised Recents card): never pre-selected, so
     ⌘⇧↵ cannot recategorise such a card unless you Tab to the chip first.
   - Suggestions for an uncategorised account are dropped.
   - Accepting a chip also applies its rename (D13a), unless the card has
     already been renamed.
   - Tab is caught in the capture phase on the row's wrapper. `AccountInput` is
     unchanged.
6. **Similar on a clean card.** The section appears whenever the card has
   candidates, so a clean card shows its count, as in the mockup. Until
   something applicable changes, the checkboxes are disabled and a hint says
   why. Once something changes, the list is filtered to the rows that change
   applies to. The section is absent when there are no candidates.
7. **Drafts and saves.** Before each save, the overlay stores the form as the
   card's draft, so a failed save rewinds with the edits intact. The store only
   keeps drafts it is handed. A card refreshed underneath it (D5a, or a
   revalidation) is reseeded unless the user has edited it. Those edits are
   compared with the text they were made on.
8. **Motion (D29).**
   - **Built as specified:** Svelte `in:`/`out:` with the pure transitions in
     `review/ui/motion.ts`, and `{#key}` on the card. Any keystroke calls
     `finishMotion` first. Reduced motion is a 60ms fade, and the stamp becomes
     a success-coloured border.
   - **Attributes for tests:** `data-exit` and `data-motion` on the stack.
   - **The ✓ stamp** is a rubber-stamp badge, rendered with `flushSync`
     immediately before the card leaves.
   - **Not built:** the 40ms flash of the chip into the field, and the counter's
     number roll. The progress bar does ease over 300ms.
   - **Footer swap:** the new state fades in over 80ms. There is no two-way
     cross-fade.
9. **D15 deferral.** Picking a payee does not yet suggest its category. There
   is no per-payee precedent route. Completing a description keeps any
   `| note` the user already typed.
10. **The hero shows the funding account's full path.** The client has no
    notion of an account display name.
11. **Settings.**
    - The storage key keeps its version. The three new fields are checked when
      the settings load, as the existing fields are: the limit is clamped to
      1–1000, the window to 0–36500, and anything invalid falls back to its
      default.
    - Every summary request now sends `today`, `reviewLimit` and
      `reviewMaxDays`.
    - Changing the limit or the window calls the new
      `journalView.refreshSummary()`.
12. **Row stripes.**
    - The window test applies to the amber unknown stripe too, and no future
      row is marked. That matches the D4 queues.
    - The cursor's rule moved out of its class and into `rowShadow` (the
      `--row-shadow` variable). It stacks inside the stripe as a second inset
      layer.
    - Rows carry a `data-review-marker` attribute.
13. **Leaving the overlay.** **Open in full editor** and a generated posting's
    rule link both close the overlay first, and any unsaved edits on the card
    are dropped. **Ignore** saves only the tag, so unsaved edits on that card
    are dropped too. **Edit date** is in the ⋯ menu (D8).
14. **The e2e engine runs on a scratch copy.** It is on `:5098`, and its
    command stages `target/e2e-review/` from `fixtures/review/` on every run. The
    spec saves, and a save must never rewrite a committed fixture. The spec
    reads the copy's bytes directly. No route returns raw text.
15. **The hero's sign and amount follow the net on balance-sheet accounts
    (Patrick's review, 2026-10-30).**
    - It used to read the transaction's mix of account types, so a stock sale
      with a broker fee showed "−".
    - Now the hero sums the balance-sheet postings per commodity. Priced
      (`@`/`@@`) legs are left out whenever any other balance-sheet leg
      exists, so a sale reads as the cash it brought in.
      - Positive → in (+), negative → out (−), zero or mixed → transfer.
      - A paycheck shows what landed, net of withholding.
      - A card refund is money in.
16. **Key hints separate modifiers with a dash on macOS** (`⌘-Shift-Enter`)
    and with `+` elsewhere. `formatKeys` used to run them together on macOS
    (`⌘ShiftEnter`, `Ctrld`), which also affected the help sheet.
17. **Visual tweaks from Patrick's review.**
    - No coloured accent on the card: the amber unknown bar and the primary
      focus ring are gone.
    - The overlay is `max-w-4xl` with roomier padding and a larger hero.
    - The suggestion chips get more space above them.
    - The toolbar's review buttons share one lighter deep-gray style, and the
      ▾ is drawn as part of the Recents button with explicit corners, not
      `join`, which the tooltip wrapper broke.
18. **The page got lighter, not the buttons** (this corrects the toolbar
    point in the visual-tweaks amendment).
    - The dark theme's base surfaces are overridden in `app.css`:
      - `base-100` #1d232a → #262b30, `oklch(28.5% 0.012 254)`: a deep,
        less blue grey;
      - `base-200` → #1d2126;
      - `base-300` → #161a1f.
    - `base-200` is lifted less, so default buttons, inputs and the header
      sit clearly below the page.
    - The review buttons are plain `btn btn-sm` again, with no divider on the
      ▾.
    - The chart contrast floor was re-checked on the new surface: every slot is
      still ≥ 3:1, and the worst is `--chart-1` at 3.09:1, down from 3.41:1.
      CVD ΔE between slots doesn't depend on the surface.
    - Both palette docs (`app.css`, `palette.ts`) now say so.
18. **The overlay is a card deck (Patrick's review, 2026-10-30: "I'm not
    seeing animations").** This replaces item 8's motion and D29's table.
    D29's rules still hold: transforms only, no blocking, keys finish the
    motion first.
    - **Why nothing showed.** The exits moved about 40px over 140–160ms, and the
      card sat inside daisyUI's `overflow-hidden` `.modal-box` and an
      `overflow-y-auto` scroller, so it could never leave the dialog.
    - **Layout.** A `fixed inset-0` layer over a `bg-black/60` backdrop. That
      layer is the only thing that clips. Above it floats a slim top bar
      (queue, N of M, progress, the saving dot, ⋯, ✕) in light text, with no
      box behind it. Below the bar, the deck is a `max-w-4xl` grid with every
      card in one cell.
      - Each card is self-contained: hero, fields, Similar, and its own
        footer. The dirty-close bar takes the footer's place inside the
        card.
      - The card's body scrolls inside it, under
        `max-height: calc(100dvh - 7.5rem)`.
      - The daisyUI `modal` classes are gone.
    - **Peeks.** The next two queue items (`cards[cursor+1..cursor+2]`) wait
      beneath the top card. They are `inert` and `aria-hidden`, show only the
      hero (`ReviewHero peek`, which carries no test ids), and are stretched
      to the top card's height.
      - **Pinned at the bottom edge** (`transform-origin: bottom`), so the
        shrink and the drop leave a 14px and a 28px strip showing. A
        centre origin would hide them.
      - **Dimmed with `filter: brightness(0.6 / 0.38)`, not opacity ~0.55 /
        ~0.25.** A translucent card shows the hero of the card behind it
        through its own, and the incoming card would ghost the next peek
        while it rises.
      - **No peek under the last card.** The summary lies there and rises
        out of the peek slot as the last card leaves.
    - **The outgoing card.** Svelte marks an outroing element `inert`, and the
      overlay's scoped CSS keys off that attribute:
      - `.deck-card[inert]` goes `position: absolute` at the top of the cell,
        so the cell, and the peeks stretched to it, take the incoming card's
        height at once.
      - Its z-index is 6, above the incoming card's 5. On ← it is 3, below
        the card coming back and above the peeks.
      - None of this applies while the layer closes (`data-closing` on the
        root), so nothing jumps during the fade.
    - **The card transitions are `|global`**, so the last card still flies
      off when the summary replaces it. The `{#key}` wraps an `{#if}`, and a
      local outro would not play there. The moment `"close"` makes them do
      nothing while the layer fades.
    - **No Svelte `delay` on any card.** Easing is computed inside `css`.
      The Reviewed hold (the stamp's 90ms) is part of one animation, because
      a delayed Svelte transition is two animations, and `finishMotion`
      would only finish the first. The peek's opening beat is the one
      `delay`, and it only fades.
    - **A card change no keystroke caused** (revalidation, a failed save's
      rewind) uses the last moment's motion. A rewind after a Reviewed
      therefore flies the current card right again.
    - **Reduced motion is a 120ms cross-fade, up from 60.** At 60ms a fade
      is all but invisible, and something should show that the card
      changed. The pile still shows, at rest, and peeks change slot in
      place. The stamp is still the green border.
    - **Keys.** `h` (previous card) and `l` (skip) sit beside ←/p and →/n,
      each with its own row in the `?` sheet. The D16 table lists them.
    - **E2e.** The two footer lookups go through `cardOf(page, n)`, because
      during an exit the outgoing card has a footer too.

    | Moment            | Element          | Motion                                                                                                    | Duration                          | Easing           |
    | ----------------- | ---------------- | --------------------------------------------------------------------------------------------------------- | --------------------------------- | ---------------- |
    | Open              | backdrop         | opacity 0 → 1                                                                                             | 180ms                             | linear           |
    | Open              | top bar          | `translateY(-8px)` → 0, opacity 0 → 1                                                                     | 180ms                             | cubicOut         |
    | Open              | first card       | `translateY(24px) scale(.97)`, opacity 0 → identity                                                       | 180ms                             | cubicOut         |
    | Open              | peeks            | opacity 0 → 1, after a 120ms beat                                                                         | 220ms                             | cubicOut         |
    | Reviewed          | ✓ stamp          | `scale(.6 → 1)`, overshooting                                                                             | 90ms                              | backOut          |
    | Reviewed          | outgoing         | holds 90ms, then `translate(120vw, -8vh) rotate(12deg)`                                                   | 90 + 280ms                        | quadIn           |
    | Saved (unknown)   | outgoing         | `translate(120vw, -8vh) rotate(12deg)`                                                                    | 280ms                             | quadIn           |
    | Skip              | outgoing         | `translate(-120vw) rotate(-8deg)`, `saturate(1 → .3)`                                                     | 260ms                             | quadIn           |
    | Ignore            | outgoing         | `translateY(105vh) rotate(4deg)`                                                                          | 300ms                             | quadIn           |
    | Any forward exit  | new top card     | peek pose `translateY(14px) scale(.95) brightness(.6)` → identity (holds 90ms after a Reviewed)           | 220ms (+90)                       | cubicOut         |
    | Any forward exit  | old peek of it   | opacity 1 → 0, under the rising card                                                                      | 220ms                             | linear           |
    | Any move          | peeks            | slot to slot, `28px/.9/.38` → `14px/.95/.6` (a CSS transition)                                            | 220ms                             | cubic-bezier(.215,.61,.355,1) |
    | Any move          | new deepest peek | opacity 0 → 1                                                                                             | 220ms                             | cubicOut         |
    | Back (← / p / h)  | previous card    | `translate(-120vw) rotate(-8deg)` → identity, on top                                                      | 280ms                             | cubicOut         |
    | Back              | current card     | identity → peek pose, opacity 1 → 0, beneath                                                              | 220ms                             | cubicOut         |
    | Last card leaves  | summary          | rises from the peek pose, like a card                                                                     | 220ms (+90)                       | cubicOut         |
    | Close             | bar + deck       | `translateY(16px)`, opacity → 0                                                                           | 160ms                             | quadIn           |
    | Close             | backdrop         | opacity → 0                                                                                               | 160ms                             | linear           |
    | Reduced motion    | every card move  | opacity cross-fade, no travel                                                                             | 120ms                             | linear           |
20. **The page-lightening is reverted** (Patrick, 2026-10-30: it had odd
    side effects elsewhere). This supersedes the previous amendment.
    - The dark theme is back on daisyUI's default surfaces: `base-100`
      #1d232a. `app.css` and `palette.ts` match the commit before.
    - The review buttons use the lighter deep-gray style again:
      `bg-base-content/10`, with one subtle border on all three and a faint
      divider on the ▾.
21. **Review layer readability.**
    - The top bar (queue, N of M, progress, ⋯, ✕) has its own surface:
      `bg-base-300/95`, a border, a shadow and a backdrop blur. It uses theme
      text colours, not `text-white`.
    - The backdrop is `bg-black/70` with a light blur, so the journal no longer
      shows through behind it.
    - **Skip** drops its `→` key hint, which read as "→ →" next to the arrow
      icon. `→`/`l`/`n` are still listed in the `?` sheet.
