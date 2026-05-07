/**
 * Public surface of the bots reporting domain.
 */

export {
  KIND_LABELS,
  KIND_SCHEMA_BY_KIND,
  REPORT_KINDS,
  isReportKind,
  reportInputSchema,
  type ReportInput,
  type ReportKind,
} from "./schemas";

export { persistBotReport, type PersistResult } from "./reports";
