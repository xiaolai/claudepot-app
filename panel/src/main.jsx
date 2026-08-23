// Entry point. Import order is load-bearing — see globals.js.
import './globals.js';

import './fonts.css';
import './vendor/ds-tokens.css';
import './vendor/ds-icons.jsx';
import './vendor/ds-kit.jsx';
import './markdown.css';

import React from 'react';
import { createRoot } from 'react-dom/client';

import { Panel } from './app/Panel.jsx';

// The service worker is registered rather than merely declared, because
// registration succeeding is the signal that the browser treats this
// origin — a privately-trusted certificate on a tailnet name — as a
// secure context. It caches nothing; see assets/sw.js.
if ('serviceWorker' in navigator) {
  window.addEventListener('load', () => {
    navigator.serviceWorker.register('/sw.js').catch(() => {
      /* Not fatal: the panel works without it. The probe at /probe is
         where a failure is diagnosed. */
    });
  });
}

createRoot(document.getElementById('root')).render(
  <React.StrictMode>
    <Panel />
  </React.StrictMode>,
);
