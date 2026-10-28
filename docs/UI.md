# UI tokens and rules

The look of the app is built from the tokens at the top of `app/src/styles.css` (`:root` for dark, `:root[data-theme="light"]`
for light). `app/src/uiRules.test.ts` and `app/src/a11yRules.test.ts` enforce what is written here.

## Colour

- Surfaces: `--bg`, `--panel`, `--s1`, `--s2` (hover/active row), `--s3` (dialog), `--s-in`, `--s-code`.
- Text: `--text`, `--muted`. State colours: `--ok`, `--warn`, `--danger`, `--accent`, `--await`, `--stall`, `--orange`, `--blue`.
  Error and danger are one colour (`--danger`); `--err`, `--ok2` and `--claude` no longer exist (brand colours are `--brand-*`).
- Tinted backgrounds: `--ok-bg`, `--err-bg`, `--warn-bg`, `--accent-bg`; tinted borders `--ok-line`, `--warn-line`, `--err-line`.
- Every text token is at least 4.5:1 on every surface in both themes (the "contrast matrix" test). `--line-ctl`, the border of
  inputs and outlined buttons, is at least 3:1.
- No hex or `rgba(` colour outside `:root`, except the black/white of scrims. A translucent state colour is
  `rgba(var(--ok-rgb), .2)`; keep the `*-rgb` tokens in step with the hex.
- `termprefs.ts` holds the xterm palettes as hex on purpose: xterm needs literal colours and the terminal theme is chosen apart
  from the app theme (Settings). They are not covered by the token rules.

## Elevation, stacking, type

- Shadows: `--shadow-sm` (card at rest), `--shadow-md` (popover, menu, toast), `--shadow-lg` (command palette, dragged card);
  lighter in the light theme. A literal black `box-shadow` fails the test.
- `z-index` above 2 uses the `--z-*` scale: `raised 1`, `bar 5`, `split 6`, `modal 10`, `resizer 20`, `pop 41`, `float 50`, `top 60`,
  `drag 1000`. Tailwind `z-[…]` in components follows the same order (the Select and dropdown lists use 70, above toasts).
- Font sizes `--fs-xs` 10, `--fs-sm` 11, `--fs-md` 12, `--fs-base` 13, `--fs-lg` 14, `--fs-xl` 16, `--fs-2xl` 20.
  Tailwind's names are shifted on purpose (`text-xs` = `--fs-sm`, `text-sm` = `--fs-md`, `text-base`, `text-lg`, `text-xl` = `--fs-xl`,
  `text-2xl` = `--fs-2xl`); for the 10px size use `text-[length:var(--fs-xs)]`.
- Mono: `--font-mono` for code, diffs, IDs and Docker (also Tailwind `font-mono`), `--font-mono-term` for the terminal
  (`DEFAULT_FONT` in `termprefs.ts` repeats it, xterm needs the string).
- Radii `--r-sm..--r-xl` and `--r-pill`.
- Layout: `--status-h` is the height of the status bar, `--left-w` / `--right-w` the side panels.

## Spacing

`--sp-1..--sp-4` (4/8/12/16 px) are defined but **not used** by the stylesheets, which write px values directly (4, 6, 8, 10, 12 are the
common ones). Moving ~500 declarations to the scale would be a large visual change for little gain, so the tokens stay as
a reference for new code; nothing enforces them.

## Icons

Every icon is a path in `app/src/Icon.tsx` (`<Icon name="…" size={14} />`): one 16x16 box, stroke 1.4, round caps. Sizes are 12, 14
or 16; any other value up to 16 is snapped to the nearest step above. A `<svg` anywhere else fails `uiRules.test.ts` (the only
exception is `StateDot.tsx`, the worktree state marker, drawn at 10px). To add an icon, add its path to `PATHS`.

## Components

`app/src/ui/` (`Button`, `Badge`, …) is the component set. The `.chip` and `.pill` classes of the old stylesheet are gone: use
`Badge` for a tag and `Button size="chip"`/`size="pill"` for a clickable one. The global `button.primary`, `button.icon`,
`button.danger`, `button:not([class])` rules in `styles.css` still serve a few call sites that pass `class="primary"` etc. to `Button`;
migrating those to variants is open work.
