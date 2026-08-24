// Minimal service worker.
//
// It caches nothing on purpose. Its only job in this build is to prove
// that registration SUCCEEDS, which is the signal that the browser
// considers this origin a secure context. A caching worker here would
// serve stale probe results and make the very thing being measured
// harder to observe.
self.addEventListener("install", () => self.skipWaiting());
self.addEventListener("activate", (e) => e.waitUntil(self.clients.claim()));
