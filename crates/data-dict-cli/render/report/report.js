// The validation report page: one page per table, chosen from the sidebar, with
// one check in full behind it. The report document rendered for a person — it
// adds nothing to the document and withholds everything the document withholds.
//
// This file holds the app root: the verdict, the sidebar, the pages, and the
// reading helpers both pages and the checks table share. The checks table lives
// in steps.js, the problems list in problems.js, and one problem in full in
// diagnostic.js.

const BASE_TITLE = "Validation report";

/* The report's own wording for its verdict, stated as the page's heading. A
   `warning` status means nothing failed, so only `error` may say the run
   did. */
const VERDICTS = {
  ok: "Validation passed",
  warning: "Passed with warnings",
  error: "Validation failed",
};

/* The verdict as a square, the same mark the sidebar and the checks table
   use. */
const VERDICT_SQUARE = { ok: "pass", warning: "warn", error: "fail" };

/* ---- Routing --------------------------------------------------------------
   Hash routes, so the back button and a pasted link both work. The raw hash is
   split before it is decoded: a table name may contain the separator, and
   decoding first would cut it in the wrong place. Navigation itself — go,
   goHome, useRoute — lives in shared/route.js. */

function parseHash() {
  const raw = location.hash.replace(/^#/, "");
  if (!raw) return null;
  const cut = raw.indexOf("/");
  if (cut < 0) return null;
  return { view: raw.slice(0, cut), key: decodeURIComponent(raw.slice(cut + 1)) };
}

/* ---- Reading the report -------------------------------------------------- */

/* Steps counted by `outcome` and nothing else. There is no default bucket: an
   outcome this page doesn't know would be lost rather than silently counted as
   a pass. */
function stepCounts(steps) {
  const counts = { pass: 0, fail: 0, unevaluated: 0 };
  for (const step of steps) {
    if (step.outcome in counts) counts[step.outcome]++;
  }
  return counts;
}

const stepsById = new Map(REPORT.steps.map((step) => [step.id, step]));

/* The problems each step accounts for. A problem with no `step` — every spec
   problem, and an undocumented column — belongs to no step. */
const problemsByStep = new Map();
REPORT.problems.forEach((problem, index) => {
  problem.index = index;
  if (problem.step == null) return;
  if (!problemsByStep.has(problem.step)) problemsByStep.set(problem.step, []);
  problemsByStep.get(problem.step).push(problem);
});

/* The tables the run covered, in the order its steps name them. */
function tableOrder(steps) {
  const seen = [];
  for (const step of steps) {
    if (!seen.includes(step.table)) seen.push(step.table);
  }
  return seen;
}

/* A table's verdict as one square: red if any of its checks failed or any
   problem names it in error, green otherwise. The sidebar's only states are
   these two — a table nothing could be said about is red, since silence there
   is itself a finding (its source could not be read). */
function tableVerdict(table) {
  const failed = REPORT.steps.some(
    (step) => step.table === table && step.outcome === "fail",
  );
  const errored = REPORT.problems.some(
    (problem) => problem.table === table && problem.severity === "error",
  );
  return failed || errored ? "fail" : "pass";
}

/* The tables the sidebar lists: those the run's steps name, then any only its
   problems or failed rows name. */
const TABLES = (() => {
  const seen = tableOrder(REPORT.steps);
  for (const problem of REPORT.problems) {
    if (problem.table && !seen.includes(problem.table)) seen.push(problem.table);
  }
  for (const entry of REPORT.failed_rows || []) {
    if (!seen.includes(entry.table)) seen.push(entry.table);
  }
  return seen;
})();

/* The page a bare `report.html` opens on: the first table with a problem, since
   that is what a reader came for, else the first table. */
const DEFAULT_TABLE =
  (TABLES.find((table) => tableVerdict(table) === "fail") || TABLES[0]) ?? null;

function tableHash(table) {
  return `#table/${encodeURIComponent(table)}`;
}

/* ---- Pages --------------------------------------------------------------- */

/* Back to a table's page, or to the report's default page when the finding
   names no table. */
function BackLink({ table, label }) {
  const home = table ? () => go(tableHash(table)) : goHome;
  return html`<a class="homelink" href="#" onClick=${(e) => { e.preventDefault(); home(); }}>
    <span class="chev"><${Icon} svg=${ICONS.back} /></span><span>${label}</span></a>`;
}

/* The step page's title block: the h1 names the target beside the check's
   verdict square and carries the way back to the table's page in its chevron. */
function StepTitle({ step }) {
  const columns = step.columns || [];
  const target = step.table + (columns.length ? `.${columns.join(", ")}` : "");
  return html`<div class="step-title">
    <h1><a class="homelink" href="#" title="Back to ${step.table}"
        onClick=${(e) => { e.preventDefault(); go(tableHash(step.table)); }}
      ><span class="chev"><${Icon} svg=${ICONS.back} /></span></a
      ><span class="sqname"><${VerdictSquare} outcome=${step.outcome} /><span class="target">${target}</span></span></h1>
  </div>`;
}

/* One step: how many rows were weighed, where the rule sits in the dictionary,
   and the rows that broke it. The excerpt is the first problem's — a step's
   problems share its target, so one location speaks for the step — falling
   back to the step's own spans when nothing failed, so a passing step shows
   its declaration too. */
function StepPage({ id }) {
  const step = stepsById.get(Number(id));
  if (!step) return html`<p class="rsection-note">No such step.</p>`;
  const problems = problemsByStep.get(step.id) || [];
  const located = problems.find((problem) => problem.location);
  const location = located ? located.location : step.location;
  const context = located ? located.context : step.context;
  return html`<section class="rsection">
    <h2>${stepLabel(step)}${step.row_count != null && html`<span class="row-total"
      >(${fmtNum(step.failed_row_count || 0)} / ${fmtNum(step.row_count)} rows failed)</span>`}</h2>
    ${location ? html`<section class="step-summary${problems[0] ? ` is-${problems[0].severity}` : ""}">
      <details>
        <summary>Constraint declaration</summary>
        <${YamlExcerpt} location=${location} context=${context} />
      </details>
    </section>` : null}
    ${problems.map((problem) => html`<${preact.Fragment} key=${problem.index}>
      <${OffendingRows} problem=${problem} />
      <${RowsNote} problem=${problem} />
    <//>`)}
  </section>`;
}

function ProblemPage({ index }) {
  const problem = REPORT.problems[Number(index)];
  if (!problem) return html`<p class="rsection-note">No such problem.</p>`;
  return html`<section class="rsection">
    <${ProblemCard} problem=${problem} showStep=${true} />
  </section>`;
}

/* Everything the run has to say about one table: its checks, the problems that
   belong to no check, and its failed rows. The checks are the data-level steps
   only; the metadata findings sit below them, since they are why checks went
   unevaluated rather than checks themselves. */
function TablePage({ table }) {
  const problems = REPORT.problems.filter((p) => p.table === table && p.step == null);
  const entry = (REPORT.failed_rows || []).find((e) => e.table === table);
  const failures = entry
    ? cellFailures(REPORT.problems.filter((p) => p.table === table))
    : null;
  return html`<div>
    <${ChecksTable} table=${table} />
    ${problems.length ? html`<section class="rsection">
      <h2>Problems</h2>
      ${problems.map((problem) => html`<${ProblemCard} key=${problem.index}
        problem=${problem} showStep=${true} />`)}
    </section>` : null}
    ${entry ? html`
      <${FailedRowsCard} rows=${entry.rows} keys=${entry.keys} values=${entry.values}
        count=${entry.count} redacted=${entry.redacted} severity="error" failures=${failures} />` : null}
  </div>`;
}

/* ---- The sidebar ----------------------------------------------------------
   One tab per table: its name and its verdict square, the failed against the
   total rows beneath, the active table marked. The tabs are the report's table
   of contents, so they stay on screen while a table's page scrolls. */

/* A table's rows as the run weighed them: the failed rows it counted (each
   counted once, however many checks it broke) against the table's rows, the
   most any step of it checked. Nothing to say when no step counted rows. */
function tableRowCounts(table) {
  let total = null;
  for (const step of REPORT.steps) {
    if (step.table === table && step.row_count != null) {
      total = Math.max(total ?? 0, step.row_count);
    }
  }
  if (total == null) return null;
  const entry = (REPORT.failed_rows || []).find((e) => e.table === table);
  return { failed: entry ? entry.count : 0, total };
}

function Sidebar({ active }) {
  return html`<nav class="sidebar">
    ${TABLES.map((table) => {
      const counts = tableRowCounts(table);
      return html`<a key=${table}
        class=${table === active ? "active" : null}
        href=${tableHash(table)}
        ><span class="sqname"><${VerdictSquare} outcome=${tableVerdict(table)} /><span class="side-name">${table}</span></span>
        ${counts && html`<span class="side-count">(${fmtNum(counts.failed)} / ${fmtNum(counts.total)})</span>`}</a>`;
    })}
  </nav>`;
}

/* ---- The app ------------------------------------------------------------- */

function App() {
  const route = useRoute();

  /* The old failed-rows page moved onto the table's page; send its links
     there. */
  const moved = route && route.view === "rows";
  useEffect(() => {
    if (moved) go(tableHash(route.key));
  }, [moved, route && route.key]);

  /* A table route names its table; no route opens the default table's page
     without rewriting the address bar. */
  const table = route && route.view === "table" ? route.key : !route ? DEFAULT_TABLE : null;

  /* The table a detail page belongs to, so the sidebar can mark it. */
  const storyStep = route && route.view === "step" ? stepsById.get(Number(route.key)) : null;
  const storyProblem = route && route.view === "problem" ? REPORT.problems[Number(route.key)] : null;
  const active = table ?? (storyStep && storyStep.table) ?? (storyProblem && storyProblem.table) ?? null;

  useEffect(() => {
    document.title = table ? `${table} — ${BASE_TITLE}` : BASE_TITLE;
  }, [table]);

  /* Escape leaves a detail view for the table it belongs to. */
  useEffect(() => {
    if (storyStep) return onEscape(20, () => (go(tableHash(storyStep.table)), true));
    if (storyProblem && storyProblem.table) {
      return onEscape(20, () => (go(tableHash(storyProblem.table)), true));
    }
    if (route && route.view === "problem") return onEscape(20, () => (goHome(), true));
  }, [route]);

  let body;
  if (table) {
    body = html`<${TablePage} table=${table} />`;
  } else if (route && route.view === "step") {
    body = html`<${StepPage} id=${route.key} />`;
  } else if (route && route.view === "problem") {
    body = html`<${ProblemPage} index=${route.key} />`;
  } else if (moved) {
    body = null;
  } else if (route) {
    body = html`<p class="rsection-note">No such view.</p>`;
  } else {
    /* No tables at all: the run's problems are all it has to show. */
    body = html`<${ProblemsCard} problems=${REPORT.problems} />`;
  }

  return html`<div>
    <header class="pagehead">
      <div class="head-title">
        ${storyStep
          ? html`<${StepTitle} step=${storyStep} />`
          : table
            ? html`<h1><span class="sqname"><${VerdictSquare} outcome=${tableVerdict(table)} /><span>${table}</span></span></h1>`
            : html`<h1>${route && route.view === "problem"
            ? html`<${BackLink} table=${active} label=${BASE_TITLE} />`
            : html`<span class="sqname"><${VerdictSquare} outcome=${VERDICT_SQUARE[REPORT.status]} />${VERDICTS[REPORT.status]}</span>`}</h1>`}
      </div>
      <div class="head-actions"><${ThemeToggle} /></div>
    </header>
    <div class="report-body">
      <${Sidebar} active=${active} />
      <div class="report-main"><div class="rcontent">${body}</div></div>
    </div>
  </div>`;
}

preact.render(html`<${App} />`, document.getElementById("app"));
