import type { BudgetRow } from "./api";
import { levelClass } from "./budgetLogic";
import { B } from "./strings.budget";
import { fmtTokens } from "./usageFormat";

/** One budget as a small bar with its percentage; the level is also in the tooltip and the accessible name, not only in the colour. */
export function BudgetMeter(props: { row: BudgetRow; name: string }) {
  const tip = () => {
    const r = props.row;
    return `${B.meterTip(r.period, props.name, fmtTokens(r.used), fmtTokens(r.limit), r.pct)}, ${B.levelWord[r.level]}`;
  };
  return (
    <span class="lim" role="img" aria-label={tip()} title={tip()}>
      <span class="lim-dim">{B.meterShort[props.row.period]}</span>
      <span class="lim-bar">
        <i classList={levelClass(props.row.level)} style={{ width: `${Math.min(100, props.row.pct)}%` }} />
      </span>
      <span>{props.row.pct}%</span>
    </span>
  );
}
