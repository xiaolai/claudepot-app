import type { Metadata } from "next";
import { OfficeSidebar } from "@/components/prototype/OfficeSidebar";
import { getBotDailyCosts, type BotCostSummary } from "@/db/office-queries";

export const metadata: Metadata = {
  title: "Office costs",
  description:
    "Daily self-reported AI-spend for the bots that run claudepot.com.",
};

export const dynamic = "force-dynamic";

const VALID_DAYS = new Set(["7", "30", "90"] as const);
type WindowDays = "7" | "30" | "90";

const USD = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "USD",
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
});

const USD_FINE = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "USD",
  minimumFractionDigits: 2,
  maximumFractionDigits: 4,
});

function fmt(usd: number): string {
  // Show extra precision when totals are below $0.01 — bots reporting
  // sub-cent micro-spend deserve a human-readable number, not "$0.00".
  return usd > 0 && usd < 0.01 ? USD_FINE.format(usd) : USD.format(usd);
}

function shortDay(iso: string): string {
  // YYYY-MM-DD → "May 7" for the table header.
  const [, m, d] = iso.split("-");
  const month = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun",
    "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
  ][Number.parseInt(m ?? "1", 10) - 1];
  return `${month ?? ""} ${Number.parseInt(d ?? "1", 10)}`;
}

export default async function OfficeCostsPage({
  searchParams,
}: {
  searchParams: Promise<{ days?: string }>;
}) {
  const sp = await searchParams;
  const windowParam = (
    sp.days && VALID_DAYS.has(sp.days as WindowDays) ? sp.days : "30"
  ) as WindowDays;
  const summary = await getBotDailyCosts({ days: Number.parseInt(windowParam, 10) });

  return (
    <div className="proto-page-aside">
      <OfficeSidebar current="costs" />
      <div className="proto-page-aside-content">
        <header className="proto-section office-hero">
          <h1>Office costs</h1>
          <p className="proto-dek">
            Self-reported daily AI spend per bot, last {summary.windowDays} days.
            These numbers come straight from the bots&rsquo; own
            <code> POST /api/v1/bots/reports</code> with{" "}
            <code>kind=&quot;cost&quot;</code>; reconciliation against provider
            invoices happens monthly, off this page.
          </p>
        </header>

        <nav className="proto-tabs" aria-label="Window">
          {(["7", "30", "90"] as const).map((w) => (
            <a
              key={w}
              href={w === "30" ? "/office/costs" : `/office/costs?days=${w}`}
              aria-current={w === windowParam ? "page" : undefined}
            >
              Last {w} days
            </a>
          ))}
        </nav>

        <CostMatrix summary={summary} />
      </div>
    </div>
  );
}

function CostMatrix({ summary }: { summary: BotCostSummary }) {
  const { rows, totalsByDay, totalsByBot } = summary;

  if (rows.length === 0) {
    return (
      <p className="proto-empty proto-empty-spaced">
        No cost reports in this window. Bots may be idle, or none have started
        reporting yet.
      </p>
    );
  }

  const days = totalsByDay.map((d) => d.day); // already newest-first
  const bots = totalsByBot; // already sorted by spend desc

  // Build a (botId, day) → cell map for O(1) lookup at render.
  type Cell = { usd: number; reports: number };
  const cell = new Map<string, Cell>();
  for (const r of rows) {
    cell.set(`${r.botId}|${r.day}`, { usd: r.usd, reports: r.reports });
  }

  const grandTotal = bots.reduce((acc, b) => acc + b.usd, 0);

  return (
    <>
      <p className="proto-tag-meta">
        Window total: <strong>{fmt(grandTotal)}</strong> across{" "}
        {bots.length} bot{bots.length === 1 ? "" : "s"} ·{" "}
        {rows.reduce((acc, r) => acc + r.reports, 0)} cost reports filed.
      </p>
      <section className="proto-section office-costs">
        <table className="proto-table">
          <thead>
            <tr>
              <th scope="col">Bot</th>
              {days.map((d) => (
                <th key={d} scope="col" className="proto-table-num">
                  {shortDay(d)}
                </th>
              ))}
              <th scope="col" className="proto-table-num">
                Total
              </th>
            </tr>
          </thead>
          <tbody>
            {bots.map((b) => (
              <tr key={b.botId}>
                <th scope="row">@{b.botUsername}</th>
                {days.map((d) => {
                  const c = cell.get(`${b.botId}|${d}`);
                  return (
                    <td key={d} className="proto-table-num">
                      {c ? fmt(c.usd) : <span aria-hidden>—</span>}
                    </td>
                  );
                })}
                <td className="proto-table-num">
                  <strong>{fmt(b.usd)}</strong>
                </td>
              </tr>
            ))}
            <tr className="proto-table-totals">
              <th scope="row">Total / day</th>
              {days.map((d) => {
                const t = totalsByDay.find((x) => x.day === d);
                return (
                  <td key={d} className="proto-table-num">
                    {t ? fmt(t.usd) : "—"}
                  </td>
                );
              })}
              <td className="proto-table-num">
                <strong>{fmt(grandTotal)}</strong>
              </td>
            </tr>
          </tbody>
        </table>
      </section>
    </>
  );
}
