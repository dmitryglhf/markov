use markov_cli::commands::ide::jsonc::{backup_path, Document};
use markov_cli::commands::ide::manager::{annotation, decide, verdict, Decision, Tally};
use markov_cli::commands::ide::paths::{BaseDirs, Os};
use markov_cli::commands::ide::targets::{JETBRAINS, VSCODE, ZED};
use markov_cli::commands::ide::{apply, inspect, remove, snippet, Change, Configured, SetupOptions};
use std::fs;
use std::path::{Path, PathBuf};

fn dirs() -> BaseDirs {
    BaseDirs {
        home: PathBuf::from("/home/ivan"),
        xdg_config: PathBuf::from("/home/ivan/.config"),
        apple_data: PathBuf::from("/home/ivan/Library/Application Support"),
        windows_config: PathBuf::from("/home/ivan/AppData/Roaming"),
        windows_local: PathBuf::from("/home/ivan/AppData/Local"),
    }
}

fn options(name: &str) -> SetupOptions {
    SetupOptions {
        name: name.to_string(),
        command: None,
        print: false,
        dry_run: false,
        vsix: None,
    }
}

fn write(dir: &Path, contents: &str) -> PathBuf {
    let path = dir.join("settings.json");
    fs::write(&path, contents).unwrap();
    path
}

#[test]
fn resolves_every_target_on_every_platform() {
    let dirs = dirs();

    assert_eq!(
        dirs.zed_settings(Os::Macos),
        PathBuf::from("/home/ivan/.config/zed/settings.json")
    );
    assert_eq!(
        dirs.zed_settings(Os::Linux),
        PathBuf::from("/home/ivan/.config/zed/settings.json")
    );
    assert_eq!(
        dirs.zed_settings(Os::Windows),
        PathBuf::from("/home/ivan/AppData/Roaming/Zed/settings.json")
    );

    assert_eq!(
        dirs.vscode_settings(Os::Macos),
        PathBuf::from("/home/ivan/Library/Application Support/Code/User/settings.json")
    );
    assert_eq!(
        dirs.vscode_settings(Os::Linux),
        PathBuf::from("/home/ivan/.config/Code/User/settings.json")
    );
    assert_eq!(
        dirs.vscode_settings(Os::Windows),
        PathBuf::from("/home/ivan/AppData/Roaming/Code/User/settings.json")
    );

    for os in [Os::Macos, Os::Linux, Os::Windows] {
        assert_eq!(
            dirs.jetbrains_acp(os),
            PathBuf::from("/home/ivan/.jetbrains/acp.json")
        );
    }
}

#[test]
fn cli_path_matches_what_the_installers_create() {
    let dirs = dirs();
    assert_eq!(
        dirs.cli_binary(Os::Macos),
        PathBuf::from("/home/ivan/.local/bin/markov")
    );
    assert_eq!(
        dirs.cli_binary(Os::Linux),
        PathBuf::from("/home/ivan/.local/bin/markov")
    );
    assert_eq!(
        dirs.cli_binary(Os::Windows),
        PathBuf::from("/home/ivan/markov/markov.exe")
    );
}

#[test]
fn keeps_comments_and_trailing_commas() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        r#"// Zed settings
{
  "ui_font_size": 16,
  "theme": {
    "mode": "system",
  },
}
"#,
    );

    assert_eq!(
        apply(
            &ZED,
            &path,
            &options("markov"),
            "/home/ivan/.local/bin/markov"
        )
        .unwrap(),
        Change::Added
    );

    let written = fs::read_to_string(&path).unwrap();
    assert!(written.contains("// Zed settings"));
    assert!(written.contains("\"mode\": \"system\","));
    assert!(written.contains("\"ui_font_size\": 16"));
    assert!(written.contains("\"agent_servers\""));
    assert!(written.contains("\"command\": \"/home/ivan/.local/bin/markov\""));
}

#[test]
fn escapes_windows_paths() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "{}\n");

    apply(
        &ZED,
        &path,
        &options("markov"),
        r"C:\Users\ivan\markov\markov.exe",
    )
    .unwrap();

    let written = fs::read_to_string(&path).unwrap();
    assert!(written.contains(r#""C:\\Users\\ivan\\markov\\markov.exe""#));

    let reparsed: serde_json::Value = serde_json::from_str(&written).unwrap();
    assert_eq!(
        reparsed["agent_servers"]["markov"]["command"],
        serde_json::json!(r"C:\Users\ivan\markov\markov.exe")
    );
}

#[test]
fn running_twice_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "{}\n");
    let command = "/home/ivan/.local/bin/markov";

    apply(&ZED, &path, &options("markov"), command).unwrap();
    let after_first = fs::read_to_string(&path).unwrap();

    assert_eq!(
        apply(&ZED, &path, &options("markov"), command).unwrap(),
        Change::Unchanged
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), after_first);
}

#[test]
fn a_moved_binary_updates_the_entry() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "{}\n");

    apply(&ZED, &path, &options("markov"), "/old/markov").unwrap();
    assert_eq!(
        apply(&ZED, &path, &options("markov"), "/new/markov").unwrap(),
        Change::Updated
    );

    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        written["agent_servers"]["markov"]["command"],
        serde_json::json!("/new/markov")
    );
}

#[test]
fn creates_a_settings_file_that_is_not_there() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("User").join("settings.json");

    assert_eq!(
        apply(
            &VSCODE,
            &path,
            &options("markov"),
            "/home/ivan/.local/bin/markov"
        )
        .unwrap(),
        Change::Added
    );

    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        written["acp.agents"]["markov"]["args"],
        serde_json::json!(["acp"])
    );
    assert!(!backup_path(&path).exists());
}

#[test]
fn leaves_other_agents_alone() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        r#"{
  "acp.agents": {
    "Claude Code": {
      "command": "npx",
      "args": ["@agentclientprotocol/claude-agent-acp@latest"],
      "env": {}
    }
  },
  "acp.logTraffic": true
}
"#,
    );

    apply(
        &VSCODE,
        &path,
        &options("markov"),
        "/home/ivan/.local/bin/markov",
    )
    .unwrap();

    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        written["acp.agents"]["Claude Code"]["command"],
        serde_json::json!("npx")
    );
    assert_eq!(written["acp.logTraffic"], serde_json::json!(true));
    assert_eq!(
        written["acp.agents"]["markov"]["command"],
        serde_json::json!("/home/ivan/.local/bin/markov")
    );
}

#[test]
fn backs_the_original_up_before_writing() {
    let dir = tempfile::tempdir().unwrap();
    let original = "{\n  \"ui_font_size\": 16\n}\n";
    let path = write(dir.path(), original);

    apply(
        &ZED,
        &path,
        &options("markov"),
        "/home/ivan/.local/bin/markov",
    )
    .unwrap();

    assert_eq!(fs::read_to_string(backup_path(&path)).unwrap(), original);
}

#[test]
fn refuses_when_the_container_is_not_an_object() {
    let dir = tempfile::tempdir().unwrap();
    let original = "{\n  \"agent_servers\": \"nonsense\"\n}\n";
    let path = write(dir.path(), original);

    let err = apply(
        &ZED,
        &path,
        &options("markov"),
        "/home/ivan/.local/bin/markov",
    )
    .unwrap_err();
    assert!(err.to_string().contains("agent_servers"));
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
}

#[test]
fn refuses_to_touch_broken_json() {
    let dir = tempfile::tempdir().unwrap();
    let original = "{ this is not json";
    let path = write(dir.path(), original);

    let err = apply(
        &ZED,
        &path,
        &options("markov"),
        "/home/ivan/.local/bin/markov",
    )
    .unwrap_err();
    assert!(err.to_string().contains("not valid JSON"));
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
}

#[test]
fn an_empty_file_is_not_a_broken_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "");

    assert_eq!(
        apply(
            &JETBRAINS,
            &path,
            &options("markov"),
            "/home/ivan/.local/bin/markov"
        )
        .unwrap(),
        Change::Added
    );

    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        written["agent_servers"]["markov"]["env"],
        serde_json::json!({})
    );
}

#[test]
fn dry_run_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let original = "{}\n";
    let path = write(dir.path(), original);

    let mut opts = options("markov");
    opts.dry_run = true;

    assert_eq!(
        apply(&ZED, &path, &opts, "/home/ivan/.local/bin/markov").unwrap(),
        Change::Added
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert!(!backup_path(&path).exists());
}

#[test]
fn removal_takes_only_our_entry() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        r#"{
  "agent_servers": {
    "Gemini": { "command": "npx", "args": [] }
  }
}
"#,
    );

    apply(
        &ZED,
        &path,
        &options("markov"),
        "/home/ivan/.local/bin/markov",
    )
    .unwrap();
    assert!(remove(&ZED, &path, "markov").unwrap());

    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert!(written["agent_servers"]["markov"].is_null());
    assert_eq!(
        written["agent_servers"]["Gemini"]["command"],
        serde_json::json!("npx")
    );

    assert!(!remove(&ZED, &path, "markov").unwrap());
}

#[test]
fn removal_ignores_a_file_that_is_not_there() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!remove(&ZED, &dir.path().join("settings.json"), "markov").unwrap());
}

#[test]
fn status_tells_our_entry_from_someone_elses() {
    let dir = tempfile::tempdir().unwrap();
    let path = write(dir.path(), "{}\n");
    let command = "/home/ivan/.local/bin/markov";

    assert_eq!(inspect(&ZED, &path, "markov", command), Configured::Missing);

    apply(&ZED, &path, &options("markov"), command).unwrap();
    assert_eq!(inspect(&ZED, &path, "markov", command), Configured::Ours);

    assert_eq!(
        inspect(&ZED, &path, "markov", "/somewhere/else/markov"),
        Configured::Other("/home/ivan/.local/bin/markov".to_string())
    );
}

#[test]
fn the_printed_snippet_is_valid_json() {
    for target in [&ZED, &JETBRAINS, &VSCODE] {
        let text = snippet(target, "markov", r"C:\Users\ivan\markov\markov.exe").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        let container = target
            .container
            .iter()
            .fold(&parsed, |value, key| &value[*key]);
        assert_eq!(
            container["markov"]["command"],
            serde_json::json!(r"C:\Users\ivan\markov\markov.exe")
        );
    }
}

#[test]
fn a_document_is_untouched_when_only_read() {
    let dir = tempfile::tempdir().unwrap();
    let original = "// keep me\n{\n  \"a\": 1,\n}\n";
    let path = write(dir.path(), original);

    let doc = Document::load(&path).unwrap();
    assert!(doc.container(&["agent_servers"]).is_none());
    assert_eq!(doc.to_text(), original);
}

#[test]
fn a_row_that_is_simply_connected_says_nothing_extra() {
    assert_eq!(annotation(&Configured::Ours, true, false, false), "");
    assert_eq!(annotation(&Configured::Missing, true, false, false), "");
}

#[test]
fn an_unreadable_file_outranks_every_other_remark() {
    let broken = Configured::Unreadable("not valid JSON".to_string());
    assert_eq!(
        annotation(&broken, false, true, true),
        "settings file is not valid JSON"
    );
}

#[test]
fn a_flatpak_outranks_a_missing_install() {
    assert_eq!(
        annotation(&Configured::Missing, false, true, false),
        "flatpak — cannot be connected"
    );
    assert_eq!(
        annotation(&Configured::Missing, false, false, true),
        "not installed"
    );
}

#[test]
fn a_drifted_entry_shows_where_it_points() {
    assert_eq!(
        annotation(
            &Configured::Other("/opt/markov".to_string()),
            true,
            false,
            true
        ),
        "points at /opt/markov"
    );
}

#[test]
fn a_missing_extension_is_the_last_thing_worth_saying() {
    assert_eq!(
        annotation(&Configured::Ours, true, false, true),
        "extension will be installed"
    );
}

#[test]
fn ticking_a_box_connects_and_unticking_disconnects() {
    assert_eq!(decide(true, false, false), Decision::Connect);
    // Already connected and still ticked: applied anyway, which repairs a path
    // that has drifted since it was written.
    assert_eq!(decide(true, true, false), Decision::Connect);
    assert_eq!(decide(false, true, false), Decision::Disconnect);
    assert_eq!(decide(false, false, false), Decision::Leave);
}

#[test]
fn the_last_line_never_calls_a_failure_a_success() {
    assert_eq!(
        verdict(&Tally {
            changes: 0,
            skipped: 0
        }),
        "Saved, nothing was different"
    );
    assert_eq!(
        verdict(&Tally {
            changes: 0,
            skipped: 1
        }),
        "1 left unchanged"
    );
    assert_eq!(
        verdict(&Tally {
            changes: 2,
            skipped: 0
        }),
        "2 change(s) saved"
    );
    assert_eq!(
        verdict(&Tally {
            changes: 1,
            skipped: 1
        }),
        "1 change(s) saved, 1 left unchanged"
    );
}

#[test]
fn a_blocked_target_is_never_written_to() {
    assert_eq!(decide(true, false, true), Decision::Blocked);
    assert_eq!(decide(true, true, true), Decision::Blocked);
    assert_eq!(decide(false, true, true), Decision::Disconnect);
}
