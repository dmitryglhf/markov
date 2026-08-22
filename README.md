<div align="center">

# `markov`

_a fork of [Goose](https://github.com/aaif-goose/goose), an open source AI agent that runs on your own machine_

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

markov is licensed under Apache 2.0, the same terms as the project it comes from.
