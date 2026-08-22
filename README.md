<div align="center">

# `markov`

<<<<<<< HEAD
_a fork of [Goose](https://github.com/aaif-goose/goose), an open source AI agent that runs on your own machine_
=======
_your native open source AI agent — desktop app, CLI, and API — for code, workflows, and everything in between_

<p align="center">
  <a href="https://opensource.org/licenses/Apache-2.0"
    ><img src="https://img.shields.io/badge/License-Apache_2.0-blue.svg"></a>
  <a href="https://discord.gg/n8R5VaWDAn"
    ><img src="https://img.shields.io/discord/1287729918100246654?logo=discord&logoColor=white&label=Join+Us&color=blueviolet" alt="Discord"></a>
  <a href="https://github.com/aaif-goose/goose/actions/workflows/ci.yml"
     ><img src="https://img.shields.io/github/actions/workflow/status/aaif-goose/goose/ci.yml?branch=main" alt="CI"></a>
  <a href="https://insights.linuxfoundation.org/project/goose"><img src="https://insights.linuxfoundation.org/api/badge/health-score?project=goose"></a>
  <a href="https://repology.org/project/goose-cli/versions"><img src="https://repology.org/badge/tiny-repos/goose-cli.svg" alt="Packaging status"></a>
</p>

<a href="https://trendshift.io/repositories/25298?utm_source=repository-badge&amp;utm_medium=badge&amp;utm_campaign=badge-repository-25298" target="_blank" rel="noopener noreferrer"><img src="https://trendshift.io/api/badge/repositories/25298" alt="aaif-goose%2Fgoose | Trendshift" width="250" height="55"/></a>
>>>>>>> v1.47.0

</div>

Markov is a general purpose agent. It reads and writes files, runs commands, and drives the tools you already have, whether the work is code, research, automation or ordinary text.

It ships in two shapes. There is a desktop application for macOS, Linux and Windows, and there is a command line interface for people who live in a terminal. Both are built from the same Rust core, so a session started in one is readable by the other.

Models are reached through a provider layer that covers the usual hosted services as well as local servers such as Ollama. External tools arrive over the Model Context Protocol, which means an extension written for another agent works here without changes.

## Install

The installer places the CLI, and on macOS and Linux the desktop application as well, inside your home directory, so it needs no administrator rights. Everything is checked against published checksums before it is unpacked.

```bash
curl -fsSL https://github.com/dmitryglhf/markov/releases/download/stable/install.sh | bash
```

On Windows the same job is done from PowerShell.

```powershell
irm https://github.com/dmitryglhf/markov/releases/download/stable/install.ps1 | iex
```

A piped script takes no arguments of its own, so pass them through the shell instead.

```bash
curl -fsSL https://github.com/dmitryglhf/markov/releases/download/stable/install.sh | bash -s -- --cli-only
```

<<<<<<< HEAD
The same script removes what it installed when given `--uninstall`.

## Build it yourself

Development dependencies come from hermit, so a fresh clone needs one line before anything else.

```bash
source bin/activate-hermit
just release
```

That leaves a `markov` binary in `target/release`. Running `just install` builds the same binary and puts it on your PATH, while `just` on its own lists everything else the repository can do.

## Documentation

Guides for the agent itself live in the upstream project and are kept current there, so what is written about providers, extensions, recipes and the desktop application holds for this fork too. Anything specific to this fork is documented next to the code it describes.
=======
# goose around with us
- [Discord](https://discord.gg/n8R5VaWDAn)
- [YouTube](https://www.youtube.com/@goose-oss)
- [LinkedIn](https://www.linkedin.com/company/goose-oss)
- [Twitter/X](https://x.com/goose_oss)
>>>>>>> v1.47.0
