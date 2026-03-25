# Hammurabi

> **Logic over Implementation. Proof over Testing.**
> ***The Constitution for AI, by AI, for AI."***
> An AI-native language system that proves logic *before* you write code.
> Hammurabi is not just a programming language; it's a mathematical proof for navigating uncertain times.

[日本語 README](./README.md)

[![Rust](https://img.shields.io/badge/language-Rust-orange?logo=rust)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue)](./LICENSE)
[![Build](https://img.shields.io/badge/build-passing-brightgreen)](#getting-started)
[![Z3](https://img.shields.io/badge/SMT_solver-Z3-purple)](https://github.com/Z3Prover/z3)

---

## Why Hammurabi

In an era where AI writes code, the question is no longer “can it write code?” but “**can we prove that code is correct?**”

Hammurabi shifts the programming paradigm from

```
“How to implement”  →  “What must hold”
```

It is a **logic-first language foundation**.  
You do not spell out functions with `if` chains; you write **constraints** in predicate logic.  
Before execution, the Z3 SMT solver can mathematically prove properties such as exhaustiveness of branches.

The name comes from the ancient Babylonian **Code of Hammurabi** — we want code to read like law: rules without ambiguity.

---

## Core concepts

Hammurabi is built on three principles (its **constitution**).

| # | Principle | Meaning |
|---|-------------|---------|
| 1 | **The "What" Interface** | Functions specify *what* must hold using predicates; imperative `if`-based implementations are ruled out |
| 2 | **Zero-Ambiguity** | All branches are meant to be proven exhaustive at the specification stage (via Z3) |
| 3 | **Hardware-Level Determinism** | Memory safety is a logical consequence, not only a runtime check |

---

## Getting Started

### Install

```bash
# After crates.io release (see roadmap)
# cargo install hammurabi --features full

# Local development (from a clone)
git clone https://github.com/hammurabi-lang/hammurabi.git
cd hammurabi
cargo install --path . --features full   # Example: AI + Z3
```

### Initialize a project

```bash
# Generate config.hb and .env.example
hb init

# Set API keys
cp .env.example .env
# Edit .env and set OPENAI_API_KEY and/or ANTHROPIC_API_KEY
```

### Enabling the Z3 backend

```bash
# macOS
brew install z3

cargo build --features z3-backend
```

---

## CLI — `hb`

### Subcommands

Each command takes **one `.hb` file** as an argument (if you pass a shell glob, only the **first** expanded path is used).

```
hb gen  <file.hb>   [OPTIONS]   Generate implementation code from contracts in .hb
hb ai   "<prompt>"  [OPTIONS]   Generate goals with AI, then generate code
hb init [--force]               Create config.hb / .env.example
hb check <file.hb>  [OPTIONS]   Syntax check + contract verification (no codegen)
```

### Shared options (`gen` / `ai`)

```
--config  <path>   Path to config.hb (auto-detected in cwd if omitted)
--agent   <name>   openai | anthropic | mock
--api-key <key>    API key (resolved from .env / env if omitted)
--model   <name>   e.g. gpt-4o, claude-3-5-sonnet-20241022
--lang    <lang>   rust | python | go | java | javascript | typescript
--verifier <name>  mock (default) | z3 (requires z3-backend feature)
```

`--verifier` applies to **`hb gen` and `hb check`**. For `hb ai` it is parsed but **not** used in the generation flow yet; run `hb check` separately if you need verification after generation.

### `check` — verify contracts with Mock or Z3

Only **`mock`** and **`z3`** are valid for `--verifier` (model names belong in `--model`).

```bash
hb check test.hb --verifier z3
hb check test.hb --verifier mock
```

### Configuration precedence

```
CLI args  >  settings inside .hb  >  config.hb  >  .env  >  environment variables
```

### Examples

```bash
# Generate from a .hb file
hb gen test.hb
hb gen test.hb --lang python
# Prove specs with Z3 before codegen (requires: cargo build --features z3-backend)
hb gen test.hb --verifier z3

# AI goal + code (Mock, no API key)
hb ai "Safely divide two integers. Divisor must not be zero." --agent mock

# AI goal + code (OpenAI / Anthropic needs the `ai` feature)
hb ai "Validate an email address." --agent openai --lang typescript

# Syntax + contract check (Mock or Z3)
hb check test.hb
hb check test.hb --verifier z3

# Project init (use --force to overwrite existing files)
hb init
hb init --force
```

### Cargo features (building the `hb` binary)

Default `cargo build` is **minimal** (Mock only; no external AI / Z3). Enable features when you need OpenAI, Anthropic, or Z3.

```bash
cargo build                              # Minimal (Mock)
cargo build --features ai                # OpenAI / Anthropic
cargo build --features z3-backend        # enables `--verifier z3`
cargo build --features full              # ai + z3-backend
cargo install --path . --features full # Example: full local install
```

---

## `.hb` file format

A small DSL for Hammurabi. Declaratively describes `ContractualGoal` (logical specification of a function).

```hb
// File-level settings (before goal blocks)
// agent:   openai              // openai | anthropic | mock
// model:   gpt-4o              // default per agent if omitted
// lang:    python              // rust | python | go | java | javascript | typescript
// api_key: $OPENAI_API_KEY    // prefer .env

// Goal block (multiple per file allowed)
goal: safe_division
  require:   Or(InRange(divisor, -9223372036854775808, -1), InRange(divisor, 1, 9223372036854775807))
  require:   InRange(dividend, -9223372036854775808, 9223372036854775807)
  ensure:    result_is_finite
  ensure:    result_within_i64_range
  invariant: no_memory_aliasing
  forbid:    RuntimeNullCheck
  forbid:    UnprovenUnwrap
```

### Predicates

| Predicate | Meaning |
|-----------|---------|
| `NonNull(x)` | Variable `x` is not null |
| `InRange(x, min, max)` | `x` lies in `[min, max]` |
| `Or(p1, p2)` | Predicate `p1` or `p2` holds |
| `<atom>` | Atomic predicate (interpreted by the Verifier) |

### Forbidden patterns (`forbid`)

| Pattern | Meaning |
|---------|---------|
| `RuntimeNullCheck` | No ad-hoc runtime null checks (`if x == nil`) |
| `UnprovenUnwrap` | No `unwrap()` / `!` without proof |
| `NonExhaustiveBranch` | Non-exhaustive branches forbidden |
| `CatchAllSuppression` | No catch-all suppression (`_ =>`, etc.) |

---

## Architecture

```
hammurabi/
├── src/
│   ├── lang/
│   │   ├── goal.rs          # ContractualGoal — predicate-logic spec for AI
│   │   └── rail.rs          # LogicRail<T> — proof-sealed container
│   ├── compiler/
│   │   └── verifier.rs      # Verifier trait — constitutional checker
│   ├── codegen.rs             # ContractualGoal → multi-language skeletons
│   ├── ai_gen/
│   │   ├── mod.rs             # AiGoalGenerator — natural language → ContractualGoal
│   │   └── client.rs          # OpenAI / Anthropic HTTP client
│   ├── config.rs              # config.hb / .env parser
│   ├── lsp/
│   │   └── mod.rs             # .hb parser + LSP (completion, diagnostics, hover)
│   ├── proof_store.rs         # ProofToken persistence and signature checks
│   ├── math.rs                # Math utilities
│   └── wasm.rs                # WASM bindings (feature: wasm)
├── src/bin/
│   ├── hb.rs                  # Main CLI (gen / ai / init / check)
│   ├── run_hb.rs              # Legacy CLI (backward compatible)
│   └── hammurabi_lsp.rs       # LSP server binary
├── config.hb                  # AI agent settings
├── .env.example               # Environment variable template
└── test.hb                    # Sample .hb file
```

### Data flow

#### `hb gen` — codegen from `.hb`

```
[.hb file]  goal blocks
    │
    ▼
[LSP parser]  parse_hb() — text → Vec<ContractualGoal>
    │  syntax issues → warnings / errors
    ▼
[Verifier]  Only when `--verifier z3`: prove specs with Z3 (abort codegen on failure)
    │  Default mock: gen does not verify here (use `hb check` anytime)
    ▼
[CodeWriter]  ContractualGoal → per-language skeleton
    │          Non-mock agents: AI may emit full implementations
    ▼
[Output]  Rust / Python / Go / Java / JavaScript / TypeScript
```

#### `hb ai` — goals from natural language

```
[Natural language]  e.g. "Safely divide two integers."
    │
    ▼
[AiGoalGenerator]  OpenAI / Anthropic / Mock
    │  emits ContractualGoal in .hb form
    ▼
[LSP parser]  parse_hb() → Vec<ContractualGoal>
    │
    ▼
[CodeWriter]  Skeleton or AI-generated implementation
```

---

## Three core components

### 1. `ContractualGoal` — the spec you hand to AI

Describe *what* must hold without imperative noise.

```rust
use hammurabi::lang::goal::{ContractualGoal, ForbiddenPattern, Predicate};

let goal = ContractualGoal::new("safe_division")
    .require(Predicate::or(
        Predicate::in_range("divisor", i64::MIN, -1),
        Predicate::in_range("divisor",  1, i64::MAX),
    ))
    .ensure(Predicate::atom("result_is_finite"))
    .invariant(Predicate::atom("no_memory_aliasing"))
    .forbid(ForbiddenPattern::RuntimeNullCheck);
```

You do **not** write `if divisor == 0 { return Err(...) }` — divisors are constrained at the specification level.

---

### 2. `LogicRail<T>` — proof-sealed container

Without a `ProofToken`, `LogicRail<T>` cannot be constructed. Unproven values do not exist.

```rust
use hammurabi::lang::rail::{Constraint, LogicRail};
use hammurabi::compiler::verifier::MockVerifier;

let verifier = MockVerifier::default();

let divisor = LogicRail::bind(
    "divisor",
    5_i64,
    vec![
        Constraint::NonNull,
        Constraint::InRange { min: 1, max: i64::MAX },
    ],
    &verifier,
)?;

let result = dividend.map(
    "result",
    |d| d / *divisor.extract(),
    vec![Constraint::InRange { min: i64::MIN, max: i64::MAX }],
    &verifier,
)?;

println!("{}", result.proof()); // ProofToken[Mock✓|hash=...]
```

---

### 3. `Verifier` — constitutional checker

```rust
pub trait Verifier {
    fn verify_constraints<T: Debug>(
        &self, value: &T, constraints: &[Constraint],
    ) -> Result<ProofToken, VerificationError>;

    fn verify_goal(
        &self, goal: &ContractualGoal,
    ) -> Result<ConstitutionalReport, VerificationError>;

    fn prove_invariant(
        &self, preconditions: &[Predicate], invariant: &Predicate,
    ) -> Result<ProofStatus, VerificationError>;
}
```

Errors are an **exhaustive enum** — no catch-all `_` patterns.

```rust
pub enum VerificationError {
    Unsatisfiable        { predicate: String },
    NonExhaustiveBranch  { context: String },
    ProofTampered        { expected: u64, got: u64 },
    ForbiddenPatternDetected { pattern: String },
    MalformedGoal        { reason: String },
    SolverError          { detail: String },
    ConstitutionViolation { article: u8, detail: String },
}
```

---

## AI agents

| Agent | `--agent` | Feature | Notes |
|-------|-----------|---------|-------|
| `MockAiGenerator` | `mock` | none | Keyword-based, offline (dev / tests) |
| `OpenAiGenerator` | `openai` | `ai` | GPT-based ContractualGoal / code generation |
| `AnthropicGenerator` | `anthropic` | `ai` | Claude-based ContractualGoal / code generation |

`Cargo.toml` has **empty** default features. Build with **`cargo build --features ai`** (or `full`) for OpenAI / Anthropic. Use `--agent mock` without API keys.

---

## Codegen target languages

| Language | `--lang` | Extension |
|----------|----------|-----------|
| Rust | `rust` | `.rs` |
| Python | `python` | `.py` |
| Go | `go` | `.go` |
| Java | `java` | `.java` |
| JavaScript | `javascript` | `.js` |
| TypeScript | `typescript` | `.ts` |

---

## LSP — `hammurabi_lsp`

Language Server Protocol for `.hb` files: completion, diagnostics, hover.

```bash
cargo build --bin hammurabi_lsp
```

Register the binary as an LSP server in your editor (VS Code, etc.) for `.hb` editing support.

---

## Roadmap

| Status | Item |
|--------|------|
| ✅ | `ContractualGoal` — predicate-logic specifications |
| ✅ | `LogicRail<T>` — proof-sealed container |
| ✅ | `MockVerifier` — dev backend |
| ✅ | `Z3Verifier` — SMT backend (`z3-backend` feature) |
| ✅ | `ProofToken` persistence and signatures (`proof_store`) |
| ✅ | `.hb` DSL parser (shared with LSP) |
| ✅ | Multi-language skeleton codegen from `ContractualGoal` (6 languages) |
| ✅ | `AiGoalGenerator` — natural language → ContractualGoal (OpenAI / Anthropic / Mock) |
| ✅ | Settings via `config.hb` / `.env` |
| ✅ | `hb` CLI — subcommands (`gen` / `ai` / `init` / `check`) |
| ✅ | LSP server — completion, diagnostics, hover for `.hb` |
| ✅ | Browser build via WASM (`wasm` feature) |
| ⏳ | Publish VS Code extension |
| ⏳ | Z3 encoding for `ForAll` / `Exists` quantifiers |
| ⏳ | New `Constraint` types (e.g. regex) |
| ⏳ | Publish on crates.io |

---

## Contributing

Issues, pull requests, and stars are welcome.

We especially appreciate help with:

- **Richer predicate logic** — Z3 encoding for `ForAll` / `Exists`
- **New `Constraint` types** — regex, type-class-like constraints, etc.
- **More tests** — edge cases and counterexamples
- **Documentation** — design articles, tutorials

```bash
git checkout -b feature/your-idea
cargo test   # Ensure tests pass before opening a PR
```

---

## Further reading

| File | Description |
|------|-------------|
| [CLAUDE.md](./CLAUDE.md) | Guide for AI assistants (Claude) working on this repo |
| [config.hb](./config.hb) | Template AI agent configuration |
| [.env.example](./.env.example) | Environment variable template |
| [test.hb](./test.hb) | Sample `.hb` file |
| `src/lang/goal.rs` | `ContractualGoal` and `Predicate` |
| `src/lang/rail.rs` | `LogicRail<T>` and `ProofToken` |
| `src/compiler/verifier.rs` | `Verifier` trait and Z3 backend |
| `src/codegen.rs` | Multi-language code generator |
| `src/ai_gen/mod.rs` | AI goal generation pipeline |
| `src/config.rs` | `config.hb` / `.env` parsing |
| `src/lsp/mod.rs` | `.hb` parser and LSP |

---

## License

MIT — see [LICENSE](./LICENSE).
