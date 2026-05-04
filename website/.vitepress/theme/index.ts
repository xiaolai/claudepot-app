import { h } from 'vue'
import type { Theme } from 'vitepress'
import DefaultTheme from 'vitepress/theme'
import BrandMark from './components/BrandMark.vue'
import './style.css'

// Inject <BrandMark /> as the navbar title. `themeConfig.siteTitle`
// is set to `false` in config.ts so the default text doesn't render
// alongside the slot.
export default {
  extends: DefaultTheme,
  Layout: () =>
    h(DefaultTheme.Layout, null, {
      'nav-bar-title-before': () => h(BrandMark),
    }),
  enhanceApp({ app }) {
    app.component('BrandMark', BrandMark)
  },
} satisfies Theme
