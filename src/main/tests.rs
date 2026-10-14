// @feature runtime
// @feature agent-memory
// @feature papercuts
// @feature sos
// @spec docs/features/runtime.md
// @spec docs/features/agent-memory.md
// @spec docs/features/papercuts.md
// @spec docs/features/sos.md
use super::*;

#[test]
fn sessions_cli_accepts_an_explicit_project_scope() {
    let cli = Cli::try_parse_from([
        "nexus",
        "sessions",
        "--project-id",
        "organization/other-repository",
        "--all",
    ])
    .unwrap();
    let Command::Sessions { project_id, all } = cli.command else {
        panic!("expected sessions command");
    };
    assert_eq!(project_id.as_deref(), Some("organization/other-repository"));
    assert!(all);
}

#[test]
fn memory_cli_requires_write_scope_and_defaults_reads_to_layered() {
    let add = Cli::try_parse_from([
        "nexus",
        "memory",
        "add",
        "--scope",
        "project",
        "Prefer typed APIs",
    ])
    .unwrap();
    let Command::Memory {
        command: MemoryCommand::Add { scope, content },
    } = add.command
    else {
        panic!("expected memory add command");
    };
    assert_eq!(scope, MemoryScopeCli::Project);
    assert_eq!(content, "Prefer typed APIs");

    let context = Cli::try_parse_from(["nexus", "memory", "context"]).unwrap();
    let Command::Memory {
        command: MemoryCommand::Context { scope },
    } = context.command
    else {
        panic!("expected memory context command");
    };
    assert_eq!(scope, MemoryReadScopeCli::Layered);

    assert!(Cli::try_parse_from(["nexus", "memory", "add", "No scope"]).is_err());
}

#[test]
fn papercuts_cli_exposes_typed_actions_and_unbounded_dump_scope() {
    let report = Cli::try_parse_from([
        "nexus",
        "papercuts",
        "report",
        "Flaky socket test",
        "--description",
        "Intermittently times out",
    ])
    .unwrap();
    let Command::Papercuts {
        command: PapercutsCommand::Report { title, description },
    } = report.command
    else {
        panic!("expected papercuts report command");
    };
    assert_eq!(title, "Flaky socket test");
    assert_eq!(description, "Intermittently times out");

    let dump = Cli::try_parse_from(["nexus", "papercuts", "dump", "--all-projects"]).unwrap();
    let Command::Papercuts {
        command: PapercutsCommand::Dump { all_projects },
    } = dump.command
    else {
        panic!("expected papercuts dump command");
    };
    assert!(all_projects);

    let response = Cli::try_parse_from([
        "nexus",
        "papercuts",
        "respond",
        "review-1",
        "--reporter-token",
        "secret-token",
        "--decision",
        "approve",
        "--response",
        "Same underlying failure",
    ])
    .unwrap();
    let Command::Papercuts {
        command: PapercutsCommand::Respond { reporter_token, .. },
    } = response.command
    else {
        panic!("expected papercut response command");
    };
    assert_eq!(reporter_token, "secret-token");
    assert!(Cli::try_parse_from([
        "nexus",
        "papercuts",
        "respond",
        "review-1",
        "--decision",
        "approve",
        "--response",
        "Same underlying failure",
    ])
    .is_err());
}

#[test]
fn sos_cli_accepts_full_reports_and_issue_lifecycle_actions() {
    let report = Cli::try_parse_from([
        "nexus",
        "sos",
        "report",
        "The release pipeline is wedged",
        "--type",
        "build_failure",
        "--environment",
        "ci-linux",
        "--version",
        "1.7.0",
        "--model",
        "gpt-6",
        "--trace",
        "step=publish\nexit=137",
        "--metadata-json",
        r#"{"job":"release","attempt":3}"#,
    ])
    .unwrap();
    let Command::Sos {
        command:
            SosCliCommand::Report {
                message,
                kind,
                environment,
                version,
                model,
                trace,
                metadata_json,
            },
    } = report.command
    else {
        panic!("expected SOS report command");
    };
    assert_eq!(message, "The release pipeline is wedged");
    assert_eq!(kind, "build_failure");
    assert_eq!(environment, "ci-linux");
    assert_eq!(version, "1.7.0");
    assert_eq!(model, "gpt-6");
    assert_eq!(trace, "step=publish\nexit=137");
    assert_eq!(metadata_json, r#"{"job":"release","attempt":3}"#);

    let get = Cli::try_parse_from(["nexus", "sos", "get", "sos-42"]).unwrap();
    assert!(matches!(
        get.command,
        Command::Sos {
            command: SosCliCommand::Get { issue_id }
        } if issue_id == "sos-42"
    ));

    let dump = Cli::try_parse_from(["nexus", "sos", "dump", "--all-projects"]).unwrap();
    assert!(matches!(
        dump.command,
        Command::Sos {
            command: SosCliCommand::Dump { all_projects: true }
        }
    ));

    let resolve = Cli::try_parse_from([
        "nexus",
        "sos",
        "resolve",
        "sos-42",
        "--resolution",
        "Rolled back the broken image",
    ])
    .unwrap();
    assert!(matches!(
        resolve.command,
        Command::Sos {
            command: SosCliCommand::Resolve { issue_id, resolution }
        } if issue_id == "sos-42" && resolution == "Rolled back the broken image"
    ));

    let deprecate = Cli::try_parse_from([
        "nexus",
        "sos",
        "deprecate",
        "sos-42",
        "--reason",
        "Reporter was running an unsupported version",
    ])
    .unwrap();
    assert!(matches!(
        deprecate.command,
        Command::Sos {
            command: SosCliCommand::Deprecate { issue_id, reason }
        } if issue_id == "sos-42" && reason == "Reporter was running an unsupported version"
    ));
}
