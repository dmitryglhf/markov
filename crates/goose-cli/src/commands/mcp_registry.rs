//! Reading a public catalogue of MCP servers.
//!
//! The registry stores metadata only: an entry points at a package on npm, PyPI
//! or a container registry, or at somebody else's running server. Everything
//! here reduces one entry to the same command-or-URL string the connect form
//! already accepts, so nothing downstream needs to know a registry exists.

use anyhow::{bail, Result};
use goose::config::Config;
use serde::Deserialize;
use std::time::Duration;

const DEFAULT_REGISTRY: &str = "https://registry.modelcontextprotocol.io/v0.1";

/// Also reads as an environment variable, so a different catalogue can be tried
/// without touching the config.
const REGISTRY_URL_KEY: &str = "GOOSE_MCP_REGISTRY_URL";

const SEARCH_LIMIT: &str = "50";
const SEARCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Our own servers ship with the binary, written in the shape the registry
/// answers with, so they reach the connect form down the same road.
const PGPRO_CATALOGUE: &str = include_str!("pgpro_catalogue.json");

/// A server worth offering: everything unusable has already been dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub name: String,
    pub title: String,
    pub description: String,
    pub options: Vec<Install>,
}

/// One way to run a server. `target` is what the connect form takes verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    pub label: String,
    pub target: String,
    pub secrets: Vec<String>,
    pub plain: Vec<Variable>,
}

/// A setting the server needs that is not worth hiding in the secret store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    pub name: String,
    pub description: String,
    pub default: Option<String>,
}

pub async fn search(query: &str) -> Result<Vec<Candidate>> {
    let url = url::Url::parse_with_params(
        &format!("{}/servers", base_url()),
        &[
            ("search", query),
            ("version", "latest"),
            ("limit", SEARCH_LIMIT),
        ],
    )?;

    let response = reqwest::Client::new()
        .get(url)
        .timeout(SEARCH_TIMEOUT)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        bail!("the registry answered {status}");
    }

    // A catalogue nobody moderates is no place to insist on valid UTF-8.
    let body = response.bytes().await?;
    Ok(candidates(&String::from_utf8_lossy(&body)))
}

/// Our servers live at addresses that differ per installation, so the catalogue
/// names a variable instead of an address. A machine that has no value for it is
/// not offered the server at all: a server nobody can reach is not on offer.
pub fn pgpro() -> Vec<Candidate> {
    candidates(&resolved(PGPRO_CATALOGUE))
        .into_iter()
        .filter_map(|mut candidate| {
            candidate
                .options
                .retain(|install| !install.target.contains(PLACEHOLDER));
            (!candidate.options.is_empty()).then_some(candidate)
        })
        .collect()
}

/// Variables the catalogue asks for and this machine cannot answer, so the
/// dialog can say what is missing instead of showing an empty list.
pub fn pgpro_unset() -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for name in placeholders(PGPRO_CATALOGUE).filter(|name| value_of(name).is_none()) {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

const PLACEHOLDER: &str = "${";

fn placeholders(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(PLACEHOLDER)
        .skip(1)
        .filter_map(|rest| rest.split_once('}'))
        .map(|(name, _)| name.to_string())
}

fn value_of(name: &str) -> Option<String> {
    Config::global()
        .get_param::<String>(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// Substitution happens before parsing, so a value lands wherever the catalogue
/// put the variable. What stays unresolved is left in place and filtered later.
fn resolved(catalogue: &str) -> String {
    let mut parts = catalogue.split(PLACEHOLDER);
    let mut out = String::with_capacity(catalogue.len());
    out.push_str(parts.next().unwrap_or_default());

    for part in parts {
        let Some((name, tail)) = part.split_once('}') else {
            out.push_str(PLACEHOLDER);
            out.push_str(part);
            continue;
        };

        match value_of(name) {
            Some(value) => out.push_str(&value),
            None => out.push_str(&format!("{PLACEHOLDER}{name}}}")),
        }
        out.push_str(tail);
    }

    out
}

fn base_url() -> String {
    Config::global()
        .get_param::<String>(REGISTRY_URL_KEY)
        .unwrap_or_else(|_| DEFAULT_REGISTRY.to_string())
        .trim_end_matches('/')
        .to_string()
}

/// Five versions of the entry schema are in circulation at once, so a strict
/// read of the whole page would let one stranger's record hide every result.
/// Records are parsed one by one and the ones that do not fit are dropped.
fn candidates(body: &str) -> Vec<Candidate> {
    let Ok(page) = serde_json::from_str::<Page>(body) else {
        return Vec::new();
    };

    page.servers
        .into_iter()
        .filter_map(|raw| serde_json::from_value::<Entry>(raw).ok())
        .filter_map(|entry| candidate(entry.server))
        .collect()
}

fn candidate(server: Server) -> Option<Candidate> {
    let options = installs(&server);
    if options.is_empty() {
        return None;
    }

    Some(Candidate {
        title: server
            .title
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| short_name(&server.name)),
        name: server.name,
        description: server.description,
        options,
    })
}

fn installs(server: &Server) -> Vec<Install> {
    let mut options: Vec<Install> = Vec::new();

    for remote in &server.remotes {
        // The retired HTTP+SSE transport cannot be connected, and offering a
        // server that is certain to fail is worse than not offering it.
        if remote.kind != "streamable-http" {
            continue;
        }
        options.push(Install {
            label: format!("remote · {}", remote.url),
            target: remote.url.clone(),
            secrets: secret_names(&remote.headers),
            plain: plain_variables(&remote.headers),
        });
    }

    for package in &server.packages {
        let Some(command) = command_for(package) else {
            continue;
        };
        options.push(Install {
            label: format!("{} · {}", package.registry_type, package.identifier),
            target: command,
            secrets: secret_names(&package.environment_variables),
            plain: plain_variables(&package.environment_variables),
        });
    }

    options
}

/// The published order is runner, its own arguments, the package, then the
/// package's arguments, so the identifier cannot simply be appended.
fn command_for(package: &Package) -> Option<String> {
    let (runner, identifier) = runner_for(package)?;

    let mut parts = runner;
    parts.extend(package.runtime_arguments.iter().filter_map(render_argument));
    parts.push(identifier);
    parts.extend(package.package_arguments.iter().filter_map(render_argument));
    Some(parts.join(" "))
}

fn runner_for(package: &Package) -> Option<(Vec<String>, String)> {
    let runtime = package
        .runtime_hint
        .as_deref()
        .filter(|hint| KNOWN_RUNTIMES.contains(hint))
        .unwrap_or(match package.registry_type.as_str() {
            "npm" => "npx",
            "pypi" => "uvx",
            "oci" => "docker",
            // MCPB bundles are a desktop format, and nuget and cargo entries
            // number in the dozens; neither has a launcher we can spell here.
            _ => return None,
        });

    let version = package.version.as_deref().filter(|v| !v.is_empty());
    Some(match runtime {
        "npx" => (
            vec!["npx".to_string(), "-y".to_string()],
            pinned(&package.identifier, version, '@'),
        ),
        "uvx" => (
            vec!["uvx".to_string()],
            pinned(&package.identifier, version, '@'),
        ),
        "docker" => (
            ["docker", "run", "-i", "--rm"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            pinned(&package.identifier, version, ':'),
        ),
        _ => return None,
    })
}

/// Hints naming a bare interpreter (`python`, `node`, `binary`) describe a
/// checkout, not something installable, so they fall back to the package type.
const KNOWN_RUNTIMES: &[&str] = &["npx", "uvx", "docker"];

/// An identifier that already carries its version is left alone: OCI images
/// usually arrive tagged.
fn pinned(identifier: &str, version: Option<&str>, separator: char) -> String {
    // The `@` opening a scoped npm package is part of its name, not a version.
    match version {
        Some(version) if !identifier.trim_start_matches('@').contains(separator) => {
            format!("{identifier}{separator}{version}")
        }
        _ => identifier.to_string(),
    }
}

fn render_argument(argument: &Argument) -> Option<String> {
    let value = argument
        .value
        .as_deref()
        .or(argument.default.as_deref())
        .filter(|value| !value.is_empty());

    if argument.kind == "named" {
        let name = argument.name.as_deref()?;
        return Some(match value {
            Some(value) => format!("{name} {value}"),
            None => name.to_string(),
        });
    }

    value.map(str::to_string)
}

fn secret_names(variables: &[Input]) -> Vec<String> {
    variables
        .iter()
        .filter(|variable| variable.is_secret)
        .map(|variable| variable.name.clone())
        .collect()
}

fn plain_variables(variables: &[Input]) -> Vec<Variable> {
    variables
        .iter()
        .filter(|variable| variable.is_required && !variable.is_secret)
        .map(|variable| Variable {
            name: variable.name.clone(),
            description: variable.description.clone().unwrap_or_default(),
            default: variable.default.clone(),
        })
        .collect()
}

/// `io.github.someone/postgres-mcp` is precise but unusable as a tool prefix.
pub fn short_name(name: &str) -> String {
    name.rsplit('/').next().unwrap_or(name).to_string()
}

#[derive(Deserialize)]
struct Page {
    #[serde(default)]
    servers: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct Entry {
    server: Server,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Server {
    name: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    packages: Vec<Package>,
    #[serde(default)]
    remotes: Vec<Remote>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Package {
    registry_type: String,
    identifier: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    runtime_hint: Option<String>,
    #[serde(default)]
    runtime_arguments: Vec<Argument>,
    #[serde(default)]
    package_arguments: Vec<Argument>,
    #[serde(default)]
    environment_variables: Vec<Input>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Remote {
    #[serde(rename = "type")]
    kind: String,
    url: String,
    #[serde(default)]
    headers: Vec<Input>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Argument {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    default: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Input {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    is_secret: bool,
    #[serde(default)]
    is_required: bool,
    #[serde(default)]
    default: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn page(servers: &str) -> String {
        format!("{{\"servers\": [{servers}], \"metadata\": {{\"count\": 1}}}}")
    }

    fn only(body: &str) -> Candidate {
        let mut found = candidates(body);
        assert_eq!(found.len(), 1, "expected exactly one usable server");
        found.remove(0)
    }

    #[test]
    fn an_npm_package_becomes_an_npx_command() {
        let found = only(&page(
            r#"{"server": {"name": "io.github.someone/thing", "description": "d",
                "packages": [{"registryType": "npm", "identifier": "@scope/thing",
                              "version": "1.2.3"}]}}"#,
        ));

        assert_eq!(found.options[0].target, "npx -y @scope/thing@1.2.3");
        assert_eq!(found.title, "thing");
    }

    #[test]
    fn a_pypi_package_becomes_a_uvx_command() {
        let found = only(&page(
            r#"{"server": {"name": "a/b", "packages":
                [{"registryType": "pypi", "identifier": "mcp-server-git", "version": "0.9"}]}}"#,
        ));

        assert_eq!(found.options[0].target, "uvx mcp-server-git@0.9");
    }

    #[test]
    fn an_image_that_already_carries_its_tag_is_not_pinned_twice() {
        let found = only(&page(
            r#"{"server": {"name": "a/b", "packages":
                [{"registryType": "oci", "identifier": "ghcr.io/a/b:1.0", "version": "1.0"}]}}"#,
        ));

        assert_eq!(
            found.options[0].target,
            "docker run -i --rm ghcr.io/a/b:1.0"
        );
    }

    #[test]
    fn a_runtime_we_cannot_spell_falls_back_to_the_package_type() {
        let found = only(&page(
            r#"{"server": {"name": "a/b", "packages":
                [{"registryType": "pypi", "identifier": "thing", "version": "2.0",
                  "runtimeHint": "python"}]}}"#,
        ));

        assert_eq!(found.options[0].target, "uvx thing@2.0");
    }

    #[test]
    fn arguments_keep_the_published_order_around_the_package() {
        let found = only(&page(
            r#"{"server": {"name": "a/b", "packages":
                [{"registryType": "npm", "identifier": "thing", "version": "1.0",
                  "runtimeArguments": [{"type": "named", "name": "--package",
                                        "value": "@clize/clize"}],
                  "packageArguments": [{"type": "positional", "value": "serve"},
                                       {"type": "named", "name": "--transport",
                                        "value": "stdio"}]}]}}"#,
        ));

        assert_eq!(
            found.options[0].target,
            "npx -y --package @clize/clize thing@1.0 serve --transport stdio"
        );
    }

    #[test]
    fn an_argument_with_nothing_to_say_is_left_out() {
        let found = only(&page(
            r#"{"server": {"name": "a/b", "packages":
                [{"registryType": "npm", "identifier": "thing",
                  "packageArguments": [{"type": "positional", "name": "path",
                                        "isRequired": true}]}]}}"#,
        ));

        assert_eq!(found.options[0].target, "npx -y thing");
    }

    #[test]
    fn a_remote_server_is_offered_as_its_url() {
        let found = only(&page(
            r#"{"server": {"name": "a/b", "remotes":
                [{"type": "streamable-http", "url": "https://host/mcp"}]}}"#,
        ));

        assert_eq!(found.options[0].target, "https://host/mcp");
    }

    #[test]
    fn a_server_reachable_only_over_the_retired_transport_is_not_offered() {
        assert!(candidates(&page(
            r#"{"server": {"name": "a/b", "remotes":
                [{"type": "sse", "url": "https://host/sse"}]}}"#,
        ))
        .is_empty());
    }

    #[test]
    fn a_bundle_we_cannot_launch_is_not_offered() {
        assert!(candidates(&page(
            r#"{"server": {"name": "a/b", "packages":
                [{"registryType": "mcpb", "identifier": "https://host/x.mcpb"}]}}"#,
        ))
        .is_empty());
    }

    #[test]
    fn an_entry_with_neither_package_nor_address_is_not_offered() {
        assert!(candidates(&page(r#"{"server": {"name": "a/b", "description": "d"}}"#)).is_empty());
    }

    #[test]
    fn a_record_written_to_another_schema_does_not_take_its_neighbours_with_it() {
        let found = candidates(&page(concat!(
            r#"{"server": {"packages": "this is not a list"}},"#,
            r#"{"server": {"name": "a/b", "remotes":
                [{"type": "streamable-http", "url": "https://host/mcp"}]}}"#
        )));

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "a/b");
    }

    #[test]
    fn a_secret_becomes_a_slot_and_a_plain_requirement_becomes_a_question() {
        let found = only(&page(
            r#"{"server": {"name": "a/b", "packages":
                [{"registryType": "npm", "identifier": "thing", "environmentVariables":
                  [{"name": "API_KEY", "isSecret": true, "isRequired": true},
                   {"name": "PROVIDER", "isRequired": true, "description": "openai or anthropic"},
                   {"name": "DEBUG"}]}]}}"#,
        ));

        assert_eq!(found.options[0].secrets, vec!["API_KEY".to_string()]);
        assert_eq!(
            found.options[0].plain,
            vec![Variable {
                name: "PROVIDER".to_string(),
                description: "openai or anthropic".to_string(),
                default: None,
            }]
        );
    }

    #[test]
    fn a_remote_header_is_collected_the_same_way_as_an_environment_variable() {
        let found = only(&page(
            r#"{"server": {"name": "a/b", "remotes":
                [{"type": "streamable-http", "url": "https://host/mcp", "headers":
                  [{"name": "Authorization", "isSecret": true, "isRequired": true}]}]}}"#,
        ));

        assert_eq!(found.options[0].secrets, vec!["Authorization".to_string()]);
    }

    #[test]
    fn every_way_to_run_one_server_is_offered_separately() {
        let found = only(&page(
            r#"{"server": {"name": "a/b",
                "remotes": [{"type": "streamable-http", "url": "https://host/mcp"}],
                "packages": [{"registryType": "npm", "identifier": "thing", "version": "1.0"}]}}"#,
        ));

        assert_eq!(found.options.len(), 2);
        assert_eq!(found.options[0].label, "remote · https://host/mcp");
        assert_eq!(found.options[1].label, "npm · thing");
    }

    #[test]
    fn a_body_that_is_not_a_page_at_all_yields_nothing() {
        assert!(candidates("<html>gateway timeout</html>").is_empty());
    }

    #[test]
    fn the_shipped_catalogue_waits_for_its_addresses() {
        let asked: Vec<String> = placeholders(PGPRO_CATALOGUE).collect();
        assert!(!asked.is_empty(), "the catalogue should not ship empty");

        for name in &asked {
            std::env::remove_var(name);
        }
        assert!(
            pgpro().is_empty(),
            "a server with no address should not be offered"
        );
        assert_eq!(pgpro_unset(), asked);

        for name in &asked {
            std::env::set_var(name, format!("https://{}.example/mcp", name.to_lowercase()));
        }
        let ours = pgpro();
        for name in &asked {
            std::env::remove_var(name);
        }

        assert_eq!(ours.len(), asked.len());
        for candidate in &ours {
            let target = &candidate.options[0].target;
            assert!(target.starts_with("https://"), "unresolved: {target}");
        }
    }

    #[tokio::test]
    async fn the_catalogue_address_can_be_pointed_somewhere_else() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await;

            let body = page(
                r#"{"server": {"name": "local/mirror", "remotes":
                    [{"type": "streamable-http", "url": "https://host/mcp"}]}}"#,
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes()).await;
        });

        std::env::set_var(REGISTRY_URL_KEY, &base);
        let found = search("anything").await;
        std::env::remove_var(REGISTRY_URL_KEY);

        let found = found.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "local/mirror");
    }
}
