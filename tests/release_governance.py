#!/usr/bin/env python3
"""Guard the fork-owned release and provenance contract.

Root cause: the inherited workflow could publish untested archives without the
license or fork notice and resolved third-party actions through mutable tags.
Single owner: ``.github/workflows/release.yml`` plus the package metadata named
below. This test is the reintroduction guard for equivalent release paths.
"""

from __future__ import annotations

import importlib.util
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def load_release_verifier():
    path = ROOT / "scripts/release/verify-release-contract.py"
    spec = importlib.util.spec_from_file_location("verify_release_contract", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ReleaseGovernanceTest(unittest.TestCase):
    def test_release_is_verified_and_self_describing(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        self.assertIn("preflight:", workflow)
        self.assertIn("needs: preflight", workflow)
        self.assertIn("scripts/release/verify-release-contract.py", workflow)
        self.assertIn('cp LICENSE "${package_dir}/"', workflow)
        self.assertIn('cp FORK_NOTICE.md "${package_dir}/"', workflow)

    def test_release_has_one_protected_entrypoint_and_freezes_only_after_audit(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        self.assertNotIn("push:\n    tags:", workflow)
        self.assertIn("workflow_dispatch:", workflow)
        self.assertIn("group: release-${{ inputs.tag }}", workflow)
        self.assertIn("environment: release", workflow)
        self.assertIn("github.ref == 'refs/heads/master'", workflow)
        self.assertIn("name: Audit draft release", workflow)
        self.assertIn("needs: [publish, audit-native]", workflow)
        self.assertIn("gh release edit", workflow)
        self.assertIn("--draft=false", workflow)
        self.assertIn('existing_state="$(gh release view', workflow)
        self.assertIn('if [[ "${existing_state}" == "false" ]]', workflow)
        self.assertIn('if [[ "${existing_state}" == "true" ]]', workflow)
        self.assertIn('gh release delete "${RELEASE_TAG}"', workflow)

    def test_release_verification_waits_for_github_attestation_with_a_deadline(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        freeze = workflow.split("  freeze:\n", 1)[1]
        self.assertIn("RELEASE_VERIFY_TIMEOUT_SECONDS", freeze)
        self.assertIn('until gh release verify "${RELEASE_TAG}"', freeze)
        self.assertIn("SECONDS >= verify_deadline", freeze)
        self.assertIn("sleep 5", freeze)

    def test_release_publishes_one_attested_archive_per_platform(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        for asset in (
            "v8-runner-linux-x86_64-musl",
            "v8-runner-macos-aarch64",
            "v8-runner-macos-x86_64",
            "v8-runner-windows-x86_64",
        ):
            self.assertIn(asset, workflow)
        self.assertIn("actions/attest-build-provenance@e8998f949152b193b063cb0ec769d69d929409be", workflow)
        self.assertIn("id-token: write", workflow)
        self.assertIn("attestations: write", workflow)
        self.assertIn("v8-runner-assets.json", workflow)
        self.assertIn("test -f dist/v8-runner-assets.json", workflow)
        self.assertIn("license-v8-runner-AGPL-3.0-only.txt", workflow)
        self.assertIn("notice-v8-runner-fork.txt", workflow)
        self.assertIn("gh attestation verify", workflow)
        self.assertIn("--deny-self-hosted-runners", workflow)

    @staticmethod
    def _release_jobs() -> dict[str, str]:
        """Работы рабочего процесса и их тела, без сторонних библиотек.

        Питон здесь живёт на стандартной библиотеке: `pip install` в CI нет ни
        одного, и разбор YAML пришлось бы туда завозить ради одной проверки.
        Структура читается по отступам — так же, как реестр читает своё
        front matter.
        """
        text = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        body = text.split("\njobs:\n", 1)[1]
        jobs: dict[str, list[str]] = {}
        current: str | None = None
        for line in body.splitlines():
            if not line.strip() or line.lstrip().startswith("#"):
                continue
            header = re.fullmatch(r"  ([A-Za-z0-9_-]+):", line)
            if header:
                current = header.group(1)
                jobs[current] = []
                continue
            if not line.startswith("  "):
                break
            if current is not None:
                jobs[current].append(line)
        return {name: "\n".join(lines) for name, lines in jobs.items()}

    def test_only_the_publish_job_publishes(self) -> None:
        """Шаги публикации живут в своей работе и никуда не съезжают.

        Разрешимость `needs` по всем файлам держит
        `test_every_workflow_job_graph_resolves`; здесь — то, чего она не видит.
        Однажды удаление шага унесло заголовок работы `publish`, и её пять шагов
        оказались внутри матричной сборки: у той нет прав на запись в релиз, а
        выполнялись бы они по разу на платформу, затирая друг другу `dist`.
        """
        jobs = self._release_jobs()
        self.assertEqual(
            set(jobs),
            {"preflight", "build", "publish", "audit-native", "audit-draft", "freeze"},
        )
        self.assertIn("      contents: write", jobs["publish"])
        self.assertIn("      contents: read", jobs["build"])
        for step in ("softprops/action-gh-release", "write-manifest", "download-artifact"):
            self.assertNotIn(step, jobs["build"], f"{step} drifted into the matrix job")
            self.assertIn(step, jobs["publish"], f"{step} left the publish job")

    def test_a_platform_is_published_in_one_form_only(self) -> None:
        """Одна платформа — один ассет.

        До v0.11.0 та же сборка выкладывалась дважды: архивом и голым бинарником
        под другим именем, и «что из этого что» приходилось объяснять словами.
        Примета держит то, что убрано: имена вернувшихся бинарников, отдельную
        роль в манифесте и таблицу, из которой их собирали.
        """
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        script = (ROOT / "scripts/release/release_assets.py").read_text(encoding="utf-8")
        body = workflow.split("body: |", 1)[1].split("files: |", 1)[0]

        for gone in ("v8-runner-darwin-arm64", "v8-runner-linux-x64", "v8-runner-win-x64.exe"):
            # В теле релиза они названы нарочно: чтобы искавший их узнал, что их нет.
            self.assertNotIn(gone, workflow.replace(body, ""), f"{gone} is published again")
            self.assertNotIn(gone, script, f"{gone} is built again")
        self.assertNotIn("DIRECT_ASSETS", script)
        self.assertNotIn("direct-binary", script)
        self.assertNotIn("unica_asset_name", workflow)

        # Каждая платформа названа в аудите ровно один раз.
        audit = workflow.split("  audit-native:\n", 1)[1].split("  audit-draft:\n", 1)[0]
        for target in (
            "x86_64-unknown-linux-musl",
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "x86_64-pc-windows-msvc",
        ):
            self.assertEqual(audit.count(f"target: {target}"), 1, f"{target} is audited once")

    def test_release_publishes_one_manifest_instead_of_per_asset_sidecars(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        self.assertNotIn("write-checksum", workflow)
        self.assertNotRegex(workflow, r"dist/[^\n]*(?:\.sha256|\.provenance\.json)")
        self.assertIn("v8-runner-assets.json", workflow)

    def test_portable_archive_documents_have_canonical_line_endings(self) -> None:
        attributes = (ROOT / ".gitattributes").read_text(encoding="utf-8")
        for name in ("README.md", "LICENSE", "FORK_NOTICE.md"):
            self.assertIn(f"{name} text eol=lf", attributes)

    def test_all_payload_assets_and_manifest_have_build_attestations(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        self.assertIn("Attest portable archive", workflow)
        self.assertIn("Attest consolidated release manifest", workflow)
        self.assertIn("for asset in $(python3 scripts/release/release_assets.py attested-assets)", workflow)

    def test_draft_auditors_can_read_unpublished_release(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        native = workflow.split("  audit-native:\n", 1)[1].split("  audit-draft:\n", 1)[0]
        draft = workflow.split("  audit-draft:\n", 1)[1].split("  freeze:\n", 1)[0]
        self.assertIn("contents: write", native)
        self.assertIn("contents: write", draft)

    def test_release_toolchain_and_source_identity_are_pinned(self) -> None:
        workflow = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
        verifier = (ROOT / "scripts/release/verify-release-contract.py").read_text(
            encoding="utf-8"
        )
        self.assertIn('toolchain: "1.95.0"', workflow)
        self.assertIn("MACOSX_DEPLOYMENT_TARGET", workflow)
        self.assertIn("refs/remotes/origin/master", verifier)
        self.assertIn("refs/tags/{args.tag}^{{commit}}", verifier)
        self.assertIn("GITHUB_SHA", verifier)
        self.assertIn("MIN_CONSOLIDATED_MANIFEST_VERSION", verifier)
        self.assertIn("consolidated release assets require", verifier)

    def test_ci_and_release_pin_the_same_toolchain(self) -> None:
        """CI обязана проверять тот компилятор, которым собирается выпуск.

        Пин живёт в трёх местах и разъехаться может молча: следующий подъём версии в
        release.yml оставил бы CI на прежней, а свойство «CI гоняет релизный компилятор»
        умерло бы незаметно. Кавычки не требуются: значение без них — та же версия и
        та же дыра.
        """
        pins = {}
        for name in ("ci.yml", "release.yml"):
            workflow = (ROOT / ".github/workflows" / name).read_text(encoding="utf-8")
            steps = re.findall(r"uses:\s*dtolnay/rust-toolchain@\S+", workflow)
            found = set(re.findall(r'toolchain:\s*"?([0-9]+\.[0-9]+(?:\.[0-9]+)?)"?', workflow))
            self.assertEqual(
                len(steps),
                len(re.findall(r"toolchain:\s*\S+", workflow)),
                f"{name}: every rust-toolchain step must pin a version explicitly",
            )
            self.assertEqual(
                1, len(found), f"{name} must pin exactly one toolchain version: {found}"
            )
            pins[name] = found.pop()

        self.assertEqual(
            pins["ci.yml"],
            pins["release.yml"],
            "ci.yml and release.yml must pin the same toolchain, "
            f"got {pins['ci.yml']} and {pins['release.yml']}",
        )

        # MSRV — обещание того же компилятора, а не отдельное число.
        cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
        msrv = re.search(r'^rust-version\s*=\s*"([^"]+)"', cargo, re.M)
        self.assertIsNotNone(msrv, "Cargo.toml must declare rust-version")
        self.assertTrue(
            pins["ci.yml"].startswith(msrv.group(1)),
            f"rust-version {msrv.group(1)} must match the pinned toolchain {pins['ci.yml']}",
        )

    def test_ci_enforces_formatting_and_lints(self) -> None:
        """Гейты живут шагами джобы Contract и блокируют по-настоящему.

        Список обязательных проверок ветки master привязан к именам джоб: вынеси их в
        новую джобу — и они перестанут блокировать, пока его не поправит администратор.
        Поэтому проверяется не наличие строк в файле, а то, что команды стоят шагами
        именно этой джобы, без continue-on-error и без сужения до одной площадки.
        """
        ci = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        job = self._workflow_job(ci, "contract")

        gates = {
            "cargo fmt --all --check": "ubuntu-latest",
            "cargo clippy --locked --all-targets -- -D warnings": None,
            "cargo deny check licenses sources": "ubuntu-latest",
        }
        for command, _ in gates.items():
            self.assertIn(
                command,
                job,
                f"{command!r} must be a step of the Contract job, not of a new one",
            )

        # Команда, закомментированная или обёрнутая в continue-on-error, перестаёт быть
        # гейтом, оставаясь подстрокой файла.
        for line in job.splitlines():
            stripped = line.strip()
            if stripped.startswith("#"):
                self.assertNotIn(
                    "cargo clippy",
                    stripped,
                    "the lint gate must not be commented out",
                )

        blocking = job.split("- name: Run contract regression scope")[0]
        soft = [
            step
            for step in blocking.split("      - name: ")[1:]
            if "continue-on-error" in step
        ]
        self.assertEqual(
            ["Report dependency advisories and duplicates"],
            [step.splitlines()[0].strip() for step in soft],
            "only the advisories report may be non-blocking",
        )

        # Линтер обязан идти и на Windows: код под cfg(windows) на Linux не собирается.
        # Цель там боевая, а не все: тестовая полна мёртвого кода из-за cfg(unix)-гейтов
        # на самих тестах, и это снимается отдельной работой.
        self.assertIn("cargo clippy --locked --bins -- -D warnings", job)
        windows_lint = job.split("- name: Lint (production target)")[1]
        self.assertIn("matrix.os == 'windows-latest'", windows_lint.split("run:")[0])

        assignments = re.findall(r"^\s*RUSTFLAGS\s*[:=]", ci, re.M)
        self.assertEqual(
            [],
            assignments,
            "-D warnings must be an argument: RUSTFLAGS would reach dependencies "
            "and invalidate the shared build cache",
        )

    @staticmethod
    def _workflow_job(workflow: str, name: str) -> str:
        """Тело одной джобы: от её ключа до следующего на том же отступе."""
        lines = workflow.splitlines()
        start = next(i for i, line in enumerate(lines) if line == f"  {name}:")
        for offset, line in enumerate(lines[start + 1 :], start=start + 1):
            if line.startswith("  ") and not line.startswith("   ") and line.strip():
                return "\n".join(lines[start:offset])
        return "\n".join(lines[start:])

    def test_consolidated_contract_accepts_v07_prereleases_only(self) -> None:
        verifier = load_release_verifier()

        verifier.require_consolidated_manifest_version("0.7.0-pre.1")
        verifier.require_consolidated_manifest_version("0.7.0-ic.1")
        verifier.require_consolidated_manifest_version("0.8.0+build.1")
        with self.assertRaisesRegex(SystemExit, "v0.7.0 or newer"):
            verifier.require_consolidated_manifest_version("0.6.99")
        with self.assertRaisesRegex(SystemExit, "semantic version"):
            verifier.require_consolidated_manifest_version("0.7.0-01")

    def test_documented_attestation_is_bound_to_verified_manifest_commit(self) -> None:
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("gh release verify-asset v0.7.0 ./v8-runner-assets.json", readme)
        self.assertIn('source_commit="$(python3', readme)
        self.assertIn(
            "for asset in v8-runner-assets.json v8-runner-linux-x86_64-musl.tar.gz", readme
        )
        self.assertIn('--source-digest "$source_commit"', readme)

    def test_pr_ci_runs_release_asset_contract_tests(self) -> None:
        ci = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
        self.assertIn("python3 tests/release_governance.py", ci)
        self.assertIn("python3 tests/release_assets.py", ci)

    def test_all_actions_are_pinned_to_full_commit_sha(self) -> None:
        for path in sorted((ROOT / ".github/workflows").glob("*.yml")):
            workflow = path.read_text(encoding="utf-8")
            floating = re.findall(r"^\s*uses:\s*[^\s@]+@(?![0-9a-f]{40}(?:\s|$))[^\s]+", workflow, re.M)
            self.assertEqual([], floating, f"floating action refs in {path}: {floating}")

    def test_every_workflow_job_graph_resolves(self) -> None:
        """Каждая работа, названная в `needs`, существует.

        GitHub не запускает рабочий процесс с висячей зависимостью — ни одного шага,
        то есть релиз или проверку просто нечем выпустить. Однажды удаление шага
        унесло с собой заголовок работы, и оба набора тестов остались зелёными:
        они сверяли подстроки, а строка `needs: [publish, audit-native]` осталась
        на месте — пропала работа.

        Это та же проверка, что делает actionlint выше по гейту, но без него: он
        внешний двоичный файл, а эта примета живёт в дереве и не зависит ни от
        сети, ни от сторонних модулей. PyYAML здесь нет намеренно — в CI нет ни
        одного `pip install`, поэтому структура читается по отступам.
        """
        for path in sorted((ROOT / ".github/workflows").glob("*.yml")):
            jobs = self._workflow_jobs(path)
            self.assertTrue(jobs, f"{path.name} declares no jobs")
            for name, block in jobs.items():
                self.assertIn("\n    steps:", f"\n{block}", f"{path.name}: {name} has no steps")
                declared = re.search(r"^    needs: (.+)$", block, re.M)
                if not declared:
                    continue
                value = declared.group(1).strip()
                needs = (
                    [item.strip() for item in value.strip("[]").split(",")]
                    if value.startswith("[")
                    else [value]
                )
                for dependency in needs:
                    self.assertIn(
                        dependency,
                        jobs,
                        f"{path.name}: {name} needs {dependency}, which is not a job there",
                    )

    @staticmethod
    def _workflow_jobs(path: Path) -> dict[str, str]:
        """Работы рабочего процесса и их тела, разбором отступов."""
        text = path.read_text(encoding="utf-8")
        if "\njobs:\n" not in text:
            return {}
        body = text.split("\njobs:\n", 1)[1]
        jobs: dict[str, list[str]] = {}
        current: str | None = None
        for line in body.splitlines():
            if not line.strip() or line.lstrip().startswith("#"):
                continue
            header = re.fullmatch(r"  ([A-Za-z0-9_-]+):", line)
            if header:
                current = header.group(1)
                jobs[current] = []
                continue
            if not line.startswith("  "):
                break
            if current is not None:
                jobs[current].append(line)
        return {name: "\n".join(lines) for name, lines in jobs.items()}

    def test_package_metadata_names_fork_and_license(self) -> None:
        cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
        self.assertIn('license = "AGPL-3.0-only"', cargo)
        self.assertIn('repository = "https://github.com/IngvarConsulting/v8-runner-rust"', cargo)
        self.assertTrue((ROOT / "FORK_NOTICE.md").is_file())

    def test_generated_schema_contract_no_longer_points_to_old_owner(self) -> None:
        tracked = [
            ROOT / "src/config/schema.rs",
            ROOT / "src/use_cases/config_init.rs",
            ROOT / "src/use_cases/tools_download.rs",
            ROOT / "docs/CONFIGURATION.md",
            ROOT / "docs/schemas/v8project.schema.json",
            ROOT / "docs/schemas/v8project.local.schema.json",
            ROOT / "tests/cli_bootstrap.rs",
            ROOT / "tests/cli_config_init.rs",
        ]
        for path in tracked:
            self.assertNotIn("alkoleft/v8-runner-rust", path.read_text(encoding="utf-8"), str(path))


if __name__ == "__main__":
    unittest.main()
