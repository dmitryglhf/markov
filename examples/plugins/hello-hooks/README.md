# hello-hooks

A tiny [Open Plugins](https://open-plugins.com) plugin that demonstrates markov's
hook system. It registers four event handlers — `SessionStart`,
`UserPromptSubmit`, `PreToolUse` and `PostToolUse` — that each shell out to
`scripts/announce.sh` to print a noticeable line to stderr and append the full
event payload to `last-event.log` next to the plugin. Only `PostToolUse` carries
a matcher (`developer__shell|developer__text_editor`); the other three fire on
everything, which is what makes the plugin useful as a probe.

This fork keeps it as a fixture rather than as documentation: it is the shortest
way to tell whether plugin discovery ran at all, which is exactly the question
that comes up when a plugin silently fails to load.

## Layout

```
hello-hooks/
├── plugin.json
├── hooks/
│   └── hooks.json
└── scripts/
    └── announce.sh
```

## Try it

markov discovers plugins under `~/.agents/plugins/<name>/` (user scope) and
`<project-root>/.agents/plugins/<name>/` (project scope) per the Open Plugins
[installation spec](https://open-plugins.com/plugin-builders/installation#recommended-storage-paths).
Project scope wins when both define the same name. With `GOOSE_PATH_ROOT` set,
the user scope moves to `$GOOSE_PATH_ROOT/.agents/plugins/` — handy for trying
this out without touching a real setup.

```bash
mkdir -p ~/.agents/plugins
cp -R examples/plugins/hello-hooks ~/.agents/plugins/hello-hooks
chmod +x ~/.agents/plugins/hello-hooks/scripts/announce.sh

# Then run markov normally; you should see lines like
# 🚀 [hello-hooks] SessionStart
# 💬 [hello-hooks] UserPromptSubmit
# ⚡ [hello-hooks] PreToolUse tool=developer__shell
# ✅ [hello-hooks] PostToolUse tool=developer__shell
markov session

# Inspect the full payloads markov passed to the hook:
tail ~/.agents/plugins/hello-hooks/last-event.log
```

## Turning it off

Two independent switches, and a plugin stays off if either one says so.

By name, in `settings.json`:

```json
{ "disabledPlugins": ["hello-hooks"] }
```

By path, in the `plugins` map of `config.yaml`, where every discovered plugin
gets an entry keyed by its absolute directory:

```yaml
plugins:
  /Users/you/.agents/plugins/hello-hooks:
    enabled: false
```

Mind where each file lives — the two do not agree in this fork. `config.yaml`
follows the fork identity (`~/.config/markov/` on Linux,
`~/Library/Application Support/ru.postgrespro.markov/` on macOS), while
`user_settings_path()` in `crates/goose/src/plugins/discovery.rs` still hardcodes
`~/.config/goose/settings.json` on every platform instead of going through
`Paths::config_dir()`. So a plugin disabled by name survives a `just reset`,
which only wipes the markov directories.
