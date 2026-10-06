#!/usr/bin/env python3
"""Контрольные случаи сверки `scripts/site_matrix.py` (#225) на искусственных данных."""

from __future__ import annotations

import importlib.util
import json
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location("site_matrix", ROOT / "scripts" / "site_matrix.py")
site_matrix = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(site_matrix)


def matrix_of(row: list[tuple[str, bool]]):
    """Матрица, где у каждой пары одна и та же строка."""
    operations = {op for ops in site_matrix.SCENARIO_OPERATIONS.values() for op in ops}
    return {(op, target): row for op in operations for target in site_matrix.TARGETS}


def site_of(chain: list[str]):
    scenarios = set(site_matrix.SCENARIO_OPERATIONS) | site_matrix.SCENARIOS_WITHOUT_ROW
    return {s: {target: list(chain) for target in site_matrix.TARGETS} for s in scenarios}


PROVIDERS = {"designer", "agent", "ibcmd", "ibcmd-rs", "webinst"}


class DefaultChainCases(unittest.TestCase):
    def problems(self, site_chain, row, known=None):
        return site_matrix.compare(matrix_of(row), PROVIDERS, site_of(site_chain), {}, known or {})

    def test_an_experimental_agent_moved_first_still_diverges(self) -> None:
        # Агент переставлен первым, но остался экспериментальным: в цепочку умолчаний он
        # не входит, и сайт с `agent` первым по-прежнему с ней расходится.
        problems = self.problems(["agent", "designer"], [("agent", False), ("designer", True)])
        self.assertTrue(problems)
        self.assertIn("матрица agent~ designer", problems[0])

    def test_an_experimental_tail_is_not_part_of_the_default_chain(self) -> None:
        self.assertTrue(self.problems(["ibcmd", "agent"], [("ibcmd", True), ("agent", False)]))
        self.assertEqual([], self.problems(["ibcmd"], [("ibcmd", True), ("agent", False)]))

    def test_an_implemented_chain_in_the_same_order_matches(self) -> None:
        self.assertEqual(
            [], self.problems(["agent", "designer"], [("agent", True), ("designer", True)])
        )

    def test_a_known_line_that_no_longer_diverges_is_reported(self) -> None:
        known = {("push", "file"): ("designer", "designer", "#206")}
        problems = self.problems(["designer"], [("designer", True)], known)
        self.assertTrue(any("уберите строку #206" in p for p in problems))


class TableCases(unittest.TestCase):
    def table(self, body: str) -> str:
        return f'<h2 id="d-ops">x</h2><table><tbody>{body}</tbody></table>'

    def test_a_row_of_header_cells_is_a_clear_failure(self) -> None:
        with self.assertRaisesRegex(site_matrix.Failure, "без клеток <td>"):
            site_matrix.table_chains(self.table("<tr><th>a</th><th>b</th></tr>"), PROVIDERS)

    def test_a_row_with_attributes_is_read(self) -> None:
        with self.assertRaisesRegex(site_matrix.Failure, "«нечто» таблицы #d-ops не сопоставлена"):
            site_matrix.table_chains(
                self.table('<tr class="x"><td class="k">нечто</td><td></td></tr>'), PROVIDERS
            )


class ArtifactCases(unittest.TestCase):
    def test_the_artifact_names_every_target_of_every_operation(self) -> None:
        text = (ROOT / "docs" / "schemas" / "capability-matrix.json").read_text(encoding="utf-8")
        matrix, providers = site_matrix.load_matrix(text)
        self.assertIn("designer", providers)
        operations = {op for op, _ in matrix}
        for operation in operations:
            for target in site_matrix.TARGETS:
                self.assertIn((operation, target), matrix)

    def test_a_malformed_artifact_names_the_regeneration_command(self) -> None:
        with self.assertRaisesRegex(site_matrix.Failure, "UPDATE_CAPABILITY_MATRIX=1"):
            site_matrix.load_matrix(json.dumps({"operations": {}}))


if __name__ == "__main__":
    unittest.main()
