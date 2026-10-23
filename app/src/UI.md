# UI rules

One look, defined in `app/src/styles.css`. `uiRules.test.ts` and `a11yRules.test.ts` fail when these are broken.

- **Colours:** only `var(--...)`. No hex or `rgb()` outside the `:root` blocks. Light and dark are two sets of the same
  variables (`:root` and `:root[data-theme="light"]`), so a component never knows the theme.
- **Type:** `--fs-xs` 10, `--fs-sm` 11, `--fs-md` 12, `--fs-base` 13, `--fs-lg` 14, `--fs-xl` 16.
- **Corners:** `--r-sm` 4 (inputs, small chips), `--r-md` 6 (buttons), `--r-lg` 8 (cards), `--r-xl` 10 (dialogs, menus), `--r-pill`.
- **Spacing:** `--sp-1` 4, `--sp-2` 8, `--sp-3` 12, `--sp-4` 16.
- **Surfaces:** `--bg` page, `--panel` panels and menus, `--s1` hover, `--s2` pressed/pills, `--s3` dialogs and cards,
  `--s-in` inputs, `--s-code` code blocks.
- **Status colours:** `--ok`, `--warn`, `--danger`/`--err`, `--accent`; their tinted backgrounds are `--ok-bg`, `--warn-bg`, `--err-bg`, `--accent-bg`.
- **Library:** shadcn-style components on Kobalte and Tailwind v4 live in `ui/` (`Button`, `Input`, `Textarea`, `Field`; more are added as screens move over). They use the same tokens through `tailwind.css` (`bg-primary`, `text-muted-foreground`, `rounded-md`...). The older hand-written styles are in the `legacy` CSS layer, below the utilities, and shrink as screens migrate. Build new UI with `ui/`, not with new global CSS.
- **Other components** (`Select.tsx`, `Modal.tsx`, `Tab.tsx`, `Confirm.tsx`): use them instead of raw `<button>`,
  `<select>` and hand-made dialogs. A dialog has one `primary` button; destructive ones use `danger`.
- **Feedback:** results and errors go to the toast (`showNotice`, `setError`), never to text inside a panel.
