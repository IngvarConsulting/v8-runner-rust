#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CI_SCOPE="${V8_RUNNER_CI_SCOPE:-contract}"
TARGET_OS_LABEL="${V8TR_CI_TARGET_OS:-$(uname -s)}"

cd "$ROOT_DIR"

case "$CI_SCOPE" in
  contract)
    case "$TARGET_OS_LABEL" in
      Windows|MINGW*|MSYS*|CYGWIN*)
        echo "Windows contract scope runs native infobase export, external full-pull, init and help CLI tests, plus selected unit regressions."
        # Сборочная проверка сюда не входит: на Windows шаг Lint в ci.yml прогоняет
        # `cargo clippy --locked --bins`, а тестовые цели собирает сам `cargo test` ниже.
        cargo test --locked --test cli_infobase_cross_platform
        cargo test --locked --test cli_external_full_pull
        cargo test --locked --test cli_config_init --test cli_help
        cargo test --locked --bin v8-runner 'support::fs::tests::'
        cargo test --locked --bin v8-runner 'config::loader::tests::declared_infobases_'
        cargo test --locked --bin v8-runner 'use_cases::infobase_owner::tests::'
        windows_contract_tests=(
          "a_closed_job_keeps_the_agent_one_shot"
          "a_ctrl_break_from_another_process_group_cancels_like_ctrl_c"
          "config::loader::tests::load_config_normalizes_relative_platform_path_against_config_directory"
          "domain::source_set::tests::accepts_absolute_path"
          "platform::locator::tests::a_missing_component_is_named_together_with_the_installations_that_have_it"
          "support::temp::tests::creates_temp_files_in_new_locations"
          "platform::designer::tests::cf_exports_pass_windows_drive_paths_without_verbatim_prefix"
          "platform::designer::tests::cf_exports_pass_windows_unc_paths_without_verbatim_prefix"
          "platform::designer::tests::cf_exports_preserve_regular_windows_targets"
          "platform::designer::tests::dt_export_passes_windows_drive_path_without_verbatim_prefix"
          "platform::designer::tests::dt_export_passes_windows_unc_path_without_verbatim_prefix"
          "platform::designer::tests::dt_export_preserves_regular_windows_target"
          "platform::ibcmd::tests::config_save_passes_windows_paths_without_verbatim_prefix"
          "platform::process::tests::detached_child_does_not_hold_redirected_stdout_open"
          "platform::process::tests::managed_detached_child_does_not_hold_redirected_stdout_open"
          "platform::process::tests::spawn_managed_cleans_windows_job_when_startup_probe_detects_early_exit"
          "platform::process::tests::windows_process_group::a_ctrl_break_to_the_runner_group_misses_its_managed_child"
          "platform::process::tests::windows_client_owner::host_job_terminates_client_before_startup_handoff"
          "platform::process::tests::windows_client_owner::released_host_job_preserves_client_after_runner_exit"
          "support::path::tests::filesystem_object_identity_changes_when_directory_is_replaced"
          "support::path::tests::resolve_from_builds_native_windows_paths"
          "support::path::tests::planned_path_rejects_regular_file_traversal_even_before_parent_components"
          "support::path::tests::planned_path_matches_native_windows_target_after_junction_and_parent"
          "support::path::tests::a_response_on_windows_names_paths_without_the_verbatim_prefix"
          "mcp::server::tests::an_mcp_answer_names_paths_without_the_verbatim_prefix"
          "support::fs::tests::replace_file_restores_original_bytes_when_stage_disappeared"
          "use_cases::staged_publication::tests::orphan_cleanup_requires_exact_target_kind_and_run_name_contract"
          "use_cases::build_project::tests::partial_load_list_releases_writer_before_reader_and_cleans_up_after_use"
          "use_cases::dump_config::tests::a_partial_dump_hands_the_designer_a_closed_object_list"
          "use_cases::dump_config::tests::dump_full_success_replaces_old_target"
          "use_cases::dump_config::tests::dump_full_preserves_old_dump_on_platform_failure"
          "use_cases::dump_config::tests::ibcmd_dump_full_uses_staging_dir_and_atomic_publish"
          "use_cases::dump_config::tests::ibcmd_dump_full_preserves_old_target_on_platform_failure"
          "use_cases::dump_config::tests::an_ibcmd_full_dump_lands_over_a_non_empty_directory_through_a_stage"
          "use_cases::dump_config::tests::a_failed_ibcmd_stage_dump_leaves_the_directory_untouched"
          "use_cases::dump_config::tests::a_broken_overlay_keeps_the_runner_copy_as_it_was"
          "use_cases::dump_config::tests::an_ibcmd_refusal_to_export_by_changes_falls_back_to_a_full_export"
          "use_cases::dump_config::tests::an_ibcmd_full_forecast_dumps_through_the_stage_without_sync"
          "use_cases::dump_config::tests::an_ibcmd_partial_dump_follows_a_full_forecast_too"
          "use_cases::dump_config::tests::ibcmd_dump_incremental_goes_by_the_runner_copy_through_a_stage"
          "platform::interactive::tests::closed_streams_on_windows_answer_with_the_exit_code_seen_after_them"
        )
        listed_tests="$(cargo test --locked -- --list)"
        for test_name in "${windows_contract_tests[@]}"; do
          if ! grep -Fxq "$test_name: test" <<<"$listed_tests"; then
            echo "Windows contract regression is missing: $test_name" >&2
            exit 2
          fi
          cargo test --locked "$test_name" -- --exact --nocapture
        done
        ;;
      Linux|Darwin)
        cargo test --locked
        ;;
      *)
        echo "Unsupported V8TR_CI_TARGET_OS: $TARGET_OS_LABEL" >&2
        echo "Expected one of: Linux, Windows, Darwin" >&2
        exit 2
        ;;
    esac
    ;;
  full)
    cargo test --locked
    ;;
  runtime-locks)
    cargo test --locked workspace_lock
    cargo test --locked advisory_lock
    cargo test --locked execute_command_reports_workspace_lock_conflict
    cargo test --locked default_port_reports_workspace_lock_conflict_before_use_case_dispatch
    ;;
  happy-path)
    bash "$ROOT_DIR/scripts/test/ci-happy-path.sh"
    ;;
  *)
    echo "Unsupported V8_RUNNER_CI_SCOPE: $CI_SCOPE" >&2
    echo "Expected one of: contract, full, runtime-locks, happy-path" >&2
    exit 2
    ;;
esac
