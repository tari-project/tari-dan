# AI Coding Agent Skills for Tari Ootle

This folder contains comprehensive development guides ("skills") that give AI coding agents the context they need to generate correct Tari Ootle code — including accurate APIs, boilerplate, working examples, and common pitfalls.

Each skill is available as a `SKILL.md` file in its own directory and is published on the [Tari Ootle documentation site](https://ootle.tari.com/skills/).

## Quick Start: Automatic Discovery

All skills are now exposed via the Agent Skills Discovery endpoint:

```
https://ootle.tari.com/.well-known/skills/
```

AI agents can automatically discover and load skills from this endpoint. No manual setup needed!

## Available Skills

| Directory | Agent | Description |
|-----------|-------|-------------|
| [`claude-code/`](claude-code/) | [Claude Code](https://claude.ai) | Claude Code / Anthropic Claude |
| [`opencode/`](opencode/) | [OpenCode](https://opencode.ai) | OpenCode CLI agent |
| [`cursor/`](cursor/) | [Cursor](https://cursor.com) | Cursor AI editor |
| [`github-copilot/`](github-copilot/) | [GitHub Copilot](https://github.com/features/copilot) | GitHub Copilot coding agent |
| [`windsurf/`](windsurf/) | [Windsurf](https://windsurf.com) | Windsurf (Cognition) |
| [`aider/`](aider/) | [Aider](https://aider.chat) | Aider CLI assistant |
| [`openai-codex/`](openai-codex/) | [OpenAI Codex](https://openai.com/codex) | OpenAI Codex CLI |
| [`amp/`](amp/) | [Amp](https://ampcode.com) | Amp coding agent |
| [`google-gemini/`](google-gemini/) | [Gemini CLI](https://github.com/google-gemini/gemini-cli) | Google Gemini CLI |
| [`antigravity/`](antigravity/) | [Antigravity](https://antigravity.dev) | Antigravity agent |

## Manual Installation (Optional)

If your tool doesn't support automatic skill discovery, you can still copy the skills manually:

### Claude Code

Copy the skill file to your project root (or `.claude/` subdirectory):

```bash
cp docs/skills/claude-code/SKILL.md ./CLAUDE.md
```

Claude Code reads `CLAUDE.md` automatically at the start of every session.

### OpenCode

OpenCode loads skills from its skills directory (`~/.config/opencode/skills/<name>/SKILL.md`). Copy the skill there:

```bash
mkdir -p ~/.config/opencode/skills/tari-ootle
cp docs/skills/opencode/SKILL.md ~/.config/opencode/skills/tari-ootle/SKILL.md
```

### Cursor

Copy into Cursor's rules directory:

```bash
mkdir -p .cursor/rules
cp docs/skills/cursor/SKILL.md .cursor/rules/tari-ootle.md
```

Or place it as `AGENTS.md` in the project root — Cursor reads that too.

### GitHub Copilot

Copy into the `.github/` directory:

```bash
mkdir -p .github
cp docs/skills/github-copilot/SKILL.md .github/copilot-instructions.md
```

Copilot's coding agent reads `.github/copilot-instructions.md` automatically.

### Windsurf

Copy to the project root:

```bash
cp docs/skills/windsurf/SKILL.md .windsurfrules
```

Or use `AGENTS.md` in the project root — Windsurf supports both.

### Aider

Load it as a read-only context file:

```bash
aider --read docs/skills/aider/SKILL.md
```

To load automatically on every run, add to `.aider.conf.yml`:

```yaml
read: docs/skills/aider/SKILL.md
```

### OpenAI Codex

Copy to the project root as `AGENTS.md`:

```bash
cp docs/skills/openai-codex/SKILL.md ./AGENTS.md
```

Codex reads `AGENTS.md` automatically before starting work. You can also place it at `~/.codex/AGENTS.md` for global defaults.

### Amp

Copy to the project root as `AGENTS.md`:

```bash
cp docs/skills/amp/SKILL.md ./AGENTS.md
```

Amp reads `AGENTS.md` automatically from the project root and subdirectories.

### Gemini CLI

Copy to the project root as `AGENTS.md`:

```bash
cp docs/skills/google-gemini/SKILL.md ./AGENTS.md
```

Or configure in `.gemini/settings.json`:

```json
{ "contextFileName": "docs/skills/google-gemini/SKILL.md" }
```

### Antigravity

Follow Antigravity's documentation for loading custom rules, using `docs/skills/antigravity/SKILL.md` as the source.

## What's Covered

Each skill document includes:

- **Overview** — Key concepts (templates, components, resources, vaults, buckets, transactions)
- **Template authoring** — Project setup, `#[template]` macro, constructors, state rules, error handling
- **Resources** — All 4 types (fungible, non-fungible, confidential, stealth), `ResourceBuilder` full API
- **Vault & Bucket** — Complete method reference with examples
- **Access rules** — `rule!` macro syntax, `ComponentAccessRules`, `OwnerRule`, `CallerContext`
- **Cross-component calls** — `ComponentManager::get`, `.call()`, `.invoke()`
- **Events & randomness** — `emit_event`, `random_bytes`, `random_u32`
- **Publishing** — WASM compilation, wallet UI, and programmatic publishing via `ootle-rs`
- **Client-side Rust** — `ootle-rs` provider setup, `TransactionBuilder`, faucet, receipt parsing
- **Testing** — `tari_template_test_tooling` API (`TemplateTest`, `call_function`, `call_method`)
- **Wallet CLI** — `tari_ootle_wallet_cli` commands for accounts, transactions, keys
- **Complete examples** — Counter, token with admin badge, guessing game
- **Common mistakes** — Top 10 pitfalls and how to avoid them
