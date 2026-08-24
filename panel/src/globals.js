// The design system ships as three files that publish onto `window`
// and read `React` from it. Keeping them byte-identical to what the
// designers delivered is worth more than the tidiness of rewriting them
// into modules: a token or an icon can be re-exported from upstream
// without a merge.
//
// This module exists only to put `React` in scope before they load, and
// it must be imported FIRST. ES module imports are evaluated in source
// order, depth-first, so `import './globals.js'` ahead of the vendor
// imports is what makes the ordering a property of the code rather than
// of luck.
import React from 'react';

window.React = React;
