# PR #894 screenshots, after the rescope

Before: `main` at `fc92f19b` (the PR's base). After: `97e34476`.
Both are `cargo make build-ui-example-no-sync` builds, captured by `capture.cjs` in Chromium at 2x, UTC, light theme, avatar salt fixed at 7.
Member reactions are added one minute apart in the order 🦀, 👍, 🎉.

| Scene | Before | After |
|---|---|---|
| 01 order, 1280px | 🎉 👍 🦀 (alphabetical) | 🦀 👍 🎉 (order people reacted) |
| 02 reader reacted 🦀 | 🦀 2 selected, last | 🦀 2 selected, first |
| 03 keyboard focus on 👍 | browser default ring | 2px link-colour ring, 2px offset (`.reaction-chip:focus-visible`) |
| 04 Shift+Enter on 🦀 | Remove-only reactor list | same list; chip order differs |
| 05 order, 390px, after a tap on 🦀 | 🎉 👍 🦀 | 🦀 👍 🎉 |

Read from the DOM (`facts.json`): before the chip is `span[role=button][tabindex=0]`, after it is `button` with no role or tabindex. `aria-pressed` and the hover title (`Test Sender B, You (click to remove)`) are the same in both.
