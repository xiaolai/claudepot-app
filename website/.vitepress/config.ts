import { defineConfig } from 'vitepress'

export default defineConfig({
  // `title` lands in <title> and OG tags as a plain string. The
  // colored "Clau" + "Depot" rendering is applied in the navbar
  // (via the BrandMark slot — see .vitepress/theme/index.ts), in
  // the home hero (via gradient on `--vp-home-hero-name-*`), and
  // in the footer copyright (via inline HTML below).
  title: 'ClauDepot',
  description: 'A control panel for Claude Code and Claude Desktop.',
  cleanUrls: true,
  lastUpdated: true,
  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: '/logo.svg' }],
    ['link', { rel: 'icon', type: 'image/png', sizes: '512x512', href: '/logo.png' }],
    ['link', { rel: 'apple-touch-icon', sizes: '180x180', href: '/apple-touch-icon.png' }],
  ],
  themeConfig: {
    logo: '/logo.svg',
    // Hide the default text title so the BrandMark slot owns the
    // navbar brand. The slot is wired in .vitepress/theme/index.ts
    // (`nav-bar-title-before`).
    siteTitle: false,

    nav: [
      { text: 'Guide', link: '/guide/what-and-why', activeMatch: '/guide/' },
      { text: 'Features', link: '/features/accounts', activeMatch: '/features/' },
      { text: 'Download', link: '/guide/getting-started' },
      {
        text: 'GitHub',
        link: 'https://github.com/xiaolai/claudepot-app',
      },
    ],

    sidebar: {
      '/guide/': [
        {
          text: 'Introduction',
          items: [
            { text: 'What & why', link: '/guide/what-and-why' },
            { text: 'Getting started', link: '/guide/getting-started' },
            { text: 'First run', link: '/guide/first-run' },
          ],
        },
        {
          text: 'Features',
          items: [
            { text: 'Accounts', link: '/features/accounts' },
            { text: 'Activities', link: '/features/activities' },
            { text: 'Projects', link: '/features/projects' },
            { text: 'Keys', link: '/features/keys' },
            { text: 'Third-parties', link: '/features/third-parties' },
            { text: 'Automations', link: '/features/automations' },
            { text: 'Global', link: '/features/global' },
            { text: 'Settings', link: '/features/settings' },
          ],
        },
      ],
      '/features/': [
        {
          text: 'Introduction',
          items: [
            { text: 'What & why', link: '/guide/what-and-why' },
            { text: 'Getting started', link: '/guide/getting-started' },
            { text: 'First run', link: '/guide/first-run' },
          ],
        },
        {
          text: 'Features',
          items: [
            { text: 'Accounts', link: '/features/accounts' },
            { text: 'Activities', link: '/features/activities' },
            { text: 'Projects', link: '/features/projects' },
            { text: 'Keys', link: '/features/keys' },
            { text: 'Third-parties', link: '/features/third-parties' },
            { text: 'Automations', link: '/features/automations' },
            { text: 'Global', link: '/features/global' },
            { text: 'Settings', link: '/features/settings' },
          ],
        },
      ],
    },

    socialLinks: [
      { icon: 'github', link: 'https://github.com/xiaolai/claudepot-app' },
    ],

    footer: {
      message: 'Released under the ISC License.',
      // VitePress renders `copyright` via v-html, so the brand-mark
      // spans pick up the same `.brand-clau` / `.brand-depot` rules
      // as the BrandMark Vue component.
      copyright:
        '<span class="brand-mark"><span class="brand-clau">Clau</span><span class="brand-depot">Depot</span></span> — a control panel for Claude Code and Claude Desktop.',
    },

    search: {
      provider: 'local',
    },

    outline: {
      level: [2, 3],
    },
  },
})
