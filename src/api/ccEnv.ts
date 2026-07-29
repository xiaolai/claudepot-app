// Bindings for Global → Config → Env Variables.
//
// Every call returns the whole `EnvOverview`, including the two write
// calls: the backend hands back the authoritative post-write state so
// the renderer reconciles against the file rather than against its own
// optimism.

import { invoke } from "@tauri-apps/api/core";
import type { EnvOverview } from "../types/ccEnv";

export const ccEnvApi = {
  /** Spec + resolved state + the three buckets, in one trip. Emits no
   *  secret bytes: a credential-capable variable arrives as
   *  `secret_set`, an unrecognized key as `withheld`. */
  ccEnvList: () => invoke<EnvOverview>("cc_env_list"),

  /** Write one documented, editable variable. The value may be a
   *  credential, so it crosses once and is zeroized backend-side;
   *  callers must clear their own React state in a `finally`. */
  ccEnvSet: (name: string, value: string) =>
    invoke<EnvOverview>("cc_env_set", { name, value }),

  /** Remove one key. Accepts any name actually present, documented or
   *  not — that is what makes a hand-set key clearable.
   *
   *  Never takes effect in a running session: CC re-applies
   *  `settings.env` additively and deletes nothing, so the old value
   *  survives until relaunch. Confirm with that sentence. */
  ccEnvClear: (name: string) => invoke<EnvOverview>("cc_env_clear", { name }),
};
