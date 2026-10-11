"""Guard CLI reference coverage and its shared output-format interaction.

Run with Python's jsonschema package and Node.js available.
"""
import html
import json
import re
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent
SCHEMAS = ROOT.parent / "schemas"


def output_pages():
    return sorted(ROOT.glob("cli-*.html")) + [ROOT / "glossary.html"]


def closed_values(definition):
    values = set(definition.get("enum", []))
    if "const" in definition:
        values.add(definition["const"])
    for branch in definition.get("oneOf", []):
        values.update(closed_values(branch))
    return values

# The site's platform.py is a generator, not the Python standard module.
# Keep it out of dependency imports when this file is run directly.
_import_path = sys.path[:]
sys.path = [entry for entry in sys.path if Path(entry).resolve() != ROOT]
try:
    import jsonschema
finally:
    sys.path = _import_path


def examples(page):
    source = page.read_text()
    fixture = re.search(
        r'<script type="application/json" id="output-examples">(.*?)</script>',
        source, re.S,
    )
    outputs = []
    commands = []
    for attrs, code in re.findall(r'<pre([^>]*)><code>(.*?)</code></pre>', source, re.S):
        attrs = dict(re.findall(r'([\w-]+)="([^"]*)"', attrs)) | {
            key: "" for key in re.findall(r'\b(data-output-command)\b', attrs)
        }
        code = html.unescape(code)
        if "data-output-command" in attrs:
            commands.append(code)
        if "data-output-example" in attrs:
            outputs.append({"id": attrs["data-output-example"],
                            "schema": attrs.get("data-output-schema"), "text": code})
    return source, json.loads(fixture[1]) if fixture else {}, commands, outputs


class CliReferenceTests(unittest.TestCase):
    def test_every_operation_has_structure_and_paired_outputs(self):
        for page in sorted(ROOT.glob("cli-*.html")):
            with self.subTest(page=page.name):
                source, fixtures, commands, outputs = examples(page)
                self.assertEqual(source.count("data-output-format"), 1)
                self.assertRegex(source, r'<p class="lede">')
                self.assertIn('class="command-variant"', source)
                if not re.search(r'<h2[^>]*id="response-classes"[^>]*>Успешные результаты</h2>', source):
                    self.assertIn("Варианты ответа", source)
                self.assertNotIn('<th>Команда</th>', source)
                self.assertTrue(commands)
                self.assertTrue(outputs)
                self.assertEqual({output["id"] for output in outputs}, set(fixtures))
                self.assertEqual(len(outputs), len(fixtures), "duplicate output IDs")
                for output in outputs:
                    self.assertTrue(output["schema"], output["id"])
                    self.assertTrue(output["text"].strip(), output["id"])

    def test_grouped_response_layout_preserves_calls_and_card_links(self):
        # Opt in through authored structure, not a second registry of pilot pages.
        for page in ROOT.glob("cli-*.html"):
            source, fixtures, _, outputs = examples(page)
            success = re.search(r'<h2[^>]*id="response-classes"[^>]*>Успешные результаты</h2>', source)
            if not success:
                continue
            with self.subTest(page=page.name):
                main = re.search(r'<main[^>]*>(.*?)</main>', source, re.S)[1]
                headings = list(re.finditer(r'<h2([^>]*)>(.*?)</h2>', main, re.S))
                variants = [h for h in headings if 'class="command-variant"' in h[1]]
                self.assertTrue(variants)
                self.assertEqual(headings[:len(variants)], variants,
                                 "calls must precede prose and responses as one group")
                execution = next((h for h in headings if h[2] == "Исполнение"), None)
                successes = next((h for h in headings if h[2] == "Успешные результаты"), None)
                errors = next((h for h in headings if h[2] == "Ошибки и отказы"), None)
                self.assertIsNotNone(execution)
                self.assertIsNotNone(successes)
                self.assertIsNotNone(errors)
                self.assertLess(variants[-1].start(), execution.start())
                self.assertLess(execution.start(), successes.start())
                self.assertLess(successes.start(), errors.start())
                call_ids = {re.search(r'\bid="([^"]+)"', h[1])[1] for h in variants}
                cards = list(re.finditer(r'<h4([^>]*)>(.*?)</h4>(.*?)(?=<h[234]\b|</main>)', source, re.S))
                self.assertTrue(cards)
                covered = set()
                linked_calls = set()
                for card in cards:
                    attrs, _, body = card.groups()
                    self.assertRegex(attrs, r'\bid="[^"]+"')
                    self.assertRegex(attrs, r'\bdata-response-class="[^"]+"')
                    self.assertRegex(body, r'<p\b', "card must explain its condition")
                    links = set(re.findall(r'href="#([^"]+)"', body)) & call_ids
                    self.assertTrue(links, "card must link to applicable calls")
                    linked_calls.update(links)
                    ids = re.findall(r'data-output-example="([^"]+)"', body)
                    self.assertTrue(ids, "card must include a CLI/JSON pair")
                    covered.update(ids)
                    expected_ok = card.start() < source.index(errors[0])
                    for ident in ids:
                        self.assertEqual(fixtures[ident]["ok"], expected_ok,
                                         "response must belong to its success/error group")
                self.assertEqual(covered, {output["id"] for output in outputs})
                self.assertEqual(linked_calls, call_ids,
                                 "each call must be linked from a response card")

    def test_json_examples_obey_envelope_and_existing_result_schemas(self):
        envelope = json.loads((SCHEMAS / "command-envelope.schema.json").read_text())
        index = json.loads((SCHEMAS / "command-data/index.json").read_text())["forms"]
        for page in output_pages():
            _, fixtures, _, outputs = examples(page)
            for output in outputs:
                with self.subTest(page=page.name, example=output["id"]):
                    result = fixtures[output["id"]]
                    jsonschema.validate(result, envelope)
                    if output["schema"] != "planned":
                        slug = output["schema"]
                        self.assertIn(slug, index[result["command"]] + ["refusal"])
                        schema = json.loads((SCHEMAS / "command-data" / (slug + ".schema.json")).read_text())
                        jsonschema.validate(result["data"], schema)
                    if not result["ok"]:
                        self.assertIn("error", result)

    def test_shared_switch_round_trips_every_page_without_rewriting_results(self):
        # Execute the production interaction against page-owned fixtures. The
        # assertions cover the public DOM interface, including failed envelopes.
        script = (ROOT / "site.js").read_text()
        start = script.index("(function () {\n  var control = document.querySelector('[data-output-format]');")
        end = script.index("\n})();", start) + len("\n})();")
        pages = []
        for page in output_pages():
            _, fixtures, commands, outputs = examples(page)
            pages.append({"name": page.name, "fixtures": fixtures,
                          "commands": commands, "outputs": outputs})
        js = r"""
const vm = require('vm'), assert = require('assert');
const input = JSON.parse(require('fs').readFileSync(0, 'utf8'));
for (const page of input.pages) {
  for (const initial of ['cli', 'json']) {
    let selected = initial, listener;
    const commands = page.commands.map(textContent => ({textContent}));
    // An already-qualified invocation must not gain a second global key.
    commands.push({textContent:'v8-runner --json-message version'});
    const outputs = page.outputs.map(output => ({textContent:output.text,
      parentElement:{dataset:{outputExample:output.id}}}));
    const originals = [...commands, ...outputs].map(node => node.textContent);
    const control = {querySelector(){return {value:selected}},
      addEventListener(type, callback){assert.equal(type,'change');listener=callback}};
    const document = {querySelector(){return control},
      getElementById(){return {textContent:JSON.stringify(page.fixtures)}},
      querySelectorAll(selector){return selector.includes('data-output-command') ? commands : outputs}};
    vm.runInNewContext(input.script,{document});
    for (const format of ['json','cli','json','json','cli']) {
      selected=format;listener();
      commands.forEach((node,i) => {
        const count = (node.textContent.match(/--json-message/g) || []).length;
        assert.equal(count,format==='json' || originals[i].includes('--json-message') ? 1 : 0,page.name);
        if(format==='cli') assert.equal(node.textContent,originals[i],page.name);
      });
      outputs.forEach((node,i) => {
        if(format==='json') assert.deepStrictEqual(JSON.parse(node.textContent),page.fixtures[page.outputs[i].id],page.name);
        else assert.equal(node.textContent,page.outputs[i].text,page.name);
      });
    }
  }
}
"""
        subprocess.run(["node", "-e", js],
                       input=json.dumps({"script": script[start:end], "pages": pages}),
                       text=True, check=True, capture_output=True)

    def test_response_inventory_covers_target_classes_and_error_codes(self):
        for page in ROOT.glob("cli-*.html"):
            source, _, _, outputs = examples(page)
            with self.subTest(page=page.name):
                self.assertEqual(source.count('id="response-classes"'), 1)
                self.assertIn('class="response-classes"', source)
                self.assertIn('glossary.html#command-responses', source)
                self.assertIn('data-response-class=', source)
                if (not re.search(r'<h2[^>]*id="response-classes"[^>]*>Успешные результаты</h2>', source)
                        and outputs and all(output["schema"] == "planned" for output in outputs)):
                    inventory = re.search(r'<table class="response-classes">(.*?)</table>', source, re.S)[1]
                    for anchor in re.findall(r'<h4 id="([^"]+)"', source):
                        self.assertIn('href="#' + anchor + '"', inventory)
        schema = json.loads((SCHEMAS / "command-envelope.schema.json").read_text())
        # The shared schema also accepts an implementation availability marker.
        # It is not an outcome of the target command and has no public row.
        target_codes = closed_values(schema["$defs"]["ErrorCode"]) - {"soon"}
        source, fixtures, _, _ = examples(ROOT / "glossary.html")
        rows = re.findall(r'data-error-code="([^"]+)" data-error-kind="([^"]+)"', source)
        self.assertEqual({code for code, _ in rows}, target_codes)
        self.assertEqual(len(rows), len(target_codes))
        kinds = dict(rows)
        demonstrated = {v["error"]["code"] for v in fixtures.values()}
        self.assertTrue(demonstrated)
        self.assertLessEqual(demonstrated, target_codes)
        for page in output_pages():
            source, fixtures, _, _ = examples(page)
            with self.subTest(page=page.name):
                self.assertNotIn("data-error-state", source)
            for key, fixture in fixtures.items():
                if not fixture["ok"]:
                    with self.subTest(page=page.name, example=key):
                        error = fixture["error"]
                        self.assertIn(error["code"], kinds)
                        self.assertEqual(error["kind"], kinds[error["code"]])

    def test_failure_examples_preserve_result_semantics(self):
        # These are target result invariants, independent of which adapter
        # currently emits an example or whether a command is implemented.
        def result(slug, key):
            return examples(ROOT / ("cli-" + slug + ".html"))[1][key]
        for slug, key, status in [
            ("make", "make-failed", "invalid_output"),
            ("upload", "upload-failed", "failed"),
            ("test", "test-no-report", "invalid_output"),
            ("test", "test-assertions-failed", "failed"),
        ]:
            failure = result(slug, key)
            self.assertFalse(failure["ok"])
            data = failure["data"]
            self.assertEqual(data["execution"]["status"], status)
            error = data["execution"]["errors"][0]
            self.assertTrue(error["code"])
            self.assertTrue(error["message"])
            if slug == "test":
                self.assertEqual(data["error_kind"], error["code"])
            else:
                self.assertEqual(data["message"], error["message"])
        # An outer command failure category and the nested result status describe
        # different things; switching the output format must preserve both.
        no_report = result("test", "test-no-report")
        self.assertEqual(no_report["error"]["kind"], "runtime")
        self.assertEqual(no_report["data"]["execution"]["status"], "invalid_output")
        failed = result("test", "test-assertions-failed")["data"]
        self.assertEqual(failed["execution"]["metrics"], failed["report"]["summary"])
        self.assertEqual(failed["execution"]["payload"], failed["report"])
        retained = result("test", "test-no-report")["data"]
        artifacts = {item["role"]: item["path"] for item in retained["execution"]["artifacts"]["items"]}
        for key, role in [("run_dir", "run_dir"), ("config_json", "config"),
                          ("junit_xml", "report"), ("yaxunit_log", "runner_log"),
                          ("platform_log", "platform_log"), ("sentinel", "sentinel")]:
            self.assertEqual(retained["retained_paths"][key], artifacts[role])
        for slug, key in [("download", "download-uncertain"),
                          ("infobase-dump", "infobase-dump-uncertain"),
                          ("infobase-restore", "restore-uncertain")]:
            failure = result(slug, key)
            self.assertEqual(failure["data"]["target_state"], "uncertain")
            self.assertTrue(failure["warnings"])
            self.assertTrue(any(step["status"] == "failed" for step in failure["steps"]))
            self.assertFalse(failure["ok"])
            self.assertTrue(failure["data"]["execution"]["errors"])
        _, create, _, _ = examples(ROOT / "cli-infobase-create.html")
        statuses = {step["status"] for v in create.values() for step in v["data"].get("steps", [])}
        expected = json.loads((SCHEMAS / "command-data/infobase-create.schema.json").read_text())["$defs"]["InitStepStatus"]
        self.assertEqual(statuses, closed_values(expected))

    def test_example_subjects_and_preview_dispatch_agree(self):
        _, launch, _, _ = examples(ROOT / "cli-launch.html")
        web = launch["launch-variant-2"]["data"]
        self.assertEqual(web["mode"], "web")
        self.assertEqual(web["via"], "web")
        self.assertEqual(web["url"], "https://web.example/demo")
        self.assertNotIn("platform_resolution", web)
        self.assertNotEqual(web["binary"], "/opt/1cv8/bin/1cv8")
        self.assertIn("message", web)
        source, push, _, outputs = examples(ROOT / "cli-push.html")
        deleted = push["push-variant-5"]["data"]["extensions"]
        self.assertEqual(deleted, [{"name": "my-ext", "action": "delete", "state": "removed"}])
        deleted_text = next(output["text"] for output in outputs if output["id"] == "push-variant-5")
        self.assertIn("my-ext", deleted_text)
        self.assertNotIn("main:", deleted_text)
        self.assertIn("v8-runner push --delete my-ext", source)
        for page in ROOT.glob("cli-*.html"):
            _, fixtures, _, _ = examples(page)
            for key, fixture in fixtures.items():
                if key.endswith("example") and "provider_dispatched" in fixture["data"]:
                    with self.subTest(page=page.name, example=key):
                        self.assertFalse(fixture["data"]["provider_dispatched"])


if __name__ == "__main__":
    unittest.main()
