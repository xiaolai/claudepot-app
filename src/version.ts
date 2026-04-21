// Named import so bundler tree-shakes everything in package.json
// except the single field we read — avoids bundling repo metadata
// (name, scripts, deps) into the renderer for one string.
import { version } from "../package.json";

export const APP_VERSION = `v${version}`;
