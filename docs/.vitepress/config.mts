import { defineConfig } from "vitepress";

// Project Pages site → served under /RepoHarbor/.
export default defineConfig({
  title: "RepoHarbor",
  description: "every repo at harbor — a Linux-native command center for your git fleet",
  base: "/RepoHarbor/",
  lang: "en-US",
  cleanUrls: true,
  lastUpdated: true,
  // The design-system page links into the source tree (../../src/...) — those
  // are meant to be read on GitHub, not as site pages. Ignore them; real
  // cross-page links are still validated.
  ignoreDeadLinks: [/\.\.\/\.\.\/src\//, /^\.\.\/\.\.\//],
  head: [
    ["link", { rel: "icon", href: "/RepoHarbor/logo.svg" }],
    ["meta", { name: "theme-color", content: "#1dd3c4" }],
  ],
  themeConfig: {
    logo: "/logo.svg",
    nav: [
      { text: "Guide", link: "/guide/introduction" },
      { text: "Features", link: "/guide/mission-control" },
      {
        text: "Internals",
        items: [
          { text: "Design system", link: "/design-system/" },
          { text: "Rendering performance", link: "/rendering-performance" },
        ],
      },
      { text: "GitHub", link: "https://github.com/azmykn/RepoHarbor" },
    ],
    sidebar: [
      {
        text: "Guide",
        collapsed: false,
        items: [
          { text: "Introduction", link: "/guide/introduction" },
          { text: "Getting started", link: "/guide/getting-started" },
          { text: "Configuration", link: "/guide/configuration" },
          { text: "Privacy & local data", link: "/guide/privacy" },
        ],
      },
      {
        text: "Feature tour",
        collapsed: false,
        items: [
          { text: "Mission Control", link: "/guide/mission-control" },
          { text: "The repo drawer", link: "/guide/repo-drawer" },
          { text: "Fleet operations", link: "/guide/fleet" },
          { text: "Launchers", link: "/guide/launchers" },
          { text: "Notifications & tray", link: "/guide/notifications" },
          { text: "Maintenance & tools", link: "/guide/maintenance" },
          { text: "Inbox, Feed & Explore", link: "/guide/inbox-feed-explore" },
          { text: "Local AI", link: "/guide/local-ai" },
        ],
      },
      {
        text: "Internals",
        collapsed: true,
        items: [
          { text: "Design system", link: "/design-system/" },
          { text: "Rendering performance", link: "/rendering-performance" },
        ],
      },
    ],
    socialLinks: [{ icon: "github", link: "https://github.com/azmykn/RepoHarbor" }],
    search: { provider: "local" },
    editLink: {
      pattern: "https://github.com/azmykn/RepoHarbor/edit/main/docs/:path",
      text: "Edit this page on GitHub",
    },
    footer: {
      message: "Released under the MIT License. DigitsCode product.",
      copyright: "© 2026 DigitsCode",
    },
  },
});
