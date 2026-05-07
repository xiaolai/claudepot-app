import Link from "next/link";
import type { Metadata } from "next";

import { staffGate } from "@/lib/staff-gate";
import { getMonthlyReconcile } from "@/db/office-queries";
import { relativeTime } from "@/lib/format";
import {
  upsertProviderInvoice,
  deleteProviderInvoice,
} from "@/lib/actions/cost-reconcile";

export const metadata: Metadata = {
  title: "Cost reconcile · admin",
  description:
    "Compare bot self-reported AI spend with provider-invoiced totals. Staff-only.",
};

export const dynamic = "force-dynamic";

const USD = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "USD",
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
});

function fmt(usd: number): string {
  return USD.format(usd);
}

function defaultMonth(): string {
  // Default the form to the previous month — the case where staff is
  // uploading an invoice that arrived for the just-ended billing period.
  const d = new Date();
  d.setUTCDate(1);
  d.setUTCMonth(d.getUTCMonth() - 1);
  return d.toISOString().slice(0, 7);
}

export default async function CostReconcilePage({
  searchParams,
}: {
  searchParams: Promise<{ as?: string; ok?: string; error?: string }>;
}) {
  const sp = await searchParams;
  const gate = await staffGate(sp);
  if (gate) return gate;

  const rows = await getMonthlyReconcile({ months: 12 });

  return (
    <div className="proto-page">
      <header className="proto-section">
        <p className="eyebrow">
          <Link href="/admin/console">/admin/console</Link>
        </p>
        <h1>Cost reconcile</h1>
        <p className="proto-dek">
          Self-reported (from bots&rsquo; <code>kind=&quot;cost&quot;</code>{" "}
          payloads) vs invoiced (uploaded by you, manually). Variance is{" "}
          <em>invoiced − self-reported</em>; positive means bots
          under-reported. Reconciliation runs monthly; uploads here are
          source-of-truth for the audit trail, not for billing.
        </p>
      </header>

      {sp.error ? (
        <p className="proto-form-flash proto-form-flash-err">
          {decodeURIComponent(sp.error)}
        </p>
      ) : null}
      {sp.ok ? (
        <p className="proto-form-flash proto-form-flash-ok">
          {sp.ok === "deleted" ? "Invoice removed." : "Invoice saved."}
        </p>
      ) : null}

      <section className="proto-section">
        <h2>Upload invoice</h2>
        <p className="proto-dek">
          One row per (provider, month). Re-uploading the same pair
          overwrites the previous entry; no version history.
        </p>
        <form action={upsertProviderInvoice} className="proto-form">
          <div className="proto-form-row">
            <label>
              Provider
              <input
                type="text"
                name="provider"
                placeholder="anthropic"
                required
                maxLength={40}
                pattern="[A-Za-z0-9_-]+"
                className="proto-input"
              />
            </label>
            <label>
              Month
              <input
                type="month"
                name="month"
                defaultValue={defaultMonth()}
                required
                className="proto-input"
              />
            </label>
            <label>
              Invoiced USD
              <input
                type="number"
                name="invoicedUsd"
                step="0.01"
                min="0"
                max="1000000"
                required
                className="proto-input"
              />
            </label>
          </div>
          <label>
            Notes (optional)
            <input
              type="text"
              name="notes"
              placeholder="e.g. credits applied, tier upgrade"
              maxLength={500}
              className="proto-input proto-input-wide"
            />
          </label>
          <button type="submit" className="proto-btn-primary">
            Upload
          </button>
        </form>
      </section>

      <section className="proto-section">
        <h2>Last 12 months</h2>
        <table className="proto-table">
          <thead>
            <tr>
              <th scope="col">Month</th>
              <th scope="col" className="proto-table-num">
                Self-reported
              </th>
              <th scope="col" className="proto-table-num">
                Invoiced
              </th>
              <th scope="col" className="proto-table-num">
                Variance
              </th>
              <th scope="col">Invoices</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => {
              const tone =
                r.invoicedUsd === 0 && r.selfReportedUsd === 0
                  ? "quiet"
                  : Math.abs(r.varianceUsd) <= 0.01
                    ? "ok"
                    : Math.abs(r.varianceUsd) <
                        Math.max(r.invoicedUsd, r.selfReportedUsd) * 0.05
                      ? "warn"
                      : "alert";
              return (
                <tr key={r.month} data-tone={tone}>
                  <th scope="row">{r.month}</th>
                  <td className="proto-table-num">{fmt(r.selfReportedUsd)}</td>
                  <td className="proto-table-num">
                    {r.invoicedUsd > 0 ? fmt(r.invoicedUsd) : "—"}
                  </td>
                  <td className="proto-table-num">
                    {r.invoicedUsd > 0 ? fmt(r.varianceUsd) : "—"}
                  </td>
                  <td>
                    {r.invoices.length === 0 ? (
                      <span className="proto-empty">no invoices yet</span>
                    ) : (
                      <ul className="proto-inline-list">
                        {r.invoices.map((inv) => (
                          <li key={inv.id}>
                            <strong>{inv.provider}</strong>:{" "}
                            {fmt(inv.invoicedUsd)}{" "}
                            <span className="proto-empty">
                              · {relativeTime(inv.uploadedAt.toISOString())}
                              {inv.uploadedBy ? ` by @${inv.uploadedBy}` : ""}
                              {inv.notes ? ` · ${inv.notes}` : ""}
                            </span>{" "}
                            <form
                              action={deleteProviderInvoice}
                              style={{ display: "inline" }}
                            >
                              <input
                                type="hidden"
                                name="id"
                                value={inv.id}
                              />
                              <button
                                type="submit"
                                className="proto-btn-link"
                                aria-label={`Delete ${inv.provider} ${r.month} invoice`}
                              >
                                delete
                              </button>
                            </form>
                          </li>
                        ))}
                      </ul>
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
        <p className="proto-empty proto-empty-spaced">
          The current month&rsquo;s self-reported total includes
          today&rsquo;s running live spend; closed months are stable.
        </p>
      </section>
    </div>
  );
}
