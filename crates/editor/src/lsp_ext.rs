use std::sync::Arc;
use std::time::Duration;

use crate::{Editor, ExecuteLspCommand};
use anyhow::{Context as _, Result, anyhow};
use collections::{HashMap, HashSet};
use futures::TryFutureExt;
use gpui::{App, Entity, Task};
use gpui::{AsyncApp, FutureExt};
use language::Buffer;
use language::Language;
use lsp::LanguageServerId;
use lsp::LanguageServerName;
use project::LanguageServerToQuery;
use project::LocationLink;
use project::Project;
use project::TaskSourceKind;
use project::lsp_store::lsp_ext_command::GetLspRunnables;
use task::ResolvedTask;
use task::TaskContext;
use text::{BufferId, ToPointUtf16 as _};
use ui::SharedString;
use util::ResultExt as _;

impl Editor {
    pub(crate) fn execute_lsp_command(
        &self,
        action: &ExecuteLspCommand,
        cx: &mut App,
    ) -> Task<Result<Option<serde_json::Value>>> {
        let result = (|| {
            let project = self
                .project
                .clone()
                .context("cannot execute an LSP command without a project")?;
            let multi_buffer = self.buffer.read(cx);
            let multi_buffer_snapshot = multi_buffer.snapshot(cx);
            let (buffer_position, _) = multi_buffer_snapshot
                .anchor_to_buffer_anchor(self.selections.newest_anchor().head())
                .context("the current selection is not in a language buffer")?;
            let buffer = multi_buffer
                .buffer(buffer_position.buffer_id)
                .context("the current language buffer is no longer available")?;

            let position = buffer_position.to_point_utf16(&buffer.read(cx).snapshot());
            let project_ref = project.read(cx);
            let lsp_store = project_ref.lsp_store().read(cx);
            let buffer_ref = buffer.read(cx);
            let adapters = buffer_ref
                .language_at(buffer_position)
                .map(|language| project_ref.languages().lsp_adapters(&language.name()))
                .unwrap_or_default();
            let relevant_server_names = adapters
                .iter()
                .map(|adapter| adapter.name())
                .collect::<HashSet<_>>();
            let buffer_worktree = buffer_ref.file().map(|file| file.worktree_id(cx));
            let opened_in_servers =
                lsp_store.language_server_ids_for_opened_buffer(buffer_ref.remote_id());
            let mut candidates = project_ref
                .language_server_statuses(cx)
                .filter(|(server_id, status)| {
                    relevant_server_names.contains(&status.name)
                        && match (opened_in_servers, buffer_worktree, status.worktree) {
                            (Some(server_ids), _, _) => server_ids.contains(server_id),
                            (None, Some(buffer_worktree), Some(server_worktree)) => {
                                buffer_worktree == server_worktree
                            }
                            (None, _, _) => true,
                        }
                })
                .filter(|(server_id, status)| {
                    action
                        .server
                        .as_ref()
                        .is_none_or(|server| status.name.0.as_ref() == server)
                        && lsp_store
                            .lsp_server_capabilities
                            .get(server_id)
                            .and_then(|capabilities| capabilities.execute_command_provider.as_ref())
                            .is_some_and(|provider| provider.commands.contains(&action.command))
                })
                .map(|(server_id, status)| (server_id, status.name.clone()))
                .collect::<Vec<_>>();
            candidates.sort_by(|(_, left), (_, right)| left.0.cmp(&right.0));

            let server_id = match candidates.as_slice() {
                [(server_id, _)] => *server_id,
                [] => {
                    return Err(match &action.server {
                        Some(server) => anyhow!(
                            "language server {server:?} does not advertise command {:?} for the current buffer",
                            action.command
                        ),
                        None => anyhow!(
                            "no language server advertises command {:?} for the current buffer",
                            action.command
                        ),
                    });
                }
                candidates => {
                    let server_names = candidates
                        .iter()
                        .map(|(_, name)| name.0.as_ref())
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Err(anyhow!(
                        "multiple language servers advertise command {:?} for the current buffer: {server_names}; specify the server field",
                        action.command
                    ));
                }
            };

            Ok(project.read(cx).lsp_store().update(cx, |lsp_store, cx| {
                lsp_store.execute_lsp_command_for_buffer(
                    server_id,
                    buffer,
                    position,
                    action.command.clone(),
                    action.arguments.clone(),
                    cx,
                )
            }))
        })();

        match result {
            Ok(task) => task,
            Err(error) => Task::ready(Err(error)),
        }
    }
}

pub(crate) fn find_specific_language_server_in_selection<F>(
    editor: &Editor,
    cx: &mut App,
    filter_language: F,
    language_server_name: LanguageServerName,
) -> Option<(
    text::Anchor,
    Arc<Language>,
    LanguageServerId,
    Entity<Buffer>,
)>
where
    F: Fn(&Language) -> bool,
{
    let project = editor.project.clone()?;
    let multi_buffer = editor.buffer();
    let mut seen_buffer_ids = HashSet::default();
    editor
        .selections
        .disjoint_anchors_arc()
        .iter()
        .find_map(|selection| {
            let multi_buffer = multi_buffer.read(cx);
            let multi_buffer_snapshot = multi_buffer.snapshot(cx);
            let (position, buffer) = multi_buffer_snapshot
                .anchor_to_buffer_anchor(selection.head())
                .and_then(|(anchor, _)| Some((anchor, multi_buffer.buffer(anchor.buffer_id)?)))?;
            if !seen_buffer_ids.insert(buffer.read(cx).remote_id()) {
                return None;
            }

            let language = buffer.read(cx).language_at(position)?;
            if filter_language(&language) {
                let server_id = buffer.update(cx, |buffer, cx| {
                    project
                        .read(cx)
                        .language_server_id_for_name(buffer, &language_server_name, cx)
                })?;
                Some((position, language, server_id, buffer))
            } else {
                None
            }
        })
}

async fn lsp_task_context(
    project: &Entity<Project>,
    buffer: &Entity<Buffer>,
    cx: &mut AsyncApp,
) -> Option<TaskContext> {
    let (worktree_store, environment) = project.read_with(cx, |project, _| {
        (project.worktree_store(), project.environment().clone())
    });

    let worktree_abs_path = cx.update(|cx| {
        let worktree_id = buffer.read(cx).file().map(|f| f.worktree_id(cx));

        worktree_id
            .and_then(|worktree_id| worktree_store.read(cx).worktree_for_id(worktree_id, cx))
            .and_then(|worktree| worktree.read(cx).root_dir())
    });

    let project_env = environment
        .update(cx, |environment, cx| {
            environment.buffer_environment(buffer, &worktree_store, cx)
        })
        .await;

    Some(TaskContext {
        cwd: worktree_abs_path.map(|p| p.to_path_buf()),
        project_env: project_env.unwrap_or_default(),
        ..TaskContext::default()
    })
}

pub fn lsp_tasks(
    project: Entity<Project>,
    task_sources: &HashMap<LanguageServerName, Vec<BufferId>>,
    for_position: Option<text::Anchor>,
    cx: &mut App,
) -> Task<Vec<(TaskSourceKind, Vec<(Option<LocationLink>, ResolvedTask)>)>> {
    let lsp_task_sources = task_sources
        .iter()
        .filter_map(|(name, buffer_ids)| {
            let buffers = buffer_ids
                .iter()
                .filter(|&&buffer_id| match for_position {
                    Some(for_position) => for_position.buffer_id == buffer_id,
                    None => true,
                })
                .filter_map(|&buffer_id| project.read(cx).buffer_for_id(buffer_id, cx))
                .collect::<Vec<_>>();

            let server_id = buffers.iter().find_map(|buffer| {
                project.read_with(cx, |project, cx| {
                    project.language_server_id_for_name(buffer.read(cx), name, cx)
                })
            });
            server_id.zip(Some(buffers))
        })
        .collect::<Vec<_>>();

    cx.spawn(async move |cx| {
        cx.spawn(async move |cx| {
            let mut lsp_tasks = HashMap::default();
            for (server_id, buffers) in lsp_task_sources {
                let mut new_lsp_tasks = Vec::new();
                for buffer in buffers {
                    let source_kind = match buffer.update(cx, |buffer, _| {
                        buffer.language().map(|language| language.name())
                    }) {
                        Some(language_name) => TaskSourceKind::Lsp {
                            server: server_id,
                            language_name: SharedString::from(language_name),
                        },
                        None => continue,
                    };
                    let id_base = source_kind.to_id_base();
                    let lsp_buffer_context = lsp_task_context(&project, &buffer, cx)
                        .await
                        .unwrap_or_default();

                    let runnables_task = project.update(cx, |project, cx| {
                        let buffer_id = buffer.read(cx).remote_id();
                        project.request_lsp(
                            buffer,
                            LanguageServerToQuery::Other(server_id),
                            GetLspRunnables {
                                buffer_id,
                                position: for_position,
                                server_id,
                            },
                            cx,
                        )
                    });
                    if let Some(new_runnables) = runnables_task.await.log_err() {
                        new_lsp_tasks.extend(new_runnables.runnables.into_iter().filter_map(
                            |(location, runnable)| {
                                let resolved_task =
                                    runnable.resolve_task(&id_base, &lsp_buffer_context)?;
                                Some((location, resolved_task))
                            },
                        ));
                    }
                    if !new_lsp_tasks.is_empty() {
                        lsp_tasks
                            .entry(source_kind)
                            .or_insert_with(Vec::new)
                            .append(&mut new_lsp_tasks);
                    }
                }
            }
            lsp_tasks.into_iter().collect()
        })
        .with_timeout(Duration::from_millis(200), &cx.background_executor())
        .unwrap_or_else(|_| {
            log::debug!("Timed out waiting for LSP tasks");
            Vec::new()
        })
        .await
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures::StreamExt as _;
    use gpui::{AppContext as _, Entity, TestAppContext};
    use language::{FakeLspAdapter, Language};
    use languages::rust_lang;
    use lsp::{LanguageServerId, LanguageServerName};
    use multi_buffer::MultiBuffer;
    use project::{FakeFs, Project};
    use serde_json::json;
    use util::path;

    use crate::{
        ExecuteLspCommand, MoveToEnd, editor_tests::init_test, test::build_editor_with_project,
    };

    use super::find_specific_language_server_in_selection;

    #[gpui::test]
    async fn test_find_language_server_at_end_of_file(cx: &mut TestAppContext) {
        init_test(cx, |_| {});

        let fs = FakeFs::new(cx.executor());
        fs.insert_file(path!("/file.rs"), "fn main() {}".into())
            .await;

        let project = Project::test(fs, [path!("/file.rs").as_ref()], cx).await;
        let language_registry = project.read_with(cx, |project, _| project.languages().clone());
        language_registry.add(rust_lang());
        let mut fake_servers =
            language_registry.register_fake_lsp("Rust", FakeLspAdapter::default());

        let underlying_buffer = project
            .update(cx, |project, cx| {
                project.open_local_buffer(path!("/file.rs"), cx)
            })
            .await
            .unwrap();

        let buffer = cx.new(|cx| MultiBuffer::singleton(underlying_buffer.clone(), cx));
        let (editor, cx) = cx.add_window_view(|window, cx| {
            build_editor_with_project(project.clone(), buffer, window, cx)
        });

        let fake_server = fake_servers.next().await.unwrap();
        cx.executor().run_until_parked();

        let expected_server_id = fake_server.server.server_id();
        let language_server_name = LanguageServerName::new_static("the-fake-language-server");
        let filter = |language: &Language| language.name().as_ref() == "Rust";

        let assert_result = |result: Option<(
            text::Anchor,
            Arc<Language>,
            LanguageServerId,
            Entity<language::Buffer>,
        )>,
                             message: &str| {
            let (_, language, server_id, buffer) = result.expect(message);
            assert_eq!(
                language.name().as_ref(),
                "Rust",
                "{message}: wrong language"
            );
            assert_eq!(server_id, expected_server_id, "{message}: wrong server ID");
            assert_eq!(buffer, underlying_buffer, "{message}: wrong buffer");
        };

        editor.update(cx, |editor, cx| {
            assert_result(
                find_specific_language_server_in_selection(
                    editor,
                    cx,
                    filter,
                    language_server_name.clone(),
                ),
                "should find correct language server at beginning of file",
            );
        });

        editor.update_in(cx, |editor, window, cx| {
            editor.move_to_end(&MoveToEnd, window, cx);
        });

        editor.update(cx, |editor, cx| {
            assert_result(
                find_specific_language_server_in_selection(
                    editor,
                    cx,
                    filter,
                    language_server_name.clone(),
                ),
                "should find correct language server at end of file",
            );
        });
    }

    #[gpui::test]
    async fn test_execute_lsp_command_expands_buffer_placeholders(cx: &mut TestAppContext) {
        init_test(cx, |_| {});

        let fs = FakeFs::new(cx.executor());
        fs.insert_file(path!("/file.rs"), "a🙂".into()).await;

        let project = Project::test(fs, [path!("/file.rs").as_ref()], cx).await;
        let language_registry = project.read_with(cx, |project, _| project.languages().clone());
        language_registry.add(rust_lang());
        let mut fake_servers = language_registry.register_fake_lsp(
            "Rust",
            FakeLspAdapter {
                capabilities: lsp::ServerCapabilities {
                    execute_command_provider: Some(lsp::ExecuteCommandOptions {
                        commands: vec!["test-command".to_string()],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        let underlying_buffer = project
            .update(cx, |project, cx| {
                project.open_local_buffer(path!("/file.rs"), cx)
            })
            .await
            .expect("opening test buffer should succeed");
        let buffer = cx.new(|cx| MultiBuffer::singleton(underlying_buffer, cx));
        let (editor, cx) = cx.add_window_view(|window, cx| {
            build_editor_with_project(project.clone(), buffer, window, cx)
        });

        let fake_server = fake_servers
            .next()
            .await
            .expect("fake language server should start");
        cx.executor().run_until_parked();
        fake_server.set_request_handler::<lsp::request::ExecuteCommand, _, _>(
            |params, _| async move {
                assert_eq!(params.command, "test-command");
                assert_eq!(
                    params.arguments,
                    vec![
                        json!("file:///file.rs"),
                        json!(0),
                        json!(3),
                        json!("literal-$line"),
                        json!({"nested": 3}),
                    ]
                );
                Ok(Some(json!({"executed": true})))
            },
        );

        editor.update_in(cx, |editor, window, cx| {
            editor.move_to_end(&MoveToEnd, window, cx);
        });
        for server in [None, Some("the-fake-language-server".to_string())] {
            let result = editor
                .update(cx, |editor, cx| {
                    editor.execute_lsp_command(
                        &ExecuteLspCommand {
                            command: "test-command".to_string(),
                            arguments: vec![
                                json!("$uri"),
                                json!("$line"),
                                json!("$column"),
                                json!("literal-$line"),
                                json!({"nested": "$column"}),
                            ],
                            server,
                        },
                        cx,
                    )
                })
                .await
                .expect("executing command should succeed");
            assert_eq!(result, Some(json!({"executed": true})));
        }

        let error = editor
            .update(cx, |editor, cx| {
                editor.execute_lsp_command(
                    &ExecuteLspCommand {
                        command: "unadvertised-command".to_string(),
                        arguments: Vec::new(),
                        server: None,
                    },
                    cx,
                )
            })
            .await
            .expect_err("an unadvertised command should fail");
        assert!(
            error
                .to_string()
                .contains("no language server advertises command")
        );
    }

    #[gpui::test]
    async fn test_execute_lsp_command_requires_unambiguous_server(cx: &mut TestAppContext) {
        init_test(cx, |_| {});

        let fs = FakeFs::new(cx.executor());
        fs.insert_file(path!("/file.rs"), "fn main() {}".into())
            .await;

        let project = Project::test(fs, [path!("/file.rs").as_ref()], cx).await;
        let language_registry = project.read_with(cx, |project, _| project.languages().clone());
        language_registry.add(rust_lang());
        let capabilities = lsp::ServerCapabilities {
            execute_command_provider: Some(lsp::ExecuteCommandOptions {
                commands: vec!["shared-command".to_string()],
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut first_server = language_registry.register_fake_lsp(
            "Rust",
            FakeLspAdapter {
                name: "server-a",
                capabilities: capabilities.clone(),
                ..Default::default()
            },
        );
        let mut second_server = language_registry.register_fake_lsp(
            "Rust",
            FakeLspAdapter {
                name: "server-b",
                capabilities,
                ..Default::default()
            },
        );

        let underlying_buffer = project
            .update(cx, |project, cx| {
                project.open_local_buffer(path!("/file.rs"), cx)
            })
            .await
            .expect("opening test buffer should succeed");
        let buffer = cx.new(|cx| MultiBuffer::singleton(underlying_buffer, cx));
        let (editor, cx) =
            cx.add_window_view(|window, cx| build_editor_with_project(project, buffer, window, cx));

        let first_server = first_server
            .next()
            .await
            .expect("first fake language server should start");
        let second_server = second_server
            .next()
            .await
            .expect("second fake language server should start");
        cx.executor().run_until_parked();
        first_server.set_request_handler::<lsp::request::ExecuteCommand, _, _>(|_, _| async move {
            Ok(Some(json!("server-a")))
        });
        second_server.set_request_handler::<lsp::request::ExecuteCommand, _, _>(
            |_, _| async move { Ok(Some(json!("server-b"))) },
        );

        let result = editor
            .update(cx, |editor, cx| {
                editor.execute_lsp_command(
                    &ExecuteLspCommand {
                        command: "shared-command".to_string(),
                        arguments: Vec::new(),
                        server: Some("server-b".to_string()),
                    },
                    cx,
                )
            })
            .await
            .expect("an explicit language server should resolve ambiguity");
        assert_eq!(result, Some(json!("server-b")));

        let error = editor
            .update(cx, |editor, cx| {
                editor.execute_lsp_command(
                    &ExecuteLspCommand {
                        command: "shared-command".to_string(),
                        arguments: Vec::new(),
                        server: None,
                    },
                    cx,
                )
            })
            .await
            .expect_err("ambiguous language servers should fail");
        assert!(
            error
                .to_string()
                .contains("multiple language servers advertise command")
        );
        assert!(error.to_string().contains("server-a, server-b"));
    }
}
