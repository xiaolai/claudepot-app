// Re-export index for the api tree. Existing imports of
// `from '../api'` keep working — TypeScript resolves to
// `./index.ts` automatically. Each shard owns its domain's
// methods; the merged `api` here is the canonical surface.

import { accountApi } from "./account";
import { projectApi } from "./project";
import { sessionApi } from "./session";
import { sessionOpsApi } from "./session-ops";
import { settingsApi } from "./settings";
import { keyApi } from "./key";
import { activityApi } from "./activity";
import { configApi } from "./config";
import { pricingApi } from "./pricing";
import { artifactUsageApi } from "./artifact-usage";
import { artifactLifecycleApi } from "./artifact-lifecycle";

export const api = {
  ...accountApi,
  ...projectApi,
  ...sessionApi,
  ...sessionOpsApi,
  ...settingsApi,
  ...keyApi,
  ...activityApi,
  ...configApi,
  ...pricingApi,
  ...artifactUsageApi,
  ...artifactLifecycleApi,
};
