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
import { memoryHealthApi } from "./memory-health";
import { memoryApi } from "./memory";
import { migrateApi } from "./migrate";
import { routeApi } from "./route";
import { automationApi } from "./automation";
import { templateApi } from "./template";
import { usageApi } from "./usage";
import { notificationApi } from "./notification";
import { serviceStatusApi } from "./service-status";
import { updatesApi } from "./updates";
import { ccTipsApi } from "./cc-tips";
import { ccDoctorApi } from "./cc-doctor";
import { rotationApi } from "./rotation";
import { permissionApi } from "./permission";
import { envSecretApi } from "./envSecret";
import { sharedMemoryApi } from "./sharedMemory";

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
  ...memoryHealthApi,
  ...memoryApi,
  ...routeApi,
  ...automationApi,
  ...templateApi,
  ...usageApi,
  ...notificationApi,
  ...serviceStatusApi,
  ...updatesApi,
  ...ccTipsApi,
  ...ccDoctorApi,
  ...rotationApi,
  ...permissionApi,
  ...envSecretApi,
  migrate: migrateApi,
  // Namespaced because the method names (search, listMemories,
  // etc.) are generic and would collide with other domains.
  sharedMemory: sharedMemoryApi,
};

export { migrateApi, sharedMemoryApi };
export type * from "./migrate";
export type * from "./sharedMemory";
