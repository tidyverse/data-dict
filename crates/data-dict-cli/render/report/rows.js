/* The failed rows of a problem: the evidence the terminal has no room for,
   as a table of row numbers and primary keys against the values they held. */

/* Which columns a `values` list names, in order of first appearance. Taken from
   the entries themselves and never from `columns`: a restricted column is
   dropped from the entry upstream, and a column built from `columns` would
   render a blank cell there — indistinguishable from a value that was absent. */
function valueColumns(values) {
  const seen = [];
  for (const row of values || []) {
    for (const column of Object.keys(row)) {
      if (!seen.includes(column)) seen.push(column);
    }
  }
  return seen;
}

const NUMERIC = /^-?\d+(\.\d+)?$/;
const MAX_DECIMALS = 6;

/* How a column renders: right-aligned with a fixed number of decimal places
   when every value it holds is numeric, the decimals being the most any value
   needs (capped). Integers keep their written form. */
function columnFormat(values, column) {
  const held = (values || [])
    .map((row) => row[column])
    .filter((value) => value != null);
  if (!held.length || !held.every((value) => NUMERIC.test(value))) {
    return { numeric: false, decimals: 0 };
  }
  const decimals = Math.min(
    MAX_DECIMALS,
    Math.max(...held.map((value) => (value.split(".")[1] || "").length)),
  );
  return { numeric: true, decimals };
}

function formatValue(value, format) {
  if (!format.numeric || format.decimals === 0) return value;
  return Number(value).toFixed(format.decimals);
}

function Value({ value, format }) {
  /* JSON null is a *missing* value, and stays distinct from a string column
     holding "null". */
  if (value === null) {
    return html`<span class="is-null" title="missing value">NULL</span>`;
  }
  if (value === undefined) return html`<span class="is-absent">—</span>`;
  return html`${formatValue(value, format)}`;
}

/* The cells a set of problems blames: row number → column → the problems
   naming it. A problem names its cells through the values it read; one that
   proves rows without values — a null in a required column — blames its own
   columns. A problem's `keys` are not consulted: they identify the row it is
   reporting, and a key column is only at fault when the check is about it, in
   which case it is named the same way any other column is. */
function cellFailures(problems) {
  const byCell = new Map();
  const mark = (row, column, problem) => {
    if (!byCell.has(row)) byCell.set(row, new Map());
    const columns = byCell.get(row);
    if (!columns.has(column)) columns.set(column, []);
    columns.get(column).push(problem);
  };
  for (const problem of problems) {
    (problem.rows || []).forEach((row, i) => {
      const named = Object.keys((problem.values && problem.values[i]) || {});
      const blamed = named.length ? named : problem.columns || [];
      blamed.forEach((column) => mark(row, column, problem));
    });
    if (problem.row != null) {
      (problem.columns || []).forEach((column) => mark(problem.row, column, problem));
    }
  }
  return byCell;
}

/* Why a cell failed, for its tooltip: one head and constraint per problem
   that names the cell — the author's own description where they wrote one,
   the check's name where they didn't. */
function cellTip(problems, column) {
  const box = el("div");
  for (const problem of problems) {
    const step = problem.step != null ? stepsById.get(problem.step) : null;
    box.appendChild(tipHead(`${problem.code} · ${column}`));
    box.appendChild(el("p", null, step ? stepLabel(step) : checkName(problem.code)));
  }
  return box;
}

/* The checks a row broke, one entry per code. A row can say why it is here
   without the reader hunting for the shaded cell, which on a wide table may be
   scrolled out of sight. */
function rowChecks(failures, row) {
  const columns = failures && failures.get(row);
  if (!columns) return [];
  const byCode = new Map();
  for (const problems of columns.values()) {
    for (const problem of problems) {
      if (!byCode.has(problem.code)) byCode.set(problem.code, new Set());
      byCode.get(problem.code).add(problem);
    }
  }
  return [...byCode.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([code, problems]) => ({ code, problems: [...problems] }));
}

/* What a code in the row's own column stands for. */
function checkTip(problems) {
  const box = el("div");
  for (const problem of problems) {
    const step = problem.step != null ? stepsById.get(problem.step) : null;
    box.appendChild(tipHead(problem.code));
    box.appendChild(el("p", null, step ? stepLabel(step) : checkName(problem.code)));
  }
  return box;
}

function RowTable({ rows, keys, values, failures }) {
  /* The checks column earns its place only where the rows came from several of
     them. A problem's own card lists rows that all broke the one check it is
     about, so there the column would repeat a constant the card already states. */
  const blame = !!failures;
  const keyCols = valueColumns(keys);
  const valCols = valueColumns(values);
  const columns = [...keyCols, ...valCols];
  const formats = Object.fromEntries(
    columns.map((c) => [c, columnFormat([...(keys || []), ...(values || [])], c)]),
  );
  const cell = (i, c) => {
    const entry = keys && keys[i] && c in keys[i] ? keys[i] : values && values[i];
    return entry ? entry[c] : undefined;
  };
  return html`<div class="row-table">
    <table>
      <thead><tr><th class="rownum">Row</th>${blame ? html`<th class="row-checks">Failed</th>` : null}${columns.map((c, j) => html`<th key=${c} class=${j === keyCols.length && j > 0 ? "val-start" : null}>${c}</th>`)}</tr></thead>
      <tbody>
        ${rows.map((row, i) => html`<tr key=${row}>
          <td class="rownum">${fmtNum(row)}</td>
          ${blame ? html`<td class="row-checks">${rowChecks(failures, row).map((c) => html`<span key=${c.code}
            class="code-chip" onMouseEnter=${(e) => showTip(checkTip(c.problems), e)}
            onMouseMove=${moveTip} onMouseLeave=${hideTip}>${c.code}</span>`)}</td>` : null}
          ${columns.map((c, j) => {
            const blamed = failures && failures.get(row) && failures.get(row).get(c);
            return html`<td key=${c}
              class=${[formats[c].numeric ? "num" : null, j === keyCols.length && j > 0 ? "val-start" : null, blamed ? "bad" : null].filter(Boolean).join(" ") || null}
              onMouseEnter=${blamed ? (e) => showTip(cellTip(blamed, c), e) : null}
              onMouseMove=${blamed ? moveTip : null}
              onMouseLeave=${blamed ? hideTip : null}>
              <${Value} value=${cell(i, c)} format=${formats[c]} />
            </td>`;
          })}
        </tr>`)}
      </tbody>
    </table>
  </div>`;
}

/* Why a problem names no rows, said inside its card: an assertion is one
   verdict about the whole table, and some checks prove a count without naming
   a row. */
function RowsNote({ problem }) {
  const rows = problem.rows || [];
  const count = problem.count;
  if (problem.kind === "assertion_false") {
    return html`<p class="rows-note">This assertion is one verdict about the whole
      table, so no row is named.</p>`;
  }
  if (problem.kind === "assertion_overflow" && problem.row != null) return null;
  if (rows.length || count == null) return null;
  return html`<p class="rows-note">
    <strong>${fmtNum(count)} ${count === 1 ? "row" : "rows"} failed.</strong>
    ${" "}This check counted them without naming them.</p>`;
}

/* A table of failed rows as its own card: the heading, the withheld badge and
   note, and the grid itself. A problem's card, an overflow's single row, and a
   dataset's failed-rows page are the same thing wearing different evidence. */
/* How many rows are on show, against how many failed and how many there are.
   `count` is a true total only where it came from one check; a dataset's is the
   distinct rows among each check's capped sample, which understates, so that
   caller marks it `sampled` and the caption claims no total. */
function rowCaption({ shown, count, checked, sampled }) {
  const of = sampled
    ? `first ${fmtNum(shown)} shown`
    : count > shown
      ? `${fmtNum(shown)} of ${fmtNum(count)} shown`
      : `all ${fmtNum(shown)} shown`;
  return `(${checked == null ? of : `${of} · ${fmtNum(checked)} rows total`})`;
}

function FailedRowsCard({ title = "Failed rows", rows, keys, values, count, checked, sampled, redacted, severity, failures, note }) {
  return html`<article class="failed-rows-card is-${severity}">
    <div class="head">
      <h3>${title}${" "}<span class="cap">${
        rowCaption({ shown: rows.length, count, checked, sampled })}</span></h3>
      ${redacted && html`<span class="key restricted">values withheld</span>`}
    </div>
    <${RowTable} rows=${rows} keys=${keys} values=${values} failures=${failures} />
    ${redacted &&
      html`<p class="note">A column here is${" "}
        <code class="tick">display: restricted</code>, so its values are withheld.
        The row numbers are exact.</p>`}
    ${note && html`<p class="note">${note}</p>`}
  </article>`;
}

/* The rows that broke a problem, as their own card. Dispatch is on what the
   problem carries rather than on its code: a check reports what its evidence
   supports, and some prove a count without naming a row (see RowsNote). */
function OffendingRows({ problem }) {
  const rows = problem.rows || [];
  if (problem.kind === "assertion_overflow" && problem.row != null) {
    return html`<${FailedRowsCard} title="Failed row" rows=${[problem.row]}
      keys=${null} values=${null} severity=${problem.severity}
      note="Evaluation stopped at the first overflow." />`;
  }
  if (!rows.length) return null;
  /* No blame map here: highlighting is the dataset page's, where every column
     is shown; a problem's own card already names what it is about. */
  const step = problem.step != null ? stepsById.get(problem.step) : null;
  return html`<${FailedRowsCard} rows=${rows} keys=${problem.keys}
    values=${problem.values} count=${problem.count} checked=${step ? step.row_count : null}
    redacted=${"redacted" in problem && problem.redacted} severity=${problem.severity} />`;
}
