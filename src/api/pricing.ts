// Pricing table fetch / refresh.
// Sharded from src/api.ts; src/api/index.ts merges every
// domain slice into the canonical `api` object.

import { invoke } from "@tauri-apps/api/core";
import type {
  PriceTableDto,
} from "../types";

export const pricingApi = {
  // Pricing — API-equivalent cost display for subscription users.
  pricingGet: () => invoke<PriceTableDto>("pricing_get"),
  pricingRefresh: () => invoke<PriceTableDto>("pricing_refresh"),
};
