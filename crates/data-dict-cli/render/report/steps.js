/* One table's checks: the data-level steps it was validated against, sortable
   and filterable. Rendered by report.js, which owns the reading helpers
   (stepCounts, problemsByStep, tableOrder) this file uses. */

/* What a step checked. An assertion names the columns it reads and carries its
   expression on the check itself, since the columns are what a reader scans
   for. */
function StepTarget({ step }) {
  const columns = step.columns || [];
  if (!columns.length) {
    return html`<span class="step-target"><span class="whole">whole table</span></span>`;
  }
  return html`<span class="step-target">${columns.join(", ")}</span>`;
}

/* A step's check as a reader says it: the author's own description where they
   wrote one, else the check's name, with the assertion it ran for an
   assertion step. */
function stepLabel(step) {
  if (step.description) return step.description;
  return step.assertion
    ? `${checkName(step.code)}: ${step.assertion}`
    : checkName(step.code);
}

/* The label as plain text, with the code quiet in parentheses. */
function StepCheck({ step }) {
  return html`<span class="step-check">${stepLabel(step)}${" "}<span class="code">(${step.code})</span></span>`;
}

/* What a bar weighs depends on what it is given: a step's own failures against
   its rows, or a table's summed against every row its checks weighed. Those are
   different units, so the tooltip names the one it is showing. Drawn only when
   there are rows to weigh, so the bar's presence is itself the claim that the
   step was evaluated — an unevaluated step, or a failing step over an empty
   table, gets none. */
function StepMeter({ rows, failed, label = "failed rows" }) {
  if (!rows) return null;
  const share = failed / rows;
  return html`<div class="step-meter">
    <div class="step-track"
      onMouseEnter=${(e) => showTip(barTip(label, failed, rows), e)}
      onMouseMove=${moveTip} onMouseLeave=${hideTip}>
      ${failed > 0 &&
        html`<div class=${`step-fill${share >= 1 ? " full" : ""}`}
          style=${`width:${Math.max(share * 100, 0)}%`} />`}
    </div>
  </div>`;
}

/* The columns the roster can sort by. `null` sorts last at either direction,
   so an unevaluated step sinks rather than masquerading as zero. */
const SORTS = {
  check: (step) => stepLabel(step),
  target: (step) => (step.columns || []).join(", "),
  failed: (step) => step.failed_row_count,
};

/* A click cycles a column ascending, descending, and back to the dictionary
   order the roster starts in — except a failures column, which starts
   descending: nobody sorts failures to find the fewest. */
const DESC_FIRST = ["failed"];

function cycleSort(sort, key) {
  const first = DESC_FIRST.includes(key) ? -1 : 1;
  if (!sort || sort.key !== key) return { key, dir: first };
  return sort.dir === first ? { key, dir: -first } : null;
}

function sortBy(items, sort, sorts) {
  if (!sort) return items;
  const value = sorts[sort.key];
  return [...items].sort((a, b) => {
    const va = value(a);
    const vb = value(b);
    if (va == null || vb == null) return va == null ? (vb == null ? 0 : 1) : -1;
    const cmp = typeof va === "number" ? va - vb : va.localeCompare(vb);
    return cmp * sort.dir;
  });
}

/* The indicator's space is reserved whether or not the column is sorted, so
   clicking a head doesn't shift the others. */
function SortHead({ label, sortKey, sort, onSort, numeric }) {
  const active = sort && sort.key === sortKey;
  return html`<th class=${numeric ? "num" : null}>
    <button class="sort-head" onClick=${() => onSort(cycleSort(sort, sortKey))}>
      ${label}<span class=${active ? "sort-ind" : "sort-ind off"}>${active && sort.dir === -1 ? "▾" : "▴"}</span>
    </button>
  </th>`;
}

/* A verdict as a coloured square: green passed, yellow warned, red failed.
   An unevaluated check gets no square, but keeps its space so the names stay
   aligned. Sized in em, so it grows with the text it sits beside. */
function VerdictSquare({ outcome }) {
  return html`<span class=${`verdict-square ${outcome === "unevaluated" ? "off" : outcome}`}></span>`;
}

/* Only a failed step links to its page — a passed or unevaluated step has
   nothing actionable to show there. The link wears the check's name. */
function StepRow({ step }) {
  const failed = step.failed_row_count;
  const evaluated = step.outcome !== "unevaluated";
  const href = step.outcome === "fail" ? `#step/${step.id}` : null;
  return html`<tr>
    <td>
      <span class="sqname"><${VerdictSquare} outcome=${step.outcome} />${href
        ? html`<a class="step-link" href=${href}><${StepCheck} step=${step} /></a>`
        : html`<${StepCheck} step=${step} />`}</span>
    </td>
    <td><${StepTarget} step=${step} /></td>
    <td class="num">${!evaluated || failed == null ? "—" : fmtNum(failed)}</td>
    <td><${StepMeter} rows=${step.row_count} failed=${step.failed_row_count || 0} /></td>
  </tr>`;
}

/* The table's fixed grid: long content clips rather than shifting the column
   edges. The first column takes the width the others leave. */
function ReportColGroup() {
  return html`<colgroup>
    <col /><col class="col-mid" /><col class="col-num" /><col class="col-meter" />
  </colgroup>`;
}

/* The data-level steps of one table. The metadata checks a data run implies
   (a column exists, a source is declared) are means, not findings — the table
   page shows them as problems instead. A table whose data could not be read
   leaves every check of it unevaluated, so the reason is given once above the
   table rather than repeated down every row. */
function ChecksTable({ table }) {
  const [sort, setSort] = useState(null);
  const [query, setQuery] = useState("");
  const [failuresOnly, setFailuresOnly] = useState(false);
  const all = REPORT.steps.filter(
    (step) => step.table === table && !step.code.startsWith("M"),
  );
  let shown = all;
  if (failuresOnly) shown = shown.filter((step) => step.outcome === "fail");
  if (query) {
    const needle = query.toLowerCase();
    shown = shown.filter((step) =>
      (step.columns || []).some((column) => column.toLowerCase().includes(needle)));
  }
  const counts = stepCounts(all);
  const unreadable = REPORT.problems.find(
    (p) => p.table === table && (p.code === "M04" || p.code === "M05"),
  );
  const note = query
    ? "No checks match."
    : all.length
      ? "No failures."
      : "No data-level checks.";
  return html`<section class="rsection">
    <div class="checks-head">
      <h2>Checks<span class="row-total">(${fmtNum(counts.fail)} / ${fmtNum(all.length)} checks failed)</span></h2>
      <div class="checks-tools">
        <input class="checks-filter" type="search" placeholder="Filter to a column…"
          value=${query} onInput=${(e) => setQuery(e.target.value)} />
        <label class="checks-only"><input type="checkbox" checked=${failuresOnly}
          onChange=${(e) => setFailuresOnly(e.target.checked)} /> Failures only</label>
      </div>
    </div>
    ${counts.unevaluated && unreadable
      ? html`<p class="rsection-note">${
          unreadable.code === "M04" ? "No source declared" : "Data could not be read"
        }, ${fmtNum(counts.unevaluated)} not evaluated.</p>`
      : null}
    ${shown.length
      ? html`<div class="tlist-wrap">
          <table class="tlist steps rtable">
            <${ReportColGroup} />
            <thead><tr>
              <${SortHead} label="Check" sortKey="check" sort=${sort} onSort=${setSort} />
              <${SortHead} label="Target" sortKey="target" sort=${sort} onSort=${setSort} />
              <${SortHead} label="Failed rows" sortKey="failed" sort=${sort} onSort=${setSort} numeric=${true} />
              <th></th>
            </tr></thead>
            <tbody class="tgroup">
              ${sortBy(shown, sort, SORTS).map((step) => html`<${StepRow} key=${step.id} step=${step} />`)}
            </tbody>
          </table>
        </div>`
      : html`<p class="rsection-note">${note}</p>`}
  </section>`;
}
