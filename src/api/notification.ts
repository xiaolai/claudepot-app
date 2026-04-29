// Notification routing — back-end side of the click-target queue.
//
// The Tauri 2 desktop notification plugin doesn't surface body-click
// events to JS, so click routing happens in the App-shell focus
// listener (see src/App.tsx). For the `host` intent the listener
// invokes the command below; for `app` and `info` intents it stays
// in the renderer.

import { invoke } from "@tauri-apps/api/core";

export const notificationApi = {
  /**
   * Activate the host terminal/editor running the given live
   * session. The backend looks up the session's PID via the live
   * runtime, walks parent processes to the first known terminal/
   * editor bundle, and asks LaunchServices to bring it to the
   * foreground. Returns `true` when a host was activated, `false`
   * when none could be resolved (session ended, or its host
   * process is unknown to us). `false` is the renderer's signal
   * to fall back to deep-linking the transcript inside Claudepot.
   *
   * Best-effort by design: there's no guarantee the host process
   * is still alive at click time, no guarantee the multiplexer's
   * pane can be focused, no guarantee an SSH'd remote session
   * has any local GUI host at all. The renderer's fallback path
   * handles all three.
   */
  notificationActivateHostForSession: (sessionId: string) =>
    invoke<boolean>("notification_activate_host_for_session", { sessionId }),
};
