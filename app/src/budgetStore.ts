import { createSignal } from "solid-js";
import { BudgetRow, budgetStatus } from "./api";

/** Spending against the token limits, as the daemon last reported it. Filled by the usage poll (Usage.tsx). */
export const [budgetRows, setBudgetRows] = createSignal<BudgetRow[]>([]);

/** Asks the daemon for the rows of these worktrees; an older daemon or a failure leaves what was there. */
export const loadBudgets = (paths: string[]) =>
  budgetStatus(paths)
    .then(setBudgetRows)
    .catch(() => {
      /* the budgets are optional: the bars just stay as they were */
    });
