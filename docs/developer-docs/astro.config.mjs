// @ts-check
import { defineConfig } from "astro/config";
import cloudflare from "@astrojs/cloudflare";
import starlight from "@astrojs/starlight";
import skills from 'astro-skills';

// https://astro.build/config
export default defineConfig({
  site: "https://ootle.tari.com",
  output: "server",
  adapter: cloudflare({
    imageService: "passthrough",
  }),
  base: '/',
  integrations: [
    skills(),
    starlight({
      favicon: "/favicon.png",
      title: "Tari Ootle Playground",
      description: "Build agentic decentralized applications on the Tari Layer 2 network using Rust-based smart contract templates.",
      head: [
        { tag: "meta", attrs: { property: "og:image", content: "https://ootle.tari.com/og-image.png" } },
        { tag: "meta", attrs: { property: "og:site_name", content: "Tari Ootle Playground" } },
        { tag: "meta", attrs: { name: "twitter:card", content: "summary_large_image" } },
        { tag: "meta", attrs: { name: "twitter:image", content: "https://ootle.tari.com/og-image.png" } },
      ],
      customCss: ["./src/styles/global.scss", "./src/styles/custom.scss", "./src/fonts/font-face.css"],
      social: [{ icon: "github", label: "GitHub", href: "https://github.com/tari-project/tari-ootle" }],
      sidebar: [
        {
          label: "Guides",
          items: [
            { label: "Getting Started", link: "/guides/getting-started/" },
            { label: "Setup a Wallet", link: "/guides/setup-a-wallet/" },
            { label: "Templates Overview", link: "/guides/template-overview/" },
            { label: "Building a Guessing Game", link: "/guides/build-a-guessing-game/" },
            { label: "Testing Your Template", link: "/guides/testing-templates/" },
            { label: "Publish the Guessing Game", link: "/guides/publishing-templates/" },
            { label: "Play the Guessing Game", link: "/guides/play-the-guessing-game/" },
            { label: "Transaction Overview", link: "/guides/transaction-overview/" },
            { label: "Tari Cli", link: "/guides/cli/" },
            { label: "Resources", link: "/guides/resources/" },
            { label: "Authorization and Access", link: "/guides/authorization-and-access/" },
            { label: "API Keys for AI Agents", link: "/guides/agent-api-keys/" },
            { label: "Stealth Transfers", link: "/guides/stealth-resources/" },
            { label: "Claim Burn", link: "/guides/claim-burn/" },
            { label: "Randomness in Templates", link: "/guides/randomness/" },
          ],
        },
        {
          label: "Reference",
          items: [{ autogenerate: { directory: "reference" } }],
        },
        { label: "For Bots", link: "/llms.txt", attrs: { target: "_blank" } },
      ],
      components: {
        Pagination: "./src/components/Pagination.astro",
      },
    }),
  ],
});
